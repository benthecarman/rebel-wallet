use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::{Arc, RwLock};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::Context;
use bark::actions::lightning::receive::{LightningReceiveState, Progress as ReceiveProgress};
use bark::ark::lightning::{PaymentHash, Preimage};
use bark::ark::vtxo::Full;
use bark::ark::{Vtxo, VtxoPolicy};
use bark::lightning_invoice::Bolt11Invoice;
use bark::movement::{Movement, PaymentMethod as BarkPaymentMethod};
use bark::persist::models::RoundStateId;
use bark::round::RoundStatus;
use bark::Wallet;
use bip39::Mnemonic;
use bitcoin::{
    bip32::{DerivationPath, Xpriv},
    Amount,
};
use flume::Sender;
use nostr_sdk::prelude::{
    nip04, Contact as NostrContact, ContactListBuilder, EventBuilder, EventBuilderTemplate, Filter,
    FinalizeEvent, Keys, Kind, PublicKey as NostrPublicKey, Tag, ToBech32,
};
use tokio::runtime::Runtime;
use zeroize::Zeroizing;

use crate::activity::{
    activity_from_movement, apply_activity_metadata, coalesce_activity_items,
    visible_activity_movements,
};
use crate::nostr_support::{
    apply_metadata_content, contact_id, deleted_profile_content, mark_profile_deleted,
    merge_contacts, metadata_from_state, nostr_client, nostr_contact_display_name,
    primal_follow_contacts, primal_search_profiles, profile_contact_from_metadata_json,
    profile_contact_from_metadata_json_with_petname, public_key_from_npub_or_hex,
    upload_profile_picture,
};
use crate::nwc::{NwcAppContext, NwcController, NwcPushRegistrationUpdate};
use crate::payments::{monitor_ark_receive, monitor_lightning_receive};
use crate::persistence::{PaymentAnnotation, ZapReceiptRecord};
use crate::price::fetch_bitcoin_price;
use crate::profile_cache::{
    clear_profile_cache, clear_profile_picture_dir, ensure_profile_picture_dir,
    new_profile_picture_download_semaphore, open_profile_cache,
    save_own_profile_picture_remote_url, update_cached_picture,
};
use crate::time::{now_label, now_unix};
use crate::updates::{AppUpdate, AsyncMsg, CoreMsg, HapticFeedback, WalletSnapshot};
use crate::wallet::WalletOpenMode;
use crate::{
    AppAction, AppState, BusyState, CapabilityRequest, CapabilityRequestKind, Contact,
    LightningAddressRegistrationPhase, MainTab, NostrMessage, PriceCurrency, ReceiveMethod,
    ReceivePhase, Screen, SecretStore, SendPhase, SetupState,
};

mod custom_address_flow;
mod profile_prefetch;
mod send_flow;
mod wallet_diagnostics;
mod wallet_lifecycle;
mod wallet_work;

use wallet_work::{
    refresh_poll_delay, WalletWorkCoordinator, WalletWorkKind, WalletWorkRequest, WalletWorkToken,
    FOREGROUND_MAINTENANCE_INTERVAL, WALLET_WORK_TIMEOUT,
};

pub(crate) const WALLET_SEED_KEY: &str = "wallet_seed";
pub(crate) const NOSTR_SECRET_KEY: &str = "nostr_secret";
const NOSTR_DERIVATION_PATH: &str = "m/44'/1237'/0'/0/0";

fn profile_picture_download_key(pubkey: &str, remote_url: &str) -> String {
    format!("{pubkey}:{remote_url}")
}

fn send_screen_removed(previous: &[Screen], next: &[Screen]) -> bool {
    previous.iter().any(|screen| matches!(screen, Screen::Send))
        && !next.iter().any(|screen| matches!(screen, Screen::Send))
}

pub(crate) fn derive_nostr_keys_from_mnemonic(mnemonic: &str) -> anyhow::Result<Keys> {
    let mnemonic = Mnemonic::from_str(mnemonic).context("invalid recovery phrase")?;
    let seed = mnemonic.to_seed("");
    let root = Xpriv::new_master(bitcoin::Network::Bitcoin, &seed)
        .context("could not create master key")?;
    let path =
        DerivationPath::from_str(NOSTR_DERIVATION_PATH).context("invalid Nostr derivation path")?;
    let secp = bitcoin::secp256k1::Secp256k1::new();
    let child = root
        .derive_priv(&secp, &path)
        .context("could not derive Nostr key")?;
    let secret_hex = Zeroizing::new(child.private_key.display_secret().to_string());
    Keys::parse(secret_hex.as_str()).context("derived invalid Nostr key")
}

fn committed_round_balance(
    pending_rounds: impl IntoIterator<Item = (RoundStateId, u64)>,
    statuses: &HashMap<RoundStateId, RoundStatus>,
) -> Option<u64> {
    let mut committed_sat = 0u64;
    for (id, amount_sat) in pending_rounds {
        let status = statuses.get(&id)?;
        if matches!(status, RoundStatus::Unconfirmed { .. }) {
            committed_sat = committed_sat.saturating_add(amount_sat);
        }
    }
    Some(committed_sat)
}

async fn committed_pending_round_balance(wallet: &Wallet) -> anyhow::Result<Option<u64>> {
    let pending = wallet.pending_round_states().await?;
    if pending.is_empty() {
        return Ok(Some(0));
    }
    let pending = pending
        .iter()
        .map(|round| (round.id(), round.state().pending_balance().to_sat()))
        .collect::<Vec<_>>();
    let statuses = wallet.sync_pending_rounds().await?;
    Ok(committed_round_balance(pending, &statuses))
}

async fn wallet_synced_msg(
    wallet: &Wallet,
    contacts: &[Contact],
    lightning_address: &crate::LightningAddressState,
    payment_annotations: &[PaymentAnnotation],
    zap_receipts: &[ZapReceiptRecord],
) -> anyhow::Result<WalletSnapshot> {
    // A delegated participation remains cancelable-by-spend while Bark reports
    // Pending. Once it is Unconfirmed, the server has assigned it to a funding
    // transaction and the input VTXOs have reached the point of no return.
    let pending_refresh_sat = committed_pending_round_balance(wallet).await?;
    let balance = wallet.balance().await.context("balance failed")?;
    // Bark's aggregate claimable balance includes both HTLCs that can still be
    // claimed off-chain and HTLCs whose preimage was already revealed. Split
    // those states so the latter aren't presented as claimable in the UI.
    let receive_balances = async {
        let pending = wallet.pending_lightning_receives().await?;
        let mut claimable = 0u64;
        let mut stuck = 0u64;
        for receive in pending {
            let (htlc_ids, amount) = match &receive.progress {
                ReceiveProgress::HtlcsReady(htlcs) => (&htlcs.vtxo_ids, &mut claimable),
                ReceiveProgress::PreimageRevealed(htlcs) => (&htlcs.vtxo_ids, &mut stuck),
                ReceiveProgress::AwaitingPayment | ReceiveProgress::Delivering(_) => continue,
            };
            let mut receive_amount = 0u64;
            for id in htlc_ids {
                receive_amount += wallet.get_vtxo_by_id(*id).await?.vtxo.amount().to_sat();
            }
            *amount += receive_amount;
        }
        Ok::<_, anyhow::Error>((claimable, stuck))
    }
    .await;
    let (pending_receive_sat, stuck_receive_sat) = match receive_balances {
        Ok(balances) => balances,
        Err(_) => (balance.claimable_lightning_receive.to_sat(), 0),
    };
    let history = wallet.history().await.context("history failed")?;
    let mut activity = Vec::new();
    for movement in visible_activity_movements(history) {
        let lightning_details = movement_lightning_details_from_vtxos(wallet, &movement).await;
        let mut item = activity_from_movement(
            movement,
            contacts,
            lightning_address.address.as_deref(),
            lightning_address.backing_ark_address.as_deref(),
        );
        if item.lightning_payment_hash.is_none() {
            item.lightning_payment_hash = lightning_details.payment_hash;
        }
        if item.lightning_payment_preimage.is_none() {
            item.lightning_payment_preimage = lightning_details.payment_preimage;
        }
        activity.push(item);
    }
    let mut activity = coalesce_activity_items(activity);
    apply_activity_metadata(&mut activity, contacts, payment_annotations, zap_receipts);
    Ok(WalletSnapshot {
        balance_sat: balance.spendable.to_sat(),
        pending_receive_sat,
        stuck_receive_sat,
        pending_send_sat: balance.pending_lightning_send.to_sat(),
        pending_refresh_sat,
        has_pending_rounds: balance.pending_in_round.to_sat() > 0,
        activity,
    })
}

#[derive(Default)]
struct MovementLightningDetails {
    payment_hash: Option<String>,
    payment_preimage: Option<String>,
}

async fn movement_lightning_details_from_vtxos(
    wallet: &Wallet,
    movement: &Movement,
) -> MovementLightningDetails {
    let mut details = MovementLightningDetails::default();
    let ids = movement
        .output_vtxos
        .iter()
        .chain(movement.input_vtxos.iter())
        .copied()
        .collect::<Vec<_>>();

    for id in ids {
        let Ok(vtxo) = wallet.get_full_vtxo(id).await else {
            continue;
        };
        let vtxo_details = lightning_details_from_vtxo(&vtxo);
        if details.payment_hash.is_none() {
            details.payment_hash = vtxo_details.payment_hash;
        }
        if details.payment_preimage.is_none() {
            details.payment_preimage = vtxo_details.payment_preimage;
        }
        if details.payment_hash.is_some() && details.payment_preimage.is_some() {
            break;
        }
    }

    details
}

fn lightning_details_from_vtxo(vtxo: &Vtxo<Full>) -> MovementLightningDetails {
    let mut details = MovementLightningDetails::default();

    match vtxo.policy() {
        VtxoPolicy::ServerHtlcSend(policy) => {
            details.payment_hash = Some(policy.payment_hash.to_string());
        }
        VtxoPolicy::ServerHtlcSend_v0(policy) => {
            details.payment_hash = Some(policy.payment_hash.to_string());
        }
        VtxoPolicy::ServerHtlcRecv(policy) => {
            details.payment_hash = Some(policy.payment_hash.to_string());
        }
        VtxoPolicy::ServerHtlcRecv_v0(policy) => {
            details.payment_hash = Some(policy.payment_hash.to_string());
        }
        VtxoPolicy::Pubkey(_) => {}
    }

    if let Some(preimage) = preimage_from_vtxo_witnesses(vtxo) {
        let computed_hash = preimage.compute_payment_hash().to_string();
        if details.payment_hash.as_deref() == Some(computed_hash.as_str()) {
            details.payment_hash = Some(computed_hash);
            details.payment_preimage = Some(preimage.to_string());
        }
    }

    details
}

fn preimage_from_vtxo_witnesses(vtxo: &Vtxo<Full>) -> Option<Preimage> {
    for tx in vtxo.transactions().map(|item| item.tx) {
        for input in tx.input {
            for element in input.witness.iter() {
                if element.len() == 32 {
                    if let Ok(preimage) = Preimage::from_slice(element) {
                        return Some(preimage);
                    }
                }
            }
        }
    }
    None
}

pub(crate) fn spawn_actor(
    data_dir: PathBuf,
    cache_dir: PathBuf,
    secrets: Arc<dyn SecretStore>,
    core_tx: Sender<CoreMsg>,
    core_rx: flume::Receiver<CoreMsg>,
    shared_state: Arc<RwLock<AppState>>,
    update_tx: Sender<AppUpdate>,
) {
    thread::spawn(move || {
        let rt = Runtime::new().expect("tokio runtime");
        let mut core = AppCore::new(data_dir, cache_dir, secrets, core_tx, rt);
        core.emit(&shared_state, &update_tx);

        while let Ok(msg) = core_rx.recv() {
            core.handle(msg);
            core.emit(&shared_state, &update_tx);
        }
    });
}

struct AppCore {
    state: AppState,
    data_dir: PathBuf,
    cache_dir: PathBuf,
    app_data_path: PathBuf,
    secrets: Arc<dyn SecretStore>,
    tx: Sender<CoreMsg>,
    rt: Runtime,
    wallet: Option<Wallet>,
    wallet_generation: u64,
    wallet_work: WalletWorkCoordinator,
    wallet_foregrounded: bool,
    last_maintenance_completed_at: Option<Instant>,
    wallet_retry_kind: Option<WalletWorkKind>,
    has_pending_rounds: bool,
    refresh_poll_nonce: u64,
    refresh_poll_scheduled: bool,
    refresh_poll_attempt: u8,
    profile_db: Option<rusqlite::Connection>,
    profile_picture_downloads: HashSet<String>,
    profile_picture_download_semaphore: Arc<tokio::sync::Semaphore>,
    profile_info_requests: HashSet<String>,
    payment_annotations: Vec<PaymentAnnotation>,
    zap_receipts: Vec<ZapReceiptRecord>,
    nwc: NwcController,
    rev: u64,
    next_capability_id: u64,
    send_fee_estimate_request_id: u64,
    pending_haptics: Vec<HapticFeedback>,
    pending_side_effects: Vec<AppUpdate>,
}

impl AppCore {
    fn new(
        data_dir: PathBuf,
        cache_dir: PathBuf,
        secrets: Arc<dyn SecretStore>,
        tx: Sender<CoreMsg>,
        rt: Runtime,
    ) -> Self {
        ensure_profile_picture_dir(&cache_dir);
        let profile_picture_download_semaphore = new_profile_picture_download_semaphore();
        let nwc = NwcController::new(
            data_dir.clone(),
            &cache_dir,
            secrets.clone(),
            tx.clone(),
            rt.handle().clone(),
            profile_picture_download_semaphore.clone(),
        );
        Self {
            state: AppState::initial(),
            app_data_path: data_dir.join("rebel-app-data.json"),
            profile_db: open_profile_cache(&cache_dir).ok(),
            data_dir,
            cache_dir,
            secrets,
            tx,
            rt,
            wallet: None,
            wallet_generation: 0,
            wallet_work: WalletWorkCoordinator::default(),
            wallet_foregrounded: false,
            last_maintenance_completed_at: None,
            wallet_retry_kind: None,
            has_pending_rounds: false,
            refresh_poll_nonce: 0,
            refresh_poll_scheduled: false,
            refresh_poll_attempt: 0,
            profile_picture_downloads: HashSet::new(),
            profile_picture_download_semaphore,
            profile_info_requests: HashSet::new(),
            payment_annotations: Vec::new(),
            zap_receipts: Vec::new(),
            nwc,
            rev: 0,
            next_capability_id: 0,
            send_fee_estimate_request_id: 0,
            pending_haptics: Vec::new(),
            pending_side_effects: Vec::new(),
        }
    }

    fn handle(&mut self, msg: CoreMsg) {
        match msg {
            CoreMsg::Action(action) => self.handle_action(action),
            CoreMsg::Async(msg) => self.handle_async(msg),
        }
        self.flush_nwc_output();
        self.rev += 1;
        self.state.rev = self.rev;
        self.state.refresh_derived();
    }

    fn with_nwc<R>(
        &mut self,
        operation: impl FnOnce(&mut NwcController, &mut NwcAppContext<'_>) -> R,
    ) -> R {
        let mut context = NwcAppContext {
            state: &mut self.state,
            wallet: self.wallet.clone(),
            wallet_generation: self.wallet_generation,
            rev: self.rev,
        };
        operation(&mut self.nwc, &mut context)
    }

    fn flush_nwc_output(&mut self) {
        let output = self.nwc.take_output();
        if output.save_app_data {
            self.save_app_data();
        }
        self.pending_haptics.extend(output.haptics);
        self.pending_side_effects.extend(output.side_effects);
    }

    fn handle_action(&mut self, action: AppAction) {
        self.state.refresh_derived();
        match action {
            AppAction::Bootstrap => self.bootstrap(),
            AppAction::CreateWallet => {
                self.state.busy.opening_wallet = true;
                let mnemonic =
                    Zeroizing::new(Mnemonic::generate(12).expect("valid mnemonic").to_string());
                self.open_wallet(mnemonic, WalletOpenMode::Create);
            }
            AppAction::RestoreWallet { mnemonic } => {
                self.state.busy.opening_wallet = true;
                self.open_wallet(
                    Zeroizing::new(mnemonic.trim().to_string()),
                    WalletOpenMode::Restore,
                );
            }
            AppAction::ReplaceWallet { mnemonic } => {
                self.wallet = None;
                self.state.busy.opening_wallet = true;
                self.state.activity.clear();
                self.state.wallet.balance_sat = 0;
                self.state.wallet.pending_receive_sat = 0;
                self.state.wallet.stuck_receive_sat = 0;
                self.state.wallet.pending_send_sat = 0;
                self.state.wallet.pending_refresh_sat = 0;
                self.has_pending_rounds = false;
                self.open_wallet(
                    Zeroizing::new(mnemonic.trim().to_string()),
                    WalletOpenMode::Replace,
                );
            }
            AppAction::DeleteWallet => self.delete_wallet(),
            AppAction::ShowSeed => {
                if let Some(seed) = self.secrets.get_secret(WALLET_SEED_KEY.to_string()) {
                    let _ = self
                        .tx
                        .send(CoreMsg::Async(AsyncMsg::Seed(Zeroizing::new(seed))));
                } else {
                    self.state.toast = Some("Recovery phrase not found.".to_string());
                    self.request_haptic(HapticFeedback::NotificationWarning);
                }
            }
            AppAction::SyncWallet => self.request_wallet_work(WalletWorkRequest::user_sync()),
            AppAction::ReloadWalletDiagnostics => self.load_wallet_diagnostics(),
            AppAction::ForceRefreshWalletVtxos => self.force_refresh_wallet_vtxos(),
            AppAction::MaintainVtxos => {
                self.request_maintenance(WalletWorkRequest::lifecycle(WalletWorkKind::Maintain))
            }
            AppAction::Foregrounded => self.foregrounded(),
            AppAction::Backgrounded => self.backgrounded(),
            AppAction::RefreshPrice => self.refresh_price(),
            AppAction::SetPriceCurrency { currency } => self.set_price_currency(currency),
            AppAction::SelectNetwork {
                network,
                server_address,
                esplora_address,
            } => self.select_network(network, server_address, esplora_address),
            AppAction::SelectTab { tab } => self.state.router.selected_tab = tab,
            AppAction::PushScreen { screen } => {
                if screen == Screen::WalletDiagnostics {
                    self.load_wallet_diagnostics();
                }
                if screen == Screen::Receive {
                    self.state.reset_receive_draft();
                }
                self.state.router.screen_stack.push(screen);
            }
            AppAction::PopScreen => {
                let previous = self.state.router.screen_stack.clone();
                self.state.router.screen_stack.pop();
                if send_screen_removed(&previous, &self.state.router.screen_stack) {
                    self.reset_send_draft();
                }
            }
            AppAction::UpdateScreenStack { stack } => {
                let should_reset_send =
                    send_screen_removed(&self.state.router.screen_stack, &stack);
                self.state.router.screen_stack = stack;
                if should_reset_send {
                    self.reset_send_draft();
                }
            }
            AppAction::SelectReceiveMethod { method } => self.state.receive.method = method,
            AppAction::SetReceiveAmount { amount_sat } => {
                self.state.receive.amount_sat = amount_sat;
                self.save_app_data();
            }
            AppAction::SetReceiveMemo { memo } => {
                self.state.receive.memo = memo;
                self.save_app_data();
            }
            AppAction::EditReceiveRequest => self.state.receive.phase = ReceivePhase::Editing,
            AppAction::BeginReceiveRequest => self.create_receive_request(),
            AppAction::ResumeReceiveMonitor => self.resume_receive_monitor(),
            AppAction::ClaimPendingLightningReceives => self.claim_pending_lightning_receives(),
            AppAction::CreateArkAddress => self.create_ark_address(),
            AppAction::CreateLightningInvoice => self.create_lightning_invoice(),
            AppAction::SetLightningAddressName { name } => {
                let name = name.trim().to_ascii_lowercase();
                self.clear_stale_lightning_address_registration_for_name(&name);
                self.state.lightning_address.custom_name = name;
                self.state.lightning_address.registration_error = None;
                self.save_app_data();
            }
            AppAction::RegisterLightningAddress => self.register_lightning_address(),
            AppAction::ConfirmLightningAddressRegistrationPayment => {
                self.confirm_lightning_address_registration_payment()
            }
            AppAction::CancelLightningAddressRegistrationPayment => {
                self.cancel_lightning_address_registration_payment()
            }
            AppAction::VerifyLightningAddressRegistration => {
                self.verify_lightning_address_registration()
            }
            AppAction::ClearLightningAddressRegistration => {
                self.clear_lightning_address_registration();
                self.save_app_data();
            }
            AppAction::SetSendSearchQuery { query } => {
                self.state.send.search_query = query;
                self.search_nostr_profiles();
            }
            AppAction::ContinueSendSearch => {
                let query = self.state.send.search_query.clone();
                self.clear_send_contact_selection();
                self.set_send_destination(query);
            }
            AppAction::SelectSendContact { contact_id } => self.select_send_contact(contact_id),
            AppAction::PrefetchProfilePictures { contact_ids } => {
                self.prefetch_profile_pictures(contact_ids)
            }
            AppAction::SetSendDestination { destination } => {
                self.clear_send_contact_selection();
                self.set_send_destination(destination);
            }
            AppAction::SetSendAmount { amount_sat } => {
                if self.state.send.amount_locked {
                    return;
                }
                self.state.send.amount_sat = amount_sat;
                self.request_send_fee_estimate();
            }
            AppAction::SetSendMemo { memo } => self.state.send.memo = memo,
            AppAction::SetSendZapEnabled { enabled } => {
                self.state.send.zap_enabled = enabled && self.state.send.zap_available;
            }
            AppAction::PayDestination => self.pay_destination(),
            AppAction::PayLightningInvoice {
                invoice,
                amount_sat,
            } => self.pay_lightning_invoice(invoice, amount_sat),
            AppAction::PayArkAddress {
                address,
                amount_sat,
            } => self.pay_ark_address(address, amount_sat),
            AppAction::DismissPaymentSuccess => {
                if self.state.receive.phase == ReceivePhase::Success {
                    self.state.receive.phase = ReceivePhase::Editing;
                    self.state.receive.lightning_paid = false;
                }
                if self.state.send.phase == SendPhase::Success {
                    self.state.send.phase = SendPhase::Editing;
                }
            }
            AppAction::ResetSendDraft => self.reset_send_draft(),
            AppAction::RequestQrScan => self.request_capability(CapabilityRequestKind::QrScan),
            AppAction::RequestClipboardRead => {
                self.request_capability(CapabilityRequestKind::ClipboardRead)
            }
            AppAction::RequestPhotoPick => {
                self.request_capability(CapabilityRequestKind::PhotoPick)
            }
            AppAction::CompleteQrScan { value } => {
                self.state.capability_request = None;
                if let Some(value) = value.filter(|v| !v.trim().is_empty()) {
                    self.request_haptic(HapticFeedback::NotificationSuccess);
                    self.clear_send_contact_selection();
                    self.set_send_destination(value);
                    if self.state.router.screen_stack.last() != Some(&Screen::Send) {
                        self.state.router.screen_stack.push(Screen::Send);
                    }
                }
            }
            AppAction::CompleteClipboardRead { value } => {
                self.state.capability_request = None;
                if let Some(value) = value.filter(|v| !v.trim().is_empty()) {
                    self.request_haptic(HapticFeedback::NotificationSuccess);
                    self.clear_send_contact_selection();
                    self.set_send_destination(value);
                }
            }
            AppAction::CompletePhotoPick { image_base64 } => {
                self.state.capability_request = None;
                if let Some(image_base64) = image_base64 {
                    self.upload_nostr_profile_picture(image_base64);
                }
            }
            AppAction::CancelCapabilityRequest => self.state.capability_request = None,
            AppAction::SetPushNotificationRegistration {
                apns_device_token,
                registration_status,
                wake_server_url,
                app_id,
                environment,
                install_id,
            } => self.with_nwc(|nwc, context| {
                nwc.update_push_registration(
                    context,
                    NwcPushRegistrationUpdate {
                        apns_device_token,
                        registration_status,
                        wake_server_url,
                        app_id,
                        environment,
                        install_id,
                    },
                )
            }),
            AppAction::OpenNwaRequest { uri } => {
                self.with_nwc(|nwc, context| nwc.open_nwa_request(context, uri))
            }
            AppAction::ApproveNwaRequest {
                relay,
                budget_sat,
                budget_interval,
                permissions,
            } => {
                self.ensure_wallet_derived_nostr_key();
                self.with_nwc(|nwc, context| {
                    nwc.approve_nwa_request(
                        context,
                        relay,
                        budget_sat,
                        budget_interval,
                        permissions,
                    )
                })
            }
            AppAction::RetryNwaCallback => {
                self.with_nwc(|nwc, context| nwc.retry_nwa_callback(context))
            }
            AppAction::CancelNwaRequest => {
                self.with_nwc(|nwc, context| nwc.cancel_nwa_request(context))
            }
            AppAction::CompleteNwaCallbackOpen { opened } => {
                self.with_nwc(|nwc, context| nwc.complete_nwa_callback_open(context, opened))
            }
            AppAction::ProcessNwcWakeRequests { requests } => {
                self.with_nwc(|nwc, context| nwc.enqueue_wake_requests(context, requests))
            }
            AppAction::CreateNwcConnection {
                name,
                relay,
                budget_sat,
                budget_interval,
                permissions,
            } => {
                self.ensure_wallet_derived_nostr_key();
                self.with_nwc(|nwc, context| {
                    nwc.create_connection(
                        context,
                        name,
                        relay,
                        budget_sat,
                        budget_interval,
                        permissions,
                    )
                })
            }
            AppAction::RequestNwcConnectionExport {
                id,
                copy_to_clipboard,
            } => {
                self.with_nwc(|nwc, context| nwc.export_connection(context, id, copy_to_clipboard))
            }
            AppAction::DeleteNwcConnection { id } => {
                self.with_nwc(|nwc, context| nwc.delete_connection(context, id))
            }
            AppAction::GenerateNostrKey => self.generate_nostr_key(),
            AppAction::ImportNostrSecret { nsec_or_hex } => self.import_nostr_secret(nsec_or_hex),
            AppAction::ExportNostrSecret => self.export_nostr_secret(),
            AppAction::ClearNostrKey => self.clear_nostr_key(),
            AppAction::EditNostrProfile {
                name,
                about,
                picture,
                lud16,
                nip05,
            } => {
                if self.state.nostr.deleted {
                    self.state.toast = Some("Deleted profiles cannot be edited.".to_string());
                    return;
                }
                self.state.nostr.name = name;
                self.state.nostr.about = about;
                self.state.nostr.picture = picture.clone();
                self.state.nostr.picture_display_url = picture;
                self.state.nostr.lud16 = lud16;
                self.state.nostr.nip05 = nip05;
                self.state.nostr.deleted = false;
                if let Some(npub) = self.state.nostr.npub.clone() {
                    if let Ok(pubkey) = public_key_from_npub_or_hex(&npub) {
                        let pubkey_hex = pubkey.to_hex();
                        let picture = self.state.nostr.picture.clone();
                        save_own_profile_picture_remote_url(
                            self.profile_db.as_ref(),
                            &pubkey_hex,
                            &self.state.nostr,
                        );
                        self.prefetch_profile_picture_for_pubkey(&pubkey_hex, &picture);
                    }
                }
                self.state.toast = Some("Nostr profile saved locally.".to_string());
                self.save_app_data();
            }
            AppAction::UploadNostrProfilePicture { image_base64 } => {
                self.upload_nostr_profile_picture(image_base64)
            }
            AppAction::AddContact {
                npub,
                name,
                lightning_address,
                lnurl,
                picture,
            } => {
                let id = contact_id(&npub);
                if !self.state.nostr.contacts.iter().any(|c| c.id == id) {
                    let name = nostr_contact_display_name(None, Some(name), None, &npub);
                    self.state.nostr.contacts.push(Contact {
                        id,
                        npub,
                        name,
                        followed: true,
                        picture,
                        lightning_address,
                        lnurl,
                        last_used: now_unix(),
                    });
                    self.sort_contacts();
                    self.save_app_data();
                }
            }
            AppAction::EditContact {
                contact_id,
                name,
                npub,
                lightning_address,
                lnurl,
                picture,
            } => {
                if let Some(c) = self
                    .state
                    .nostr
                    .contacts
                    .iter_mut()
                    .find(|c| c.id == contact_id)
                {
                    c.name = name;
                    c.npub = npub;
                    c.lightning_address = lightning_address;
                    c.lnurl = lnurl;
                    c.picture = picture;
                    c.last_used = now_unix();
                    self.sort_contacts();
                    self.save_app_data();
                }
            }
            AppAction::FollowContact { contact_id } => {
                if let Some(c) = self
                    .state
                    .nostr
                    .contacts
                    .iter_mut()
                    .find(|c| c.id == contact_id)
                {
                    c.followed = true;
                    c.last_used = now_unix();
                    self.save_app_data();
                }
            }
            AppAction::UnfollowContact { contact_id } => {
                if let Some(c) = self
                    .state
                    .nostr
                    .contacts
                    .iter_mut()
                    .find(|c| c.id == contact_id)
                {
                    c.followed = false;
                    c.last_used = now_unix();
                    self.save_app_data();
                }
            }
            AppAction::DeleteContact { contact_id } => {
                self.state.nostr.contacts.retain(|c| c.id != contact_id);
                self.sort_contacts();
                self.save_app_data();
            }
            AppAction::PublishNostrProfile => self.publish_nostr_profile(),
            AppAction::RefreshNostrProfile => self.refresh_nostr_profile(),
            AppAction::DeleteNostrProfile => self.delete_nostr_profile(),
            AppAction::PublishContactList => self.publish_contact_list(),
            AppAction::RefreshContactList => self.refresh_contact_list(),
            AppAction::ClearNostrProfileCache => self.clear_nostr_profile_cache(),
            AppAction::LoadDirectMessages { contact_id } => self.load_direct_messages(contact_id),
            AppAction::SendDirectMessage {
                contact_id,
                message,
            } => self.send_direct_message(contact_id, message),
            AppAction::ClearToast => self.state.toast = None,
            AppAction::ClearRecoveryPhrase => self.state.recovery_phrase = None,
            AppAction::ClearRevealedNostrSecret => self.state.revealed_nostr_secret = None,
            AppAction::RequestHaptic { feedback } => self.request_haptic(feedback),
        }
        self.maybe_start_queued_wallet_work();
        self.reconcile_refresh_poll();
    }

    fn handle_async(&mut self, msg: AsyncMsg) {
        if self.is_stale_wallet_async(&msg) {
            return;
        }
        self.clear_busy_for_async(&msg);
        match msg {
            AsyncMsg::WalletDiagnosticsLoaded { report, .. } => {
                self.state.wallet_diagnostics = report;
                self.state.wallet_diagnostics_loading = false;
            }
            AsyncMsg::WalletReady {
                generation: _,
                wallet,
                mnemonic,
                recovery_notice,
            } => {
                if !self.save_wallet_seed(mnemonic.as_str()) {
                    let message = "Could not save the recovery phrase to the Keychain. \
                                   The wallet is not safely stored. Delete the wallet and \
                                   try again before receiving funds."
                        .to_string();
                    self.state.setup = SetupState::Error {
                        message: message.clone(),
                    };
                    self.state.toast = Some(message);
                    self.request_haptic(HapticFeedback::NotificationError);
                    return;
                }
                self.wallet = Some(wallet);
                self.state.setup = SetupState::Ready;
                self.state.router.default_screen = Screen::Home;
                self.state.router.selected_tab = MainTab::Home;
                self.state.router.screen_stack.clear();
                self.ensure_wallet_derived_nostr_key();
                self.ensure_lightning_address();
                self.with_nwc(|nwc, context| {
                    nwc.publish_pending_info_events(context);
                    nwc.process_pending_wake_requests(context);
                });
                self.request_wallet_work(WalletWorkRequest::lifecycle(WalletWorkKind::Load));
                self.request_maintenance(WalletWorkRequest::lifecycle(WalletWorkKind::Maintain));
                self.claim_pending_lightning_receives();
                if let Some(notice) = recovery_notice {
                    self.state.toast = Some(notice.message);
                    self.request_haptic(if notice.warning {
                        HapticFeedback::NotificationWarning
                    } else {
                        HapticFeedback::NotificationSuccess
                    });
                }
            }
            AsyncMsg::WalletOpenFailed {
                generation: _,
                message,
            } => {
                self.state.busy.bootstrapping = false;
                self.state.busy.opening_wallet = false;
                if matches!(self.state.setup, SetupState::NeedsSetup) {
                    self.state.setup = SetupState::Error {
                        message: message.clone(),
                    };
                }
                self.state.toast = Some(message);
                self.request_haptic(HapticFeedback::NotificationError);
            }
            AsyncMsg::WalletWorkFinished {
                generation,
                operation_id,
                result,
            } => self.finish_wallet_work(generation, operation_id, result),
            AsyncMsg::WalletRefreshPollDue { generation, nonce } => {
                self.handle_refresh_poll_due(generation, nonce)
            }
            AsyncMsg::ArkAddress(address) => {
                self.state.receive.ark_address = Some(address);
                if self.state.receive.receive_request.is_none() {
                    self.state.receive.phase = ReceivePhase::ShowingRequest;
                }
            }
            AsyncMsg::ReceiveRequest {
                uri,
                ark_address,
                lightning_invoice,
                payment_hash,
            } => {
                self.state.receive.method = ReceiveMethod::Lightning;
                self.state.receive.receive_request = Some(uri);
                self.state.receive.ark_address = Some(ark_address);
                self.state.receive.lightning_invoice = Some(lightning_invoice);
                self.state.receive.lightning_payment_hash = Some(payment_hash);
                self.state.receive.lightning_status = "waiting".to_string();
                self.state.receive.lightning_paid = false;
                self.state.receive.phase = ReceivePhase::ShowingRequest;
            }
            AsyncMsg::ArkReceiveConfirmed {
                address,
                amount_sat,
            } => {
                if self.state.receive.phase == ReceivePhase::ShowingRequest
                    && self.state.receive.ark_address.as_deref() == Some(address.as_str())
                {
                    self.state.receive.amount_sat = amount_sat;
                    self.state.receive.phase = ReceivePhase::Success;
                    self.request_haptic(HapticFeedback::NotificationSuccess);
                }
                self.maintain_vtxos();
            }
            AsyncMsg::LightningInvoice {
                invoice,
                payment_hash,
            } => {
                self.state.receive.lightning_invoice = Some(invoice);
                self.state.receive.lightning_payment_hash = Some(payment_hash);
                self.state.receive.lightning_status = "waiting".to_string();
                self.state.receive.lightning_paid = false;
                self.state.receive.phase = ReceivePhase::ShowingRequest;
            }
            AsyncMsg::LightningReceiveStatus {
                payment_hash,
                status,
                paid,
            } => {
                if self.state.receive.lightning_payment_hash.as_deref()
                    == Some(payment_hash.as_str())
                {
                    self.state.receive.lightning_status = status;
                    self.state.receive.lightning_paid = paid;
                }
            }
            AsyncMsg::LightningReceiveClaimed { payment_hash } => {
                if self.state.receive.lightning_payment_hash.as_deref()
                    == Some(payment_hash.as_str())
                {
                    self.state.receive.lightning_status = "paid".to_string();
                    self.state.receive.lightning_paid = true;
                    self.state.receive.phase = ReceivePhase::Success;
                    self.request_haptic(HapticFeedback::NotificationSuccess);
                }
                self.with_nwc(|nwc, context| nwc.reconcile_payment_notifications(context));
                self.maintain_vtxos();
            }
            AsyncMsg::LightningReceivesClaimed { payment_hashes } => {
                self.state.busy.claiming_lightning_receives = false;
                let matches_current = self
                    .state
                    .receive
                    .lightning_payment_hash
                    .as_deref()
                    .map(|hash| payment_hashes.iter().any(|h| h == hash))
                    .unwrap_or(false);
                if matches_current && !self.state.receive.lightning_paid {
                    self.state.receive.lightning_status = "paid".to_string();
                    self.state.receive.lightning_paid = true;
                    self.state.receive.phase = ReceivePhase::Success;
                    self.request_haptic(HapticFeedback::NotificationSuccess);
                }
                if !payment_hashes.is_empty() {
                    self.with_nwc(|nwc, context| nwc.reconcile_payment_notifications(context));
                    self.maintain_vtxos();
                }
            }
            AsyncMsg::LightningAddressReady(ark_address) => {
                self.state.lightning_address.backing_ark_address = Some(ark_address.clone());
                self.with_nwc(|nwc, context| nwc.refresh_connection_uris(context));
                self.save_lightning_address_ark_address(&ark_address);
                self.save_app_data();
            }
            AsyncMsg::LightningAddressRegistrationUpdated {
                name,
                lightning_address,
                payment_ark_address,
                invoice,
                purchase_id,
                amount_msats,
                active,
                paid,
                paid_from_wallet,
                requires_confirmation,
                annotation,
                warning,
            } => {
                if let Some(annotation) = annotation {
                    self.upsert_payment_annotation(annotation);
                    self.save_app_data();
                }
                self.apply_lightning_address_registration_update(
                    name,
                    lightning_address,
                    payment_ark_address,
                    invoice,
                    purchase_id,
                    amount_msats,
                    active,
                    paid,
                    paid_from_wallet,
                    requires_confirmation,
                    warning,
                );
            }
            AsyncMsg::SendFeeEstimateDue {
                request_id,
                destination,
                amount_sat,
                estimate_amount_sat,
                kind,
            } => {
                if self.send_fee_estimate_is_current(request_id, &destination, amount_sat) {
                    self.perform_send_fee_estimate(
                        request_id,
                        destination,
                        amount_sat,
                        estimate_amount_sat,
                        kind,
                    );
                }
            }
            AsyncMsg::SendFeeEstimated {
                request_id,
                destination,
                amount_sat,
                fee_sat,
                total_sat,
            } => {
                if self.send_fee_estimate_is_current(request_id, &destination, amount_sat) {
                    self.state.send.estimating_fee = false;
                    self.state.send.fee_estimate_sat = Some(fee_sat);
                    self.state.send.total_cost_sat = Some(total_sat);
                    self.state.send.fee_estimate_error = None;
                }
            }
            AsyncMsg::SendFeeEstimateFailed {
                request_id,
                destination,
                amount_sat,
                error,
            } => {
                if self.send_fee_estimate_is_current(request_id, &destination, amount_sat) {
                    self.state.send.estimating_fee = false;
                    self.state.send.fee_estimate_sat = None;
                    self.state.send.total_cost_sat = None;
                    self.state.send.fee_estimate_error = Some(error);
                }
            }
            AsyncMsg::Paid { result, annotation } => {
                if let Some(annotation) = annotation {
                    self.upsert_payment_annotation(annotation);
                    self.save_app_data();
                }
                self.state.send.phase = SendPhase::Success;
                self.state.send.success_amount_display = self.state.send.amount_display.clone();
                self.state.send.last_result = Some(result);
                self.request_haptic(HapticFeedback::NotificationSuccess);
                self.maintain_vtxos();
            }
            AsyncMsg::ZapReceiptsLoaded { receipts, records } => {
                let contacts = self.cache_fetched_profile_contacts(records);
                let contact_ids = contacts
                    .iter()
                    .map(|contact| contact.id.clone())
                    .collect::<Vec<_>>();
                merge_contacts(&mut self.state.nostr.contacts, contacts);
                self.sort_contacts();
                self.zap_receipts = receipts;
                self.save_app_data();
                self.prefetch_profile_pictures(contact_ids);
                self.refresh_activity_metadata();
            }
            AsyncMsg::Seed(seed) => {
                self.state.recovery_phrase = Some((*seed).clone());
            }
            AsyncMsg::NostrProfileLoaded { nostr, profile } => {
                self.state.nostr.name = nostr.name;
                self.state.nostr.about = nostr.about;
                self.state.nostr.picture = nostr.picture;
                self.state.nostr.picture_display_url = nostr.picture_display_url;
                self.state.nostr.lud16 = nostr.lud16;
                self.state.nostr.nip05 = nostr.nip05;
                self.state.nostr.deleted = nostr.deleted;
                if let Some(profile) = profile {
                    let pubkey_hex = profile.pubkey_hex.clone();
                    let contact = self.cache_fetched_profile_contact(profile);
                    if !self.state.nostr.deleted {
                        self.state.nostr.picture_display_url = contact.picture.clone();
                        self.prefetch_profile_picture_for_pubkey(&pubkey_hex, &contact.picture);
                    }
                }
                self.save_app_data();
            }
            AsyncMsg::NostrContactsLoaded(contacts) => {
                let contacts = self.cache_fetched_profile_contacts(contacts);
                let contact_ids = contacts
                    .iter()
                    .map(|contact| contact.id.clone())
                    .collect::<Vec<_>>();
                merge_contacts(&mut self.state.nostr.contacts, contacts);
                self.sort_contacts();
                self.state.toast = Some("Nostr contacts refreshed from Primal.".to_string());
                self.save_app_data();
                self.prefetch_profile_pictures(contact_ids);
                self.refresh_activity_metadata();
            }
            AsyncMsg::PrimalContactsLoaded {
                records,
                show_toast,
            } => {
                let contacts = self.cache_fetched_profile_contacts(records);
                let contact_ids = contacts
                    .iter()
                    .map(|contact| contact.id.clone())
                    .collect::<Vec<_>>();
                merge_contacts(&mut self.state.nostr.contacts, contacts);
                self.sort_contacts();
                if show_toast {
                    self.state.toast = Some("Nostr contacts refreshed from Primal.".to_string());
                }
                self.save_app_data();
                self.prefetch_profile_pictures(contact_ids);
                self.refresh_activity_metadata();
            }
            AsyncMsg::NostrSearchLoaded { query, contacts } => {
                if self.state.send.search_query.trim() == query {
                    self.state.send.global_search_results =
                        self.cache_fetched_profile_contacts(contacts);
                    let contact_ids = self
                        .state
                        .send
                        .global_search_results
                        .iter()
                        .map(|contact| contact.id.clone())
                        .collect::<Vec<_>>();
                    self.prefetch_profile_pictures(contact_ids);
                }
            }
            AsyncMsg::PrimalProfilesLoaded { records } => {
                for record in &records {
                    self.profile_info_requests.remove(&record.pubkey_hex);
                }
                let contacts = self.cache_fetched_profile_contacts(records);
                let contact_ids = contacts
                    .iter()
                    .map(|contact| contact.id.clone())
                    .collect::<Vec<_>>();
                merge_contacts(&mut self.state.nostr.contacts, contacts);
                self.sort_contacts();
                self.save_app_data();
                self.prefetch_profile_pictures(contact_ids);
            }
            AsyncMsg::PrimalProfilesFailed { pubkeys } => {
                for pubkey in pubkeys {
                    self.profile_info_requests.remove(&pubkey);
                }
            }
            AsyncMsg::ProfilePictureCached { pubkey, remote_url } => {
                self.profile_picture_downloads
                    .remove(&profile_picture_download_key(&pubkey, &remote_url));
                if let Some(conn) = self.profile_db.as_ref() {
                    let _ = update_cached_picture(conn, &pubkey, &remote_url);
                }
                self.refresh_contact_picture_for_pubkey(&pubkey);
                self.refresh_own_profile_picture_for_pubkey(&pubkey);
                self.refresh_activity_picture_for_pubkey(&pubkey);
            }
            AsyncMsg::ProfilePictureCacheFailed { pubkey, remote_url } => {
                self.profile_picture_downloads
                    .remove(&profile_picture_download_key(&pubkey, &remote_url));
            }
            AsyncMsg::NwcIconCached { remote_url } => {
                self.with_nwc(|nwc, context| nwc.finish_icon_cache(context, remote_url, true));
            }
            AsyncMsg::NwcIconCacheFailed { remote_url } => {
                self.with_nwc(|nwc, context| nwc.finish_icon_cache(context, remote_url, false));
            }
            AsyncMsg::NostrProfilePictureUploaded(url) => {
                self.state.nostr.picture = url.clone();
                self.state.nostr.picture_display_url = url;
                self.state.toast = Some("Profile picture uploaded.".to_string());
                self.request_haptic(HapticFeedback::NotificationSuccess);
                self.save_app_data();
            }
            AsyncMsg::NostrPublished(message) => {
                self.state.toast = Some(message);
            }
            AsyncMsg::DirectMessagesLoaded(messages) => {
                self.state.direct_messages = messages;
            }
            AsyncMsg::DirectMessageSent(message) => {
                self.state.direct_messages.push(message);
                self.state.toast = Some("Message sent.".to_string());
                self.request_haptic(HapticFeedback::NotificationSuccess);
            }
            AsyncMsg::NwcWakeEngineFinished {
                generation,
                request,
                disposition,
            } => self.with_nwc(|nwc, context| {
                nwc.finish_wake_engine(context, generation, request, disposition)
            }),
            AsyncMsg::NwcWakeRequestFailed {
                generation: _,
                event_id,
                error,
            } => self.with_nwc(|nwc, context| nwc.fail_wake_request(context, event_id, error)),
            AsyncMsg::NwcWakeRetryDue {
                generation: _,
                event_id,
            } => self.with_nwc(|nwc, context| nwc.retry_wake_request(context, event_id)),
            AsyncMsg::NwcInfoEventPublished {
                client_pubkey,
                relay,
            } => self.with_nwc(|nwc, context| {
                nwc.finish_info_event(context, client_pubkey, relay, None)
            }),
            AsyncMsg::NwcInfoEventFailed {
                client_pubkey,
                relay,
                error,
            } => self.with_nwc(|nwc, context| {
                nwc.finish_info_event(context, client_pubkey, relay, Some(error))
            }),
            AsyncMsg::NwcPushRegistrationFinished {
                applied,
                deferred,
                next_attempt_at,
                error,
            } => self.with_nwc(|nwc, context| {
                nwc.finish_push_registration(context, applied, deferred, next_attempt_at, error)
            }),
            AsyncMsg::NwcPushRetryDue { nonce } => {
                self.with_nwc(|nwc, context| nwc.handle_push_retry_due(context, nonce))
            }
            AsyncMsg::PriceUpdated { currency, price } => {
                self.state.wallet.price_currency = currency;
                self.state.wallet.btc_price = Some(price);
            }
            AsyncMsg::PriceFailed => {
                self.state.wallet.price_currency = PriceCurrency::BTC;
                self.state.wallet.btc_price = Some(1.0);
            }
            AsyncMsg::Error(message) => {
                if self.state.receive.phase == ReceivePhase::Creating {
                    self.state.receive.phase = ReceivePhase::Editing;
                }
                if self.state.send.phase == SendPhase::Sending {
                    self.state.send.phase = SendPhase::Editing;
                }
                if matches!(
                    self.state.lightning_address.registration_phase,
                    LightningAddressRegistrationPhase::Registering
                        | LightningAddressRegistrationPhase::Verifying
                ) {
                    let has_invoice = self
                        .state
                        .lightning_address
                        .registration_invoice
                        .as_ref()
                        .is_some_and(|invoice| !invoice.trim().is_empty());
                    self.state.lightning_address.registration_phase = if has_invoice {
                        LightningAddressRegistrationPhase::AwaitingPayment
                    } else {
                        LightningAddressRegistrationPhase::Idle
                    };
                    self.state.lightning_address.registration_status_text = if has_invoice {
                        "Awaiting payment".to_string()
                    } else {
                        "Ready".to_string()
                    };
                    self.state
                        .lightning_address
                        .registration_requires_confirmation = false;
                    self.state.lightning_address.registration_error = Some(message.clone());
                }
                if matches!(self.state.setup, SetupState::NeedsSetup) {
                    self.state.setup = SetupState::Error {
                        message: message.clone(),
                    };
                }
                self.state.toast = Some(message);
                self.request_haptic(HapticFeedback::NotificationError);
            }
        }
        self.maybe_start_queued_wallet_work();
        self.reconcile_refresh_poll();
    }

    fn clear_busy_for_async(&mut self, msg: &AsyncMsg) {
        match msg {
            AsyncMsg::WalletReady { .. } => {
                self.state.busy.bootstrapping = false;
                self.state.busy.opening_wallet = false;
            }
            AsyncMsg::WalletOpenFailed { .. }
            | AsyncMsg::WalletDiagnosticsLoaded { .. }
            | AsyncMsg::WalletWorkFinished { .. }
            | AsyncMsg::WalletRefreshPollDue { .. } => {}
            AsyncMsg::ArkAddress(_)
            | AsyncMsg::ReceiveRequest { .. }
            | AsyncMsg::LightningInvoice { .. } => {
                self.state.busy.creating_invoice = false;
            }
            AsyncMsg::Paid { .. } => self.state.busy.sending_payment = false,
            AsyncMsg::LightningAddressReady(_)
            | AsyncMsg::LightningAddressRegistrationUpdated { .. }
            | AsyncMsg::SendFeeEstimateDue { .. }
            | AsyncMsg::SendFeeEstimated { .. }
            | AsyncMsg::SendFeeEstimateFailed { .. } => {}
            AsyncMsg::NostrProfilePictureUploaded(_) => {
                self.state.busy.uploading_profile_picture = false;
            }
            AsyncMsg::NostrPublished(_) => self.state.busy.publishing_nostr = false,
            AsyncMsg::NostrProfileLoaded { .. }
            | AsyncMsg::NostrContactsLoaded(_)
            | AsyncMsg::PrimalContactsLoaded { .. } => self.state.busy.refreshing_contacts = false,
            AsyncMsg::Error(_) => {
                let bootstrapping = self.state.busy.bootstrapping;
                let opening_wallet = self.state.busy.opening_wallet;
                let syncing_wallet = self.state.busy.syncing_wallet;
                let maintaining_vtxos = self.state.busy.maintaining_vtxos;
                self.state.busy = BusyState::default();
                self.state.busy.bootstrapping = bootstrapping;
                self.state.busy.opening_wallet = opening_wallet;
                self.state.busy.syncing_wallet = syncing_wallet;
                self.state.busy.maintaining_vtxos = maintaining_vtxos;
            }
            AsyncMsg::ArkReceiveConfirmed { .. }
            | AsyncMsg::LightningReceiveStatus { .. }
            | AsyncMsg::LightningReceiveClaimed { .. }
            | AsyncMsg::LightningReceivesClaimed { .. }
            | AsyncMsg::ZapReceiptsLoaded { .. }
            | AsyncMsg::Seed(_)
            | AsyncMsg::DirectMessagesLoaded(_)
            | AsyncMsg::DirectMessageSent(_)
            | AsyncMsg::NwcWakeEngineFinished { .. }
            | AsyncMsg::NwcWakeRequestFailed { .. }
            | AsyncMsg::NwcWakeRetryDue { .. }
            | AsyncMsg::NwcInfoEventPublished { .. }
            | AsyncMsg::NwcInfoEventFailed { .. }
            | AsyncMsg::NwcPushRegistrationFinished { .. }
            | AsyncMsg::NwcPushRetryDue { .. }
            | AsyncMsg::NostrSearchLoaded { .. }
            | AsyncMsg::PrimalProfilesLoaded { .. }
            | AsyncMsg::PrimalProfilesFailed { .. }
            | AsyncMsg::ProfilePictureCached { .. }
            | AsyncMsg::ProfilePictureCacheFailed { .. }
            | AsyncMsg::NwcIconCached { .. }
            | AsyncMsg::NwcIconCacheFailed { .. }
            | AsyncMsg::PriceUpdated { .. }
            | AsyncMsg::PriceFailed => {}
        }
    }

    fn emit(&mut self, shared: &Arc<RwLock<AppState>>, tx: &Sender<AppUpdate>) {
        let mut snapshot = self.state.clone();
        snapshot.refresh_derived();
        match shared.write() {
            Ok(mut g) => *g = snapshot.clone(),
            Err(poison) => *poison.into_inner() = snapshot.clone(),
        }
        let _ = tx.send(AppUpdate::FullState(snapshot));
        for feedback in self.pending_haptics.drain(..) {
            let _ = tx.send(AppUpdate::Haptic(feedback));
        }
        for update in self.pending_side_effects.drain(..) {
            let _ = tx.send(update);
        }
    }

    fn request_haptic(&mut self, feedback: HapticFeedback) {
        self.pending_haptics.push(feedback);
    }

    fn request_capability(&mut self, kind: CapabilityRequestKind) {
        self.next_capability_id += 1;
        self.state.capability_request = Some(CapabilityRequest {
            id: self.next_capability_id,
            kind,
        });
        self.request_haptic(HapticFeedback::ImpactLight);
    }

    fn refresh_price(&self) {
        let tx = self.tx.clone();
        let currency = self.state.wallet.price_currency.clone();
        self.rt.spawn(async move {
            let msg = match fetch_bitcoin_price(&currency).await {
                Ok(price) => AsyncMsg::PriceUpdated { currency, price },
                Err(_) => AsyncMsg::PriceFailed,
            };
            let _ = tx.send(CoreMsg::Async(msg));
        });
    }

    fn sync_wallet(&mut self) {
        self.request_wallet_work(WalletWorkRequest::data_changed(WalletWorkKind::Sync));
    }

    fn maintain_vtxos(&mut self) {
        self.request_maintenance(WalletWorkRequest::data_changed(WalletWorkKind::Maintain));
    }

    fn foregrounded(&mut self) {
        self.wallet_foregrounded = true;
        self.cancel_refresh_poll(true);
        self.with_nwc(|nwc, context| nwc.foregrounded(context));
        let maintenance_due = self
            .last_maintenance_completed_at
            .is_none_or(|last| last.elapsed() >= FOREGROUND_MAINTENANCE_INTERVAL);
        if maintenance_due {
            self.request_maintenance(WalletWorkRequest::lifecycle(WalletWorkKind::Maintain));
        } else {
            self.request_wallet_work(WalletWorkRequest::lifecycle(WalletWorkKind::Sync));
        }
    }

    fn backgrounded(&mut self) {
        self.wallet_foregrounded = false;
        self.cancel_refresh_poll(true);
    }

    fn request_maintenance(&mut self, request: WalletWorkRequest) {
        if !request.ensure_after_current
            && self
                .wallet_work
                .in_flight()
                .is_some_and(|token| token.kind >= request.kind)
        {
            let _ = self.wallet_work.request(self.wallet_generation, request);
            self.refresh_wallet_busy_state();
            return;
        }

        if self.state.busy.sending_payment {
            self.wallet_work.defer(request);
            self.refresh_wallet_busy_state();
            return;
        }

        if self.send_screen_blocks_maintenance() {
            self.wallet_work.defer(request);
            self.refresh_wallet_busy_state();
            return;
        }

        self.request_wallet_work(request);
    }

    fn request_wallet_work(&mut self, request: WalletWorkRequest) {
        if self.wallet.is_none() {
            return;
        }
        if self.state.busy.sending_payment {
            self.wallet_work.defer(request);
            self.refresh_wallet_busy_state();
            return;
        }
        if self.send_screen_blocks_maintenance()
            && (request.kind == WalletWorkKind::Maintain
                || self
                    .wallet_work
                    .queued()
                    .is_some_and(|queued| queued.kind == WalletWorkKind::Maintain))
        {
            self.wallet_work.defer(request);
            self.refresh_wallet_busy_state();
            return;
        }

        self.cancel_refresh_poll(false);
        let token = self.wallet_work.request(self.wallet_generation, request);
        self.refresh_wallet_busy_state();
        if let Some(token) = token {
            self.spawn_wallet_work(token);
        }
    }

    fn maybe_start_queued_wallet_work(&mut self) {
        if self.wallet.is_none() || self.state.busy.sending_payment {
            return;
        }
        if self.wallet_work.queued().is_some_and(|request| {
            request.kind == WalletWorkKind::Maintain && self.send_screen_blocks_maintenance()
        }) {
            return;
        }

        let token = self.wallet_work.start_queued(self.wallet_generation);
        self.refresh_wallet_busy_state();
        if let Some(token) = token {
            self.cancel_refresh_poll(false);
            self.spawn_wallet_work(token);
        }
    }

    fn spawn_wallet_work(&self, token: WalletWorkToken) {
        let Some(wallet) = self.wallet.clone() else {
            return;
        };
        let tx = self.tx.clone();
        let contacts = self.state.nostr.contacts.clone();
        let lightning_address = self.state.lightning_address.clone();
        let payment_annotations = self.payment_annotations.clone();
        let zap_receipts = self.zap_receipts.clone();
        let db_path = self.data_dir.join(self.state.wallet.network.db_file_name());
        self.rt.spawn(async move {
            let work = async {
                match token.kind {
                    WalletWorkKind::Load => {
                        crate::wallet_reconciliation::reconcile(&wallet, &db_path).await?;
                    }
                    WalletWorkKind::Sync => {
                        wallet.sync().await;
                        crate::wallet_reconciliation::reconcile(&wallet, &db_path).await?;
                        wallet
                            .progress_pending_rounds(None)
                            .await
                            .context("pending round reconciliation failed")?;
                    }
                    WalletWorkKind::Maintain => {
                        wallet.sync().await;
                        crate::wallet_reconciliation::reconcile(&wallet, &db_path).await?;
                        wallet.progress_pending_rounds(None).await?;
                        wallet.maybe_schedule_maintenance_refresh_delegated().await?;
                    }
                    WalletWorkKind::ForceRefresh => {
                        crate::wallet_reconciliation::reconcile(&wallet, &db_path).await?;
                        wallet.progress_pending_rounds(None).await
                            .context("pending round reconciliation failed")?;
                        let pending = wallet.pending_round_input_vtxos().await?;
                        let vtxos: Vec<_> = wallet.spendable_vtxos().await?.into_iter()
                            .filter(|vtxo| !pending.iter().any(|input| input.vtxo.id() == vtxo.vtxo.id()))
                            .collect();
                        anyhow::ensure!(!vtxos.is_empty(), "No locally spendable VTXOs outside pending rounds available to refresh.");
                        let eligible_sat: u64 = vtxos.iter().map(|v| v.vtxo.amount().to_sat()).sum();
                        anyhow::ensure!(eligible_sat >= ark::vtxo::VTXO_DUST_SAT,
                            "Only {eligible_sat} sats are eligible outside pending rounds; a refresh requires at least {} sats before fees. No refresh submitted", ark::vtxo::VTXO_DUST_SAT);
                        wallet.refresh_vtxos_delegated(vtxos).await
                            .context("VTXO refresh request failed")?
                            .context("No refresh was submitted")?;
                    }
                }

                wallet_synced_msg(
                    &wallet,
                    &contacts,
                    &lightning_address,
                    &payment_annotations,
                    &zap_receipts,
                )
                .await
            };
            let result = match tokio::time::timeout(WALLET_WORK_TIMEOUT, work).await {
                Ok(Ok(snapshot)) => Ok(snapshot),
                Ok(Err(error)) => Err(format!("{error:#}")),
                Err(_) => Err(format!(
                    "Wallet {} timed out after {} seconds.",
                    match token.kind {
                        WalletWorkKind::Load => "load",
                        WalletWorkKind::Sync => "sync",
                        WalletWorkKind::Maintain => "maintenance",
                        WalletWorkKind::ForceRefresh => "refresh request (it may already have been submitted; reload diagnostics before retrying)",
                    },
                    WALLET_WORK_TIMEOUT.as_secs(),
                )),
            };
            let _ = tx.send(CoreMsg::Async(AsyncMsg::WalletWorkFinished {
                generation: token.generation,
                operation_id: token.id,
                result,
            }));
        });
    }

    fn finish_wallet_work(
        &mut self,
        generation: u64,
        operation_id: u64,
        result: Result<WalletSnapshot, String>,
    ) {
        let Some(token) = self.wallet_work.finish(generation, operation_id) else {
            return;
        };
        self.refresh_wallet_busy_state();

        if token.kind == WalletWorkKind::ForceRefresh {
            self.state.wallet_refresh_running = false;
            self.state.wallet_refresh_status = match &result {
                Ok(_) => "Refresh submitted, not yet confirmed complete. Keep the app open and reload local records to check REFRESH HISTORY.".to_string(),
                Err(message) => format!("Refresh did not finish successfully: {message}. Check REFRESH HISTORY before retrying; a request may already have been submitted."),
            };
            if let Ok(snapshot) = result {
                self.apply_wallet_snapshot(snapshot);
            }
            // Explicit refreshes are never retried automatically.
            self.load_wallet_diagnostics();
            return;
        }

        match result {
            Ok(snapshot) => {
                if token.kind == WalletWorkKind::Maintain {
                    self.last_maintenance_completed_at = Some(Instant::now());
                }
                if self
                    .wallet_retry_kind
                    .is_some_and(|retry_kind| token.kind >= retry_kind)
                {
                    self.wallet_retry_kind = None;
                }
                if self.wallet_retry_kind.is_none() {
                    self.state.wallet.sync_error = None;
                }
                self.apply_wallet_snapshot(snapshot);
            }
            Err(message) => {
                self.wallet_retry_kind = Some(
                    self.wallet_retry_kind
                        .map_or(token.kind, |kind| kind.max(token.kind)),
                );
                self.state.wallet.sync_error = Some(match token.kind {
                    WalletWorkKind::Load => {
                        "Wallet state failed to load. It will retry automatically.".to_string()
                    }
                    WalletWorkKind::Sync => {
                        "Wallet sync failed. It will retry automatically.".to_string()
                    }
                    WalletWorkKind::Maintain => {
                        "Wallet maintenance failed. It will retry automatically.".to_string()
                    }
                    WalletWorkKind::ForceRefresh => unreachable!("handled above"),
                });
                if token.report_errors {
                    self.state.toast = Some(match token.kind {
                        WalletWorkKind::Load => format!("Wallet load failed: {message}"),
                        WalletWorkKind::Sync => format!("Sync failed: {message}"),
                        WalletWorkKind::Maintain => {
                            format!("Wallet maintenance failed: {message}")
                        }
                        WalletWorkKind::ForceRefresh => unreachable!("handled above"),
                    });
                    self.request_haptic(HapticFeedback::NotificationError);
                }
            }
        }
    }

    fn apply_wallet_snapshot(&mut self, snapshot: WalletSnapshot) {
        self.state.wallet.balance_sat = snapshot.balance_sat;
        self.state.wallet.pending_receive_sat = snapshot.pending_receive_sat;
        self.state.wallet.stuck_receive_sat = snapshot.stuck_receive_sat;
        self.state.wallet.pending_send_sat = snapshot.pending_send_sat;
        if let Some(pending_refresh_sat) = snapshot.pending_refresh_sat {
            self.state.wallet.pending_refresh_sat = pending_refresh_sat;
        }
        self.has_pending_rounds = snapshot.has_pending_rounds;
        self.state.wallet.last_sync = Some(now_label());
        self.state.activity = snapshot.activity;
        self.prefetch_activity_profile_pictures();
        self.scan_zap_receipts();
    }

    fn refresh_activity_metadata(&mut self) {
        apply_activity_metadata(
            &mut self.state.activity,
            &self.state.nostr.contacts,
            &self.payment_annotations,
            &self.zap_receipts,
        );
    }

    fn refresh_wallet_busy_state(&mut self) {
        let in_flight = self.wallet_work.in_flight();
        self.state.busy.syncing_wallet = in_flight.is_some();
        self.state.busy.maintaining_vtxos =
            in_flight.is_some_and(|token| token.kind == WalletWorkKind::Maintain);
    }

    fn send_screen_blocks_maintenance(&self) -> bool {
        self.state
            .router
            .screen_stack
            .iter()
            .any(|screen| matches!(screen, Screen::Send))
            && self.state.send.phase != SendPhase::Success
    }

    fn is_stale_wallet_async(&self, msg: &AsyncMsg) -> bool {
        let generation = match msg {
            AsyncMsg::WalletReady { generation, .. }
            | AsyncMsg::WalletDiagnosticsLoaded { generation, .. }
            | AsyncMsg::WalletOpenFailed { generation, .. }
            | AsyncMsg::WalletWorkFinished { generation, .. }
            | AsyncMsg::WalletRefreshPollDue { generation, .. }
            | AsyncMsg::NwcWakeEngineFinished { generation, .. }
            | AsyncMsg::NwcWakeRequestFailed { generation, .. }
            | AsyncMsg::NwcWakeRetryDue { generation, .. } => Some(*generation),
            _ => None,
        };
        generation.is_some_and(|generation| generation != self.wallet_generation)
    }

    fn handle_refresh_poll_due(&mut self, generation: u64, nonce: u64) {
        if generation != self.wallet_generation
            || nonce != self.refresh_poll_nonce
            || !self.refresh_poll_scheduled
        {
            return;
        }
        self.refresh_poll_scheduled = false;
        if !self.wallet_foregrounded {
            return;
        }

        let kind = self.wallet_retry_kind.unwrap_or(WalletWorkKind::Sync);
        let request = WalletWorkRequest::lifecycle(kind);
        if kind == WalletWorkKind::Maintain {
            self.request_maintenance(request);
        } else {
            self.request_wallet_work(request);
        }
    }

    fn reconcile_refresh_poll(&mut self) {
        let needs_poll = self.has_pending_rounds || self.wallet_retry_kind.is_some();
        if !self.wallet_foregrounded || !needs_poll {
            if !needs_poll {
                self.cancel_refresh_poll(true);
            }
            return;
        }
        if self.wallet_work.has_work() || self.refresh_poll_scheduled {
            return;
        }

        let delay = refresh_poll_delay(self.refresh_poll_attempt);
        self.refresh_poll_attempt = self.refresh_poll_attempt.saturating_add(1);
        self.refresh_poll_nonce = self.refresh_poll_nonce.wrapping_add(1);
        let nonce = self.refresh_poll_nonce;
        let generation = self.wallet_generation;
        let tx = self.tx.clone();
        self.refresh_poll_scheduled = true;
        self.rt.spawn(async move {
            tokio::time::sleep(delay).await;
            let _ = tx.send(CoreMsg::Async(AsyncMsg::WalletRefreshPollDue {
                generation,
                nonce,
            }));
        });
    }

    fn cancel_refresh_poll(&mut self, reset_attempts: bool) {
        if self.refresh_poll_scheduled {
            self.refresh_poll_nonce = self.refresh_poll_nonce.wrapping_add(1);
            self.refresh_poll_scheduled = false;
        }
        if reset_attempts {
            self.refresh_poll_attempt = 0;
        }
    }

    pub(super) fn invalidate_wallet_session(&mut self) -> u64 {
        self.wallet_generation = self.wallet_generation.wrapping_add(1).max(1);
        self.wallet = None;
        self.state.wallet_diagnostics.clear();
        self.state.wallet_diagnostics_loading = false;
        self.state.wallet_refresh_running = false;
        self.state.wallet_refresh_status.clear();
        self.wallet_work.reset();
        self.last_maintenance_completed_at = None;
        self.wallet_retry_kind = None;
        self.has_pending_rounds = false;
        self.nwc.reset_wallet_session();
        self.state.wallet.sync_error = None;
        self.cancel_refresh_poll(true);
        self.refresh_wallet_busy_state();
        self.wallet_generation
    }

    fn create_ark_address(&mut self) {
        let Some(wallet) = self.wallet.clone() else {
            return;
        };
        self.state.receive.phase = ReceivePhase::Creating;
        self.state.busy.creating_invoice = true;
        let tx = self.tx.clone();
        self.rt.spawn(async move {
            match wallet.new_address().await {
                Ok(address) => {
                    monitor_ark_receive(wallet, tx, address).await;
                }
                Err(e) => {
                    let _ = tx.send(CoreMsg::Async(AsyncMsg::Error(format!(
                        "Could not create Ark address: {e:#}"
                    ))));
                }
            }
        });
    }

    fn create_receive_request(&mut self) {
        let Some(mut wallet) = self.wallet.clone() else {
            return;
        };
        let amount_sat = self.state.receive.amount_sat;
        if amount_sat == 0 {
            self.state.toast = Some("Enter an amount to create a Lightning request.".to_string());
            self.request_haptic(HapticFeedback::NotificationWarning);
            return;
        }

        self.state.receive.phase = ReceivePhase::Creating;
        self.state.receive.receive_request = None;
        self.state.receive.ark_address = None;
        self.state.receive.lightning_invoice = None;
        self.state.receive.lightning_payment_hash = None;
        self.state.receive.lightning_status = "waiting".to_string();
        self.state.receive.lightning_paid = false;
        self.state.busy.creating_invoice = true;
        self.request_haptic(HapticFeedback::ImpactMedium);

        let memo = self.state.receive.memo.trim().to_string();
        let tx = self.tx.clone();
        thread::spawn(move || {
            let rt = Runtime::new().expect("tokio runtime");
            let result_tx = tx.clone();
            let result = rt.block_on(async move {
                let mut builder = wallet.bip321_uri().amount(Amount::from_sat(amount_sat));
                if !memo.is_empty() {
                    builder = builder.message(memo);
                }
                let uri = builder.build().await?;
                let uri_text = uri.to_string();
                let request = wallet
                    .parse_payment_request(&uri_text)
                    .await
                    .context("failed to parse generated BIP321 request")?;

                let ark_address = request
                    .options
                    .iter()
                    .find_map(|option| match &option.method {
                        BarkPaymentMethod::Ark(address) => Some(address.clone()),
                        _ => None,
                    })
                    .context("generated BIP321 request did not include an Ark address")?;
                let lightning_invoice = request
                    .options
                    .iter()
                    .find_map(|option| match &option.method {
                        BarkPaymentMethod::Invoice(invoice) => Some(invoice.to_string()),
                        _ => None,
                    })
                    .context("generated BIP321 request did not include a Lightning invoice")?;
                let invoice = Bolt11Invoice::from_str(&lightning_invoice)
                    .context("generated Lightning invoice was invalid")?;
                let payment_hash: PaymentHash = (*invoice.payment_hash()).into();
                let payment_hash_text = payment_hash.to_string();

                let _ = result_tx.send(CoreMsg::Async(AsyncMsg::ReceiveRequest {
                    uri: uri_text,
                    ark_address: ark_address.to_string(),
                    lightning_invoice,
                    payment_hash: payment_hash_text,
                }));

                let ark_wallet = wallet.clone();
                let ark_tx = result_tx.clone();
                tokio::spawn(async move {
                    monitor_ark_receive(ark_wallet, ark_tx, ark_address).await;
                });
                monitor_lightning_receive(wallet, result_tx, payment_hash).await;

                anyhow::Ok(())
            });

            if let Err(e) = result {
                let _ = tx.send(CoreMsg::Async(AsyncMsg::Error(format!(
                    "Could not create receive request: {e:#}"
                ))));
            }
        });
    }

    /// Re-attempt an in-flight Lightning receive after the app returns to the
    /// foreground. iOS suspends the whole process while backgrounded, which can
    /// stop the original monitor before the payment is claimed. Restarting the
    /// monitor ensures a payment that arrived while suspended is claimed as soon
    /// as the user reopens the app. No-op unless we are still showing an unpaid
    /// Lightning request.
    fn resume_receive_monitor(&mut self) {
        if self.state.receive.phase != ReceivePhase::ShowingRequest
            || self.state.receive.lightning_paid
        {
            return;
        }
        let Some(payment_hash_text) = self.state.receive.lightning_payment_hash.clone() else {
            return;
        };
        let Some(wallet) = self.wallet.clone() else {
            return;
        };
        let Ok(payment_hash) = PaymentHash::from_str(&payment_hash_text) else {
            return;
        };
        let tx = self.tx.clone();
        self.rt.spawn(async move {
            monitor_lightning_receive(wallet, tx, payment_hash).await;
        });
    }

    /// Claim any Lightning receives whose HTLCs have already arrived but were
    /// never finalized — for example because the app was suspended or closed
    /// before the receive monitor could reveal the preimage. Unlike the receive
    /// monitor, this sweep is independent of the receive screen: it runs on
    /// wallet open and whenever the app returns to the foreground, so a payment
    /// that landed while we were away no longer gets stuck in "claimable".
    fn claim_pending_lightning_receives(&mut self) {
        let Some(wallet) = self.wallet.clone() else {
            return;
        };
        if self.state.busy.claiming_lightning_receives {
            return;
        }
        self.state.busy.claiming_lightning_receives = true;
        let tx = self.tx.clone();
        self.rt.spawn(async move {
            let pending = match wallet.pending_lightning_receives().await {
                Ok(pending) => pending,
                Err(_) => {
                    let _ = tx.send(CoreMsg::Async(AsyncMsg::LightningReceivesClaimed {
                        payment_hashes: vec![],
                    }));
                    return;
                }
            };
            let claimable: Vec<_> = pending
                .into_iter()
                .filter(|receive| matches!(&receive.progress, ReceiveProgress::HtlcsReady(_)))
                .collect();
            if claimable.is_empty() {
                let _ = tx.send(CoreMsg::Async(AsyncMsg::LightningReceivesClaimed {
                    payment_hashes: vec![],
                }));
                return;
            }
            let mut claimed = vec![];
            for receive in claimable {
                let payment_hash = receive.payment_hash;
                if let Ok(Ok(state)) = tokio::time::timeout(
                    Duration::from_secs(30),
                    wallet.try_claim_lightning_receive(payment_hash, false),
                )
                .await
                {
                    if matches!(
                        state,
                        LightningReceiveState::Settled(_)
                            | LightningReceiveState::InProgress(
                                bark::actions::lightning::receive::LightningReceive {
                                    progress: ReceiveProgress::PreimageRevealed(_)
                                        | ReceiveProgress::Delivering(_),
                                    ..
                                }
                            )
                    ) {
                        claimed.push(payment_hash.to_string());
                    }
                }
            }
            let _ = tx.send(CoreMsg::Async(AsyncMsg::LightningReceivesClaimed {
                payment_hashes: claimed,
            }));
        });
    }

    fn create_lightning_invoice(&mut self) {
        let Some(wallet) = self.wallet.clone() else {
            return;
        };
        self.state.receive.phase = ReceivePhase::Creating;
        self.state.busy.creating_invoice = true;
        let amount_sat = self.state.receive.amount_sat;
        let memo = self.state.receive.memo.trim().to_string();
        let memo = if memo.is_empty() { None } else { Some(memo) };
        let tx = self.tx.clone();
        self.rt.spawn(async move {
            match wallet
                .bolt11_invoice(Amount::from_sat(amount_sat), memo, None)
                .await
            {
                Ok(invoice) => {
                    let payment_hash: PaymentHash = (*invoice.payment_hash()).into();
                    let payment_hash_text = payment_hash.to_string();
                    let _ = tx.send(CoreMsg::Async(AsyncMsg::LightningInvoice {
                        invoice: invoice.to_string(),
                        payment_hash: payment_hash_text,
                    }));
                    monitor_lightning_receive(wallet, tx, payment_hash).await;
                }
                Err(e) => {
                    let _ = tx.send(CoreMsg::Async(AsyncMsg::Error(format!(
                        "Could not create Lightning invoice: {e:#}"
                    ))));
                }
            }
        });
    }

    /// Save the wallet seed to the Keychain, retrying once before giving up.
    fn save_wallet_seed(&self, mnemonic: &str) -> bool {
        if self
            .secrets
            .set_secret(WALLET_SEED_KEY.to_string(), mnemonic.to_string())
        {
            return true;
        }
        self.secrets
            .set_secret(WALLET_SEED_KEY.to_string(), mnemonic.to_string())
    }

    /// Save the Nostr secret to the Keychain, retrying once. On failure a
    /// warning toast is shown instead of silently proceeding.
    fn persist_nostr_secret(&mut self, nsec: &str) -> bool {
        let saved = self
            .secrets
            .set_secret(NOSTR_SECRET_KEY.to_string(), nsec.to_string())
            || self
                .secrets
                .set_secret(NOSTR_SECRET_KEY.to_string(), nsec.to_string());
        if !saved {
            self.state.toast = Some(
                "Could not save the Nostr key to the Keychain. It will not survive an app restart."
                    .to_string(),
            );
            self.request_haptic(HapticFeedback::NotificationWarning);
        }
        saved
    }

    fn generate_nostr_key(&mut self) {
        let keys = Keys::generate();
        match (keys.secret_key().to_bech32(), keys.public_key().to_bech32()) {
            (Ok(nsec), Ok(npub)) => {
                let nsec = Zeroizing::new(nsec);
                if self.persist_nostr_secret(nsec.as_str()) {
                    self.reset_nostr_identity(npub);
                    self.state.toast = Some("Nostr key generated in Keychain.".to_string());
                    self.request_haptic(HapticFeedback::NotificationSuccess);
                    self.save_app_data();
                    self.sync_primal_follow_contacts(false);
                }
            }
            _ => {
                self.state.toast = Some("Could not encode generated Nostr key.".to_string());
                self.request_haptic(HapticFeedback::NotificationError);
            }
        }
    }

    fn import_nostr_secret(&mut self, nsec_or_hex: String) {
        let value = Zeroizing::new(nsec_or_hex.trim().to_string());
        if value.is_empty() {
            self.state.toast = Some("Enter a Nostr secret key.".to_string());
            self.request_haptic(HapticFeedback::NotificationWarning);
            return;
        }
        match Keys::parse(value.as_str()) {
            Ok(keys) => match (keys.secret_key().to_bech32(), keys.public_key().to_bech32()) {
                (Ok(nsec), Ok(npub)) => {
                    let nsec = Zeroizing::new(nsec);
                    if self.persist_nostr_secret(nsec.as_str()) {
                        self.reset_nostr_identity(npub);
                        self.state.toast =
                            Some("Nostr key imported. Refreshing profile...".to_string());
                        self.request_haptic(HapticFeedback::NotificationSuccess);
                        self.save_app_data();
                        self.refresh_nostr_profile();
                        self.sync_primal_follow_contacts(false);
                    }
                }
                _ => {
                    self.state.toast = Some("Could not encode imported Nostr key.".to_string());
                    self.request_haptic(HapticFeedback::NotificationError);
                }
            },
            Err(_) => {
                self.state.toast = Some("Invalid Nostr secret key.".to_string());
                self.request_haptic(HapticFeedback::NotificationError);
            }
        }
    }

    fn export_nostr_secret(&mut self) {
        let secret = self
            .secrets
            .get_secret(NOSTR_SECRET_KEY.to_string())
            .map(Zeroizing::new);
        if let Some(secret) = secret {
            self.state.revealed_nostr_secret = Some((*secret).clone());
            self.request_haptic(HapticFeedback::NotificationWarning);
        } else {
            self.state.toast = Some("No Nostr secret key found.".to_string());
            self.request_haptic(HapticFeedback::NotificationWarning);
        }
    }

    fn clear_nostr_key(&mut self) {
        let _ = self.secrets.delete_secret(NOSTR_SECRET_KEY.to_string());
        if !self.ensure_wallet_derived_nostr_key() {
            self.state.nostr.npub = None;
            self.state.nostr.name = "Rebel".to_string();
            self.state.nostr.about.clear();
            self.state.nostr.picture.clear();
            self.state.nostr.picture_display_url.clear();
            self.state.nostr.lud16.clear();
            self.state.nostr.nip05.clear();
            self.state.nostr.deleted = false;
            self.state.nostr.contacts.clear();
            self.state.direct_messages.clear();
        }
        self.save_app_data();
        self.request_haptic(HapticFeedback::NotificationWarning);
    }

    fn clear_nostr_profile_cache(&mut self) {
        if let Some(conn) = self.profile_db.as_ref() {
            let _ = clear_profile_cache(conn);
        }
        let _ = clear_profile_picture_dir(&self.cache_dir);
        self.profile_picture_downloads.clear();
        self.profile_info_requests.clear();

        for contact in &mut self.state.nostr.contacts {
            contact.picture.clear();
        }
        for contact in &mut self.state.send.global_search_results {
            contact.picture.clear();
        }
        self.state.nostr.picture_display_url.clear();
        self.state.send.global_search_results.clear();
        self.state.toast = Some("Nostr profile cache cleared.".to_string());
        self.request_haptic(HapticFeedback::NotificationWarning);
        self.save_app_data();
    }

    fn load_nostr_key(&mut self) {
        let secret = self
            .secrets
            .get_secret(NOSTR_SECRET_KEY.to_string())
            .map(Zeroizing::new);
        if let Some(secret) = secret {
            if let Ok(keys) = Keys::parse(secret.as_str()) {
                let npub = keys.public_key().to_bech32().unwrap();
                if self.state.nostr.npub.as_deref() != Some(npub.as_str()) {
                    self.reset_nostr_identity(npub);
                    self.save_app_data();
                }
                self.refresh_nostr_profile();
                self.sync_primal_follow_contacts(false);
            }
        }
    }

    fn ensure_wallet_derived_nostr_key(&mut self) -> bool {
        if self
            .secrets
            .get_secret(NOSTR_SECRET_KEY.to_string())
            .is_some()
        {
            return true;
        }

        let Some(mnemonic) = self
            .secrets
            .get_secret(WALLET_SEED_KEY.to_string())
            .map(Zeroizing::new)
        else {
            return false;
        };

        let Ok(keys) = derive_nostr_keys_from_mnemonic(mnemonic.as_str()) else {
            return false;
        };

        match (keys.secret_key().to_bech32(), keys.public_key().to_bech32()) {
            (Ok(nsec), Ok(npub)) => {
                let nsec = Zeroizing::new(nsec);
                // The key is re-derivable from the wallet seed, so a failed
                // Keychain write is not fatal here; warn and continue.
                self.persist_nostr_secret(nsec.as_str());
                self.reset_nostr_identity(npub);
                self.save_app_data();
                self.refresh_nostr_profile();
                self.sync_primal_follow_contacts(false);
                true
            }
            _ => false,
        }
    }

    fn reset_nostr_identity(&mut self, npub: String) {
        self.state.nostr.npub = Some(npub);
        self.state.nostr.name = "Rebel".to_string();
        self.state.nostr.about.clear();
        self.state.nostr.picture.clear();
        self.state.nostr.picture_display_url.clear();
        self.state.nostr.lud16.clear();
        self.state.nostr.nip05.clear();
        self.state.nostr.deleted = false;
        self.state.nostr.contacts.clear();
        self.state.direct_messages.clear();
    }

    fn nostr_keys(&self) -> anyhow::Result<Keys> {
        let secret = self
            .secrets
            .get_secret(NOSTR_SECRET_KEY.to_string())
            .map(Zeroizing::new)
            .context("Nostr secret key not found")?;
        Keys::parse(secret.as_str()).context("invalid Nostr secret key")
    }

    fn publish_nostr_profile(&mut self) {
        if self.state.nostr.deleted {
            self.delete_nostr_profile();
            return;
        }

        let keys = match self.nostr_keys() {
            Ok(keys) => keys,
            Err(e) => {
                let _ = self
                    .tx
                    .send(CoreMsg::Async(AsyncMsg::Error(format!("{e:#}"))));
                return;
            }
        };
        self.state.busy.publishing_nostr = true;
        let nostr = self.state.nostr.clone();
        let tx = self.tx.clone();
        self.rt.spawn(async move {
            let result = async {
                let metadata = metadata_from_state(&nostr)?;
                let client = nostr_client().await?;
                let event = EventBuilder::metadata(&metadata).finalize(&keys)?;
                let out = client.send_event(&event).await?;
                Ok::<_, anyhow::Error>(AsyncMsg::NostrPublished(format!(
                    "Published profile to {} relays.",
                    out.success.len()
                )))
            }
            .await
            .unwrap_or_else(|e| AsyncMsg::Error(format!("Nostr profile publish failed: {e:#}")));
            let _ = tx.send(CoreMsg::Async(result));
        });
    }

    fn refresh_nostr_profile(&mut self) {
        let keys = match self.nostr_keys() {
            Ok(keys) => keys,
            Err(e) => {
                let _ = self
                    .tx
                    .send(CoreMsg::Async(AsyncMsg::Error(format!("{e:#}"))));
                return;
            }
        };
        self.state.busy.refreshing_contacts = true;
        let mut nostr = self.state.nostr.clone();
        let tx = self.tx.clone();
        self.rt.spawn(async move {
            let result = async {
                let client = nostr_client().await?;
                let filter = Filter::new()
                    .author(keys.public_key())
                    .kind(Kind::Metadata)
                    .limit(10);
                let events = client
                    .fetch_events(filter)
                    .timeout(Duration::from_secs(10))
                    .await?;
                let mut profile = None;
                if let Some(event) = events.iter().max_by_key(|event| event.created_at.as_secs()) {
                    apply_metadata_content(&mut nostr, &event.content)?;
                    profile = Some(profile_contact_from_metadata_json(
                        event.pubkey,
                        event.content.clone(),
                        event.created_at.as_secs(),
                        true,
                    ));
                }
                Ok::<_, anyhow::Error>(AsyncMsg::NostrProfileLoaded { nostr, profile })
            }
            .await
            .unwrap_or_else(|e| AsyncMsg::Error(format!("Nostr profile refresh failed: {e:#}")));
            let _ = tx.send(CoreMsg::Async(result));
        });
    }

    fn delete_nostr_profile(&mut self) {
        let keys = match self.nostr_keys() {
            Ok(keys) => keys,
            Err(e) => {
                let _ = self
                    .tx
                    .send(CoreMsg::Async(AsyncMsg::Error(format!("{e:#}"))));
                return;
            }
        };
        mark_profile_deleted(&mut self.state.nostr);
        self.save_app_data();
        self.state.busy.publishing_nostr = true;
        let tx = self.tx.clone();
        self.rt.spawn(async move {
            let result = async {
                let client = nostr_client().await?;
                let content = deleted_profile_content();
                let event = EventBuilder::new(Kind::Metadata, content).finalize(&keys)?;
                let out = client.send_event(&event).await?;
                Ok::<_, anyhow::Error>(AsyncMsg::NostrPublished(format!(
                    "Published profile deletion to {} relays.",
                    out.success.len()
                )))
            }
            .await
            .unwrap_or_else(|e| AsyncMsg::Error(format!("Nostr profile delete failed: {e:#}")));
            let _ = tx.send(CoreMsg::Async(result));
        });
    }

    fn upload_nostr_profile_picture(&mut self, image_base64: String) {
        if self.state.nostr.deleted {
            self.state.toast = Some("Deleted profiles cannot be edited.".to_string());
            return;
        }

        let keys = match self.nostr_keys() {
            Ok(keys) => keys,
            Err(e) => {
                let _ = self
                    .tx
                    .send(CoreMsg::Async(AsyncMsg::Error(format!("{e:#}"))));
                return;
            }
        };
        self.state.busy.uploading_profile_picture = true;
        let tx = self.tx.clone();
        self.rt.spawn(async move {
            let result = upload_profile_picture(keys, image_base64)
                .await
                .map(AsyncMsg::NostrProfilePictureUploaded)
                .unwrap_or_else(|e| {
                    AsyncMsg::Error(format!("Profile picture upload failed: {e:#}"))
                });
            let _ = tx.send(CoreMsg::Async(result));
        });
    }

    fn publish_contact_list(&mut self) {
        let keys = match self.nostr_keys() {
            Ok(keys) => keys,
            Err(e) => {
                let _ = self
                    .tx
                    .send(CoreMsg::Async(AsyncMsg::Error(format!("{e:#}"))));
                return;
            }
        };
        self.state.busy.publishing_nostr = true;
        let contacts = self.state.nostr.contacts.clone();
        let tx = self.tx.clone();
        self.rt.spawn(async move {
            let result = async {
                let nostr_contacts = contacts
                    .iter()
                    .filter(|c| c.followed)
                    .filter_map(|c| public_key_from_npub_or_hex(&c.npub).ok())
                    .map(NostrContact::new)
                    .collect::<Vec<_>>();
                let event = ContactListBuilder::new(nostr_contacts)
                    .build()
                    .finalize(&keys)?;
                let client = nostr_client().await?;
                let out = client.send_event(&event).await?;
                Ok::<_, anyhow::Error>(AsyncMsg::NostrPublished(format!(
                    "Published contact list to {} relays.",
                    out.success.len()
                )))
            }
            .await
            .unwrap_or_else(|e| AsyncMsg::Error(format!("Nostr contact publish failed: {e:#}")));
            let _ = tx.send(CoreMsg::Async(result));
        });
    }

    fn refresh_contact_list(&mut self) {
        self.sync_primal_follow_contacts(true);
    }

    fn sync_primal_follow_contacts(&mut self, show_toast: bool) {
        let keys = match self.nostr_keys() {
            Ok(keys) => keys,
            Err(e) => {
                let _ = self
                    .tx
                    .send(CoreMsg::Async(AsyncMsg::Error(format!("{e:#}"))));
                return;
            }
        };
        if show_toast {
            self.state.busy.refreshing_contacts = true;
        }
        let tx = self.tx.clone();
        self.rt.spawn(async move {
            let result = async {
                let contacts = primal_follow_contacts(keys.public_key()).await?;
                if !contacts.is_empty() {
                    return Ok::<_, anyhow::Error>(AsyncMsg::PrimalContactsLoaded {
                        records: contacts,
                        show_toast,
                    });
                }

                if !show_toast {
                    return Ok::<_, anyhow::Error>(AsyncMsg::PrimalContactsLoaded {
                        records: Vec::new(),
                        show_toast,
                    });
                }

                let client = nostr_client().await?;
                let filter = Filter::new()
                    .author(keys.public_key())
                    .kind(Kind::ContactList)
                    .limit(1);
                let events = client
                    .fetch_events(filter)
                    .timeout(Duration::from_secs(10))
                    .await?;
                let mut contacts = Vec::new();
                if let Some(event) = events.first() {
                    let mut pubkeys = Vec::new();
                    for tag in event.tags.iter() {
                        let fields = tag.as_slice();
                        if fields.first().map(|s| s.as_str()) != Some("p") {
                            continue;
                        }
                        let Some(pubkey) = fields.get(1) else {
                            continue;
                        };
                        let Ok(key) = NostrPublicKey::from_hex(pubkey) else {
                            continue;
                        };
                        pubkeys.push(key);
                        let npub = key.to_bech32().unwrap_or_else(|_| pubkey.to_string());
                        contacts.push(Contact {
                            id: contact_id(&npub),
                            npub: npub.clone(),
                            name: nostr_contact_display_name(
                                None,
                                None,
                                fields.get(3).cloned(),
                                &npub,
                            ),
                            followed: true,
                            picture: String::new(),
                            lightning_address: String::new(),
                            lnurl: String::new(),
                            last_used: now_unix(),
                        });
                    }
                    if !pubkeys.is_empty() {
                        let metadata_filter = Filter::new()
                            .authors(pubkeys.clone())
                            .kind(Kind::Metadata)
                            .limit(250);
                        let metadata_events = client
                            .fetch_events(metadata_filter)
                            .timeout(Duration::from_secs(10))
                            .await?;
                        let mut records = metadata_events
                            .iter()
                            .map(|event| {
                                let npub = event
                                    .pubkey
                                    .to_bech32()
                                    .unwrap_or_else(|_| event.pubkey.to_hex());
                                let petname = contacts
                                    .iter()
                                    .find(|contact| contact.npub == npub)
                                    .map(|contact| contact.name.clone());
                                profile_contact_from_metadata_json_with_petname(
                                    event.pubkey,
                                    event.content.clone(),
                                    event.created_at.as_secs(),
                                    true,
                                    petname,
                                )
                            })
                            .collect::<Vec<_>>();
                        for contact in contacts {
                            if records
                                .iter()
                                .any(|record| record.contact.npub == contact.npub)
                            {
                                continue;
                            }
                            let Ok(key) = public_key_from_npub_or_hex(&contact.npub) else {
                                continue;
                            };
                            let mut record =
                                profile_contact_from_metadata_json(key, "{}".to_string(), 0, true);
                            record.contact.name = contact.name;
                            record.contact.lightning_address = contact.lightning_address;
                            record.contact.lnurl = contact.lnurl;
                            records.push(record);
                        }
                        return Ok::<_, anyhow::Error>(AsyncMsg::NostrContactsLoaded(records));
                    }
                }
                let records = contacts
                    .into_iter()
                    .filter_map(|contact| {
                        let key = public_key_from_npub_or_hex(&contact.npub).ok()?;
                        let mut record =
                            profile_contact_from_metadata_json(key, "{}".to_string(), 0, true);
                        record.contact.name = contact.name;
                        record.contact.lightning_address = contact.lightning_address;
                        record.contact.lnurl = contact.lnurl;
                        Some(record)
                    })
                    .collect();
                Ok::<_, anyhow::Error>(AsyncMsg::NostrContactsLoaded(records))
            }
            .await
            .unwrap_or_else(|e| {
                if show_toast {
                    AsyncMsg::Error(format!("Nostr contact refresh failed: {e:#}"))
                } else {
                    AsyncMsg::PrimalContactsLoaded {
                        records: Vec::new(),
                        show_toast,
                    }
                }
            });
            let _ = tx.send(CoreMsg::Async(result));
        });
    }

    fn search_nostr_profiles(&mut self) {
        let query = self.state.send.search_query.trim().to_string();
        if query.len() < 2 {
            self.state.send.global_search_results.clear();
            return;
        }

        let tx = self.tx.clone();
        self.rt.spawn(async move {
            let result = match primal_search_profiles(&query).await {
                Ok(contacts) => AsyncMsg::NostrSearchLoaded { query, contacts },
                Err(_) => AsyncMsg::NostrSearchLoaded {
                    query,
                    contacts: Vec::new(),
                },
            };
            let _ = tx.send(CoreMsg::Async(result));
        });
    }

    fn load_direct_messages(&self, contact_id: String) {
        let keys = match self.nostr_keys() {
            Ok(keys) => keys,
            Err(e) => {
                let _ = self
                    .tx
                    .send(CoreMsg::Async(AsyncMsg::Error(format!("{e:#}"))));
                return;
            }
        };
        let Some(contact) = self
            .state
            .nostr
            .contacts
            .iter()
            .find(|c| c.id == contact_id)
            .cloned()
        else {
            return;
        };
        let tx = self.tx.clone();
        self.rt.spawn(async move {
            let result = async {
                let peer = public_key_from_npub_or_hex(&contact.npub)?;
                let client = nostr_client().await?;
                let filter = Filter::new()
                    .authors([keys.public_key(), peer])
                    .pubkeys([keys.public_key(), peer])
                    .kind(Kind::EncryptedDirectMessage)
                    .limit(100);
                let events = client
                    .fetch_events(filter)
                    .timeout(Duration::from_secs(10))
                    .await?;
                let mut messages = Vec::new();
                for event in events.into_iter() {
                    let counterparty = if event.pubkey == keys.public_key() {
                        peer
                    } else {
                        event.pubkey
                    };
                    let Ok(body) = nip04::decrypt(keys.secret_key(), &counterparty, &event.content)
                    else {
                        continue;
                    };
                    messages.push(NostrMessage {
                        id: event.id.to_hex(),
                        contact_id: contact.id.clone(),
                        body,
                        inbound: event.pubkey != keys.public_key(),
                        timestamp: event.created_at.to_human_datetime(),
                    });
                }
                messages.sort_by(|a, b| a.timestamp.cmp(&b.timestamp));
                Ok::<_, anyhow::Error>(AsyncMsg::DirectMessagesLoaded(messages))
            }
            .await
            .unwrap_or_else(|e| AsyncMsg::Error(format!("Nostr DM refresh failed: {e:#}")));
            let _ = tx.send(CoreMsg::Async(result));
        });
    }

    fn send_direct_message(&self, contact_id: String, message: String) {
        let message = message.trim().to_string();
        if message.is_empty() {
            return;
        }
        let keys = match self.nostr_keys() {
            Ok(keys) => keys,
            Err(e) => {
                let _ = self
                    .tx
                    .send(CoreMsg::Async(AsyncMsg::Error(format!("{e:#}"))));
                return;
            }
        };
        let Some(contact) = self
            .state
            .nostr
            .contacts
            .iter()
            .find(|c| c.id == contact_id)
            .cloned()
        else {
            return;
        };
        let tx = self.tx.clone();
        self.rt.spawn(async move {
            let result = async {
                let peer = public_key_from_npub_or_hex(&contact.npub)?;
                let encrypted = nip04::encrypt(keys.secret_key(), &peer, &message)?;
                let tag = Tag::parse(["p".to_string(), peer.to_hex()])?;
                let event = EventBuilder::new(Kind::EncryptedDirectMessage, encrypted)
                    .tag(tag)
                    .finalize(&keys)?;
                let client = nostr_client().await?;
                client.send_event(&event).await?;
                Ok::<_, anyhow::Error>(AsyncMsg::DirectMessageSent(NostrMessage {
                    id: event.id.to_hex(),
                    contact_id: contact.id,
                    body: message,
                    inbound: false,
                    timestamp: event.created_at.to_human_datetime(),
                }))
            }
            .await
            .unwrap_or_else(|e| AsyncMsg::Error(format!("Nostr DM send failed: {e:#}")));
            let _ = tx.send(CoreMsg::Async(result));
        });
    }
}

#[cfg(test)]
mod tests {
    use crate::activity::{
        best_zap_receipt_for_activity, zap_receipt_activity_assignments, zap_receipt_match_score,
    };
    use crate::core::custom_address_flow::lightning_address_matches_name;
    use crate::persistence::ServerConfig;
    use crate::profile_cache::{load_profile, save_profile, ProfileCacheEntry};
    use crate::wallet::{open_bark_wallet, WalletOpenMode};
    use crate::zaps::fetch_received_zap_receipts;
    use crate::{ActivityIconKind, ActivityItem, WalletNetwork};
    use nostr_sdk::prelude::{
        Alphabet, FromBech32, Keys, SecretKey as NostrSecretKey, SingleLetterTag,
    };

    use super::*;

    #[test]
    fn derives_nostr_key_from_wallet_seed_path() {
        let keys = derive_nostr_keys_from_mnemonic(
            "leader monkey parrot ring guide accident before fence cannon height naive bean",
        )
        .unwrap();

        assert_eq!(
            keys.secret_key().as_secret_bytes(),
            NostrSecretKey::parse(
                "7f7ff03d123792d6ac594bfa67bf6d0c0ab55b6b1fdb6249303fe861f1ccba9a",
            )
            .unwrap()
            .as_secret_bytes(),
        );
    }

    #[test]
    fn detects_send_screen_removed_from_route_stack() {
        assert!(send_screen_removed(&[Screen::Send], &[]));
        assert!(send_screen_removed(
            &[
                Screen::ContactDetail {
                    contact_id: "alice".to_string()
                },
                Screen::Send,
            ],
            &[Screen::ContactDetail {
                contact_id: "alice".to_string()
            }]
        ));
        assert!(!send_screen_removed(&[], &[Screen::Send]));
        assert!(!send_screen_removed(&[Screen::Send], &[Screen::Send]));
        assert!(!send_screen_removed(&[Screen::Receive], &[]));
    }

    #[test]
    fn only_unconfirmed_rounds_are_reported_as_committed() {
        use bitcoin::hashes::Hash as _;

        let pending_id = RoundStateId(1);
        let committed_id = RoundStateId(2);
        let statuses = HashMap::from([
            (pending_id, RoundStatus::Pending),
            (
                committed_id,
                RoundStatus::Unconfirmed {
                    funding_txid: bitcoin::Txid::all_zeros(),
                },
            ),
        ]);

        assert_eq!(
            committed_round_balance([(pending_id, 21_000), (committed_id, 34_000)], &statuses),
            Some(34_000)
        );
    }

    #[test]
    fn unresolved_round_status_preserves_last_committed_balance() {
        let pending_id = RoundStateId(1);

        assert_eq!(
            committed_round_balance([(pending_id, 21_000)], &HashMap::new()),
            None
        );
    }

    #[test]
    fn pending_custom_lightning_address_must_match_name() {
        assert!(lightning_address_matches_name(
            "alice@signet.arkzap.me",
            "alice"
        ));
        assert!(!lightning_address_matches_name(
            "alice@signet.arkzap.me",
            "bob"
        ));
        assert!(!lightning_address_matches_name("alice", "alice"));
    }

    #[test]
    fn loading_active_custom_address_ignores_stale_draft_name() {
        let data_dir = tempfile::tempdir().expect("temp data dir");
        let cache_dir = tempfile::tempdir().expect("temp cache dir");
        let (tx, _rx) = flume::unbounded();
        let mut core = AppCore::new(
            data_dir.path().to_path_buf(),
            cache_dir.path().to_path_buf(),
            Arc::new(TestSecretStore),
            tx,
            Runtime::new().expect("tokio runtime"),
        );
        core.state.lightning_address.custom_address =
            Some("benthecarman@signet.arkzap.me".to_string());
        core.state.lightning_address.custom_name = "benthecarman".to_string();
        core.save_app_data();

        let raw = std::fs::read_to_string(&core.app_data_path).expect("app data");
        let mut json: serde_json::Value = serde_json::from_str(&raw).expect("json");
        json["custom_lightning_address_name"] = serde_json::Value::String("carman".to_string());
        std::fs::write(
            &core.app_data_path,
            serde_json::to_string_pretty(&json).expect("json"),
        )
        .expect("write app data");

        let (tx, _rx) = flume::unbounded();
        let mut restored = AppCore::new(
            data_dir.path().to_path_buf(),
            cache_dir.path().to_path_buf(),
            Arc::new(TestSecretStore),
            tx,
            Runtime::new().expect("tokio runtime"),
        );
        restored.load_app_data();

        assert_eq!(
            restored.state.lightning_address.custom_address.as_deref(),
            Some("benthecarman@signet.arkzap.me")
        );
        assert_eq!(restored.state.lightning_address.custom_name, "benthecarman");
        assert_eq!(
            restored.state.lightning_address.registration_phase,
            LightningAddressRegistrationPhase::Active
        );
        assert!(restored
            .state
            .lightning_address
            .registration_invoice
            .is_none());
    }

    #[test]
    fn canceling_custom_address_payment_discards_attempt_and_restores_active_name() {
        let data_dir = tempfile::tempdir().expect("temp data dir");
        let cache_dir = tempfile::tempdir().expect("temp cache dir");
        let (tx, _rx) = flume::unbounded();
        let mut core = AppCore::new(
            data_dir.path().to_path_buf(),
            cache_dir.path().to_path_buf(),
            Arc::new(TestSecretStore),
            tx,
            Runtime::new().expect("tokio runtime"),
        );
        core.state.lightning_address.custom_address =
            Some("benthecarman@signet.arkzap.me".to_string());
        core.state.lightning_address.custom_name = "carman".to_string();
        core.state.lightning_address.registration_address =
            Some("carman@signet.arkzap.me".to_string());
        core.state.lightning_address.backing_ark_address = Some("tark1example".to_string());
        core.state.lightning_address.registration_invoice = Some("lnbc1invoice".to_string());
        core.state.lightning_address.registration_purchase_id = Some("42".to_string());
        core.state.lightning_address.registration_amount_sat = 1_000;
        core.state
            .lightning_address
            .registration_requires_confirmation = true;
        core.state.lightning_address.registration_phase =
            LightningAddressRegistrationPhase::AwaitingPayment;

        core.cancel_lightning_address_registration_payment();

        assert_eq!(core.state.lightning_address.custom_name, "benthecarman");
        assert_eq!(
            core.state.lightning_address.registration_phase,
            LightningAddressRegistrationPhase::Active
        );
        assert!(core.state.lightning_address.registration_invoice.is_none());
        assert!(core.pending_custom_lightning_address().is_none());

        let raw = std::fs::read_to_string(&core.app_data_path).expect("app data");
        let json: serde_json::Value = serde_json::from_str(&raw).expect("json");
        assert_eq!(
            json["custom_lightning_address_name"].as_str(),
            Some("benthecarman")
        );
        assert!(json["pending_custom_lightning_address"].is_null());
    }

    #[test]
    fn custom_address_registration_payment_address_does_not_replace_backing_address() {
        let data_dir = tempfile::tempdir().expect("temp data dir");
        let cache_dir = tempfile::tempdir().expect("temp cache dir");
        let (tx, _rx) = flume::unbounded();
        let mut core = AppCore::new(
            data_dir.path().to_path_buf(),
            cache_dir.path().to_path_buf(),
            Arc::new(TestSecretStore),
            tx,
            Runtime::new().expect("tokio runtime"),
        );
        core.state.lightning_address.backing_ark_address = Some("tark1backing".to_string());

        core.apply_lightning_address_registration_update(
            "alice".to_string(),
            "alice@signet.arkzap.me".to_string(),
            "tark1payment".to_string(),
            Some("lnbc1invoice".to_string()),
            Some("42".to_string()),
            Some(1_000_000),
            false,
            false,
            false,
            true,
            None,
        );

        assert_eq!(
            core.state.lightning_address.backing_ark_address.as_deref(),
            Some("tark1backing")
        );
        assert_eq!(
            core.state
                .lightning_address
                .registration_payment_ark_address
                .as_deref(),
            Some("tark1payment")
        );
        let pending = core
            .pending_custom_lightning_address()
            .expect("pending registration");
        assert_eq!(pending.ark_address, "tark1backing");
        assert_eq!(pending.payment_ark_address.as_deref(), Some("tark1payment"));
    }

    #[test]
    fn import_nostr_secret_error_toast_does_not_echo_input() {
        let data_dir = tempfile::tempdir().expect("temp data dir");
        let cache_dir = tempfile::tempdir().expect("temp cache dir");
        let (tx, _rx) = flume::unbounded();
        let mut core = AppCore::new(
            data_dir.path().to_path_buf(),
            cache_dir.path().to_path_buf(),
            Arc::new(TestSecretStore),
            tx,
            Runtime::new().expect("tokio runtime"),
        );
        let submitted = "nsec1thisshouldnotshowupinatoast";

        core.import_nostr_secret(submitted.to_string());

        let toast = core.state.toast.as_deref().expect("toast");
        assert_eq!(toast, "Invalid Nostr secret key.");
        assert!(!toast.contains(submitted));
    }

    #[test]
    fn selecting_nostr_contact_makes_zap_available_immediately() {
        let data_dir = tempfile::tempdir().expect("temp data dir");
        let cache_dir = tempfile::tempdir().expect("temp cache dir");
        let (tx, _rx) = flume::unbounded();
        let mut core = AppCore::new(
            data_dir.path().to_path_buf(),
            cache_dir.path().to_path_buf(),
            Arc::new(TestSecretStore),
            tx,
            Runtime::new().expect("tokio runtime"),
        );
        let npub = Keys::generate()
            .public_key()
            .to_bech32()
            .expect("generated npub");
        core.state.nostr.contacts.push(Contact {
            id: "alice".to_string(),
            npub,
            name: "Alice".to_string(),
            followed: true,
            picture: String::new(),
            lightning_address: "alice@example.com".to_string(),
            lnurl: String::new(),
            last_used: 0,
        });

        core.handle(CoreMsg::Action(AppAction::SelectSendContact {
            contact_id: "alice".to_string(),
        }));

        assert_eq!(
            core.state.send.selected_contact_id.as_deref(),
            Some("alice")
        );
        assert_eq!(core.state.send.destination, "alice@example.com");
        assert!(core.state.send.zap_available);
    }

    #[test]
    fn matches_lightning_address_zap_receipt_by_destination_amount() {
        let receipt = ZapReceiptRecord {
            event_id: "zap-1".to_string(),
            sender_pubkey: "sender".to_string(),
            recipient_pubkey: "recipient".to_string(),
            invoice: None,
            payment_hash: None,
            amount_msat: Some(21_000),
            lnurl: Some("lnurl1test".to_string()),
            comment: None,
            created_at: 1,
        };
        let item = test_activity_item("Lightning address", 20, 21);

        assert!(best_zap_receipt_for_activity(&[receipt], &item).is_some());
    }

    #[test]
    fn does_not_match_stale_zap_receipt_to_lightning_address_activity_by_amount() {
        let receipt = ZapReceiptRecord {
            event_id: "zap-stale".to_string(),
            sender_pubkey: "sender".to_string(),
            recipient_pubkey: "recipient".to_string(),
            invoice: None,
            payment_hash: None,
            amount_msat: Some(1_000_000),
            lnurl: Some("lnurl1test".to_string()),
            comment: None,
            created_at: 1_781_055_500,
        };
        let mut item = test_activity_item("Lightning address", 1_000, 1_000);
        item.completed_at_unix = 1_781_055_500 + 61;

        assert!(best_zap_receipt_for_activity(&[receipt], &item).is_none());
    }

    #[test]
    fn does_not_match_non_lightning_address_activity_by_amount_only() {
        let receipt = ZapReceiptRecord {
            event_id: "zap-1".to_string(),
            sender_pubkey: "sender".to_string(),
            recipient_pubkey: "recipient".to_string(),
            invoice: None,
            payment_hash: None,
            amount_msat: Some(21_000),
            lnurl: Some("lnurl1test".to_string()),
            comment: None,
            created_at: 1,
        };
        let item = test_activity_item("Ark", 21, 21);

        assert!(best_zap_receipt_for_activity(&[receipt], &item).is_none());
    }

    #[test]
    fn does_not_match_ark_activity_by_amount_even_when_time_is_close() {
        let receipt = ZapReceiptRecord {
            event_id: "zap-1".to_string(),
            sender_pubkey: "sender".to_string(),
            recipient_pubkey: "recipient".to_string(),
            invoice: None,
            payment_hash: None,
            amount_msat: Some(1_000_000),
            lnurl: Some("lnurl1test".to_string()),
            comment: None,
            created_at: 1_781_055_500,
        };
        let mut item = test_activity_item("Ark", 1_000, 1_000);
        item.completed_at_unix = 1_781_056_000;

        assert!(best_zap_receipt_for_activity(&[receipt], &item).is_none());
    }

    #[test]
    fn picks_exact_payment_hash_before_amount_fallback() {
        let older = ZapReceiptRecord {
            event_id: "zap-older".to_string(),
            sender_pubkey: "wrong-sender".to_string(),
            recipient_pubkey: "recipient".to_string(),
            invoice: None,
            payment_hash: None,
            amount_msat: Some(1_000_000),
            lnurl: Some("lnurl1test".to_string()),
            comment: None,
            created_at: 1_781_055_100,
        };
        let closer = ZapReceiptRecord {
            event_id: "zap-closer".to_string(),
            sender_pubkey: "right-sender".to_string(),
            recipient_pubkey: "recipient".to_string(),
            invoice: None,
            payment_hash: Some("payment-hash".to_string()),
            amount_msat: Some(1_000_000),
            lnurl: Some("lnurl1test".to_string()),
            comment: None,
            created_at: 1_781_055_980,
        };
        let mut item = test_activity_item("Lightning address", 1_000, 1_000);
        item.completed_at_unix = 1_781_055_100;
        item.lightning_payment_hash = Some("payment-hash".to_string());
        let receipts = vec![older, closer];

        let receipt = best_zap_receipt_for_activity(&receipts, &item).unwrap();

        assert_eq!(receipt.sender_pubkey, "right-sender");
    }

    #[test]
    fn prefers_lnurl_zap_receipt_for_lightning_address_amount_fallback() {
        let wrong = ZapReceiptRecord {
            event_id: "zap-wrong".to_string(),
            sender_pubkey: "wrong-sender".to_string(),
            recipient_pubkey: "recipient".to_string(),
            invoice: None,
            payment_hash: None,
            amount_msat: Some(1_000_000),
            lnurl: None,
            comment: None,
            created_at: 1_781_055_500,
        };
        let expected = ZapReceiptRecord {
            event_id: "zap-expected".to_string(),
            sender_pubkey: "expected-sender".to_string(),
            recipient_pubkey: "recipient".to_string(),
            invoice: None,
            payment_hash: None,
            amount_msat: Some(1_000_000),
            lnurl: Some("lnurl1test".to_string()),
            comment: None,
            created_at: 1_781_055_490,
        };
        let mut item = test_activity_item("Lightning address", 1_000, 1_000);
        item.completed_at_unix = 1_781_055_500;
        let receipts = vec![wrong, expected];

        let receipt = best_zap_receipt_for_activity(&receipts, &item).unwrap();

        assert_eq!(receipt.sender_pubkey, "expected-sender");
    }

    #[test]
    fn assigns_each_zap_receipt_to_only_one_activity() {
        let receipt = ZapReceiptRecord {
            event_id: "zap-1".to_string(),
            sender_pubkey: "sender".to_string(),
            recipient_pubkey: "recipient".to_string(),
            invoice: None,
            payment_hash: Some("payment-hash".to_string()),
            amount_msat: Some(1_000_000),
            lnurl: Some("lnurl1test".to_string()),
            comment: None,
            created_at: 1_781_055_500,
        };
        let mut first = test_activity_item("Ark", 1_000, 1_000);
        first.id = "activity-1".to_string();
        first.lightning_payment_hash = Some("payment-hash".to_string());
        first.completed_at_unix = 1_781_055_500;
        let mut second = test_activity_item("Ark", 1_000, 1_000);
        second.id = "activity-2".to_string();
        second.lightning_payment_hash = Some("payment-hash".to_string());
        second.completed_at_unix = 1_781_055_510;
        let activity = vec![first, second];

        let assignments = zap_receipt_activity_assignments(&[receipt], &activity);

        assert_eq!(assignments.len(), 1);
        assert_eq!(assignments[0].1, 0);
    }

    #[test]
    fn assigns_each_activity_to_only_one_zap_receipt() {
        let older = ZapReceiptRecord {
            event_id: "zap-older".to_string(),
            sender_pubkey: "older-sender".to_string(),
            recipient_pubkey: "recipient".to_string(),
            invoice: None,
            payment_hash: Some("payment-hash".to_string()),
            amount_msat: Some(1_000_000),
            lnurl: Some("lnurl1test".to_string()),
            comment: None,
            created_at: 1_781_055_100,
        };
        let closer = ZapReceiptRecord {
            event_id: "zap-closer".to_string(),
            sender_pubkey: "closer-sender".to_string(),
            recipient_pubkey: "recipient".to_string(),
            invoice: None,
            payment_hash: Some("payment-hash".to_string()),
            amount_msat: Some(1_000_000),
            lnurl: Some("lnurl1test".to_string()),
            comment: None,
            created_at: 1_781_055_490,
        };
        let mut item = test_activity_item("Ark", 1_000, 1_000);
        item.lightning_payment_hash = Some("payment-hash".to_string());
        item.completed_at_unix = 1_781_055_500;
        let receipts = vec![older, closer];
        let activity = vec![item];

        let assignments = zap_receipt_activity_assignments(&receipts, &activity);

        assert_eq!(assignments, vec![(0, 0)]);
    }

    #[test]
    fn assigns_one_lnurl_amount_fallback_when_one_receipt_matches_multiple_activities() {
        let receipt = ZapReceiptRecord {
            event_id: "zap-1".to_string(),
            sender_pubkey: "sender".to_string(),
            recipient_pubkey: "recipient".to_string(),
            invoice: None,
            payment_hash: None,
            amount_msat: Some(1_000_000),
            lnurl: Some("lnurl1test".to_string()),
            comment: None,
            created_at: 1_781_055_500,
        };
        let mut first = test_activity_item("Lightning address", 1_000, 1_000);
        first.id = "activity-1".to_string();
        first.completed_at_unix = 1_781_055_500;
        let mut second = test_activity_item("Lightning address", 1_000, 1_000);
        second.id = "activity-2".to_string();
        second.completed_at_unix = 1_781_055_510;

        let assignments = zap_receipt_activity_assignments(&[receipt], &[first, second]);

        assert_eq!(assignments, vec![(0, 0)]);
    }

    #[test]
    fn assigns_one_lnurl_amount_fallback_when_one_activity_matches_multiple_receipts() {
        let older = ZapReceiptRecord {
            event_id: "zap-older".to_string(),
            sender_pubkey: "older-sender".to_string(),
            recipient_pubkey: "recipient".to_string(),
            invoice: None,
            payment_hash: None,
            amount_msat: Some(1_000_000),
            lnurl: Some("lnurl1test".to_string()),
            comment: None,
            created_at: 1_781_055_460,
        };
        let newer = ZapReceiptRecord {
            event_id: "zap-newer".to_string(),
            sender_pubkey: "newer-sender".to_string(),
            recipient_pubkey: "recipient".to_string(),
            invoice: None,
            payment_hash: None,
            amount_msat: Some(1_000_000),
            lnurl: Some("lnurl1test".to_string()),
            comment: None,
            created_at: 1_781_055_500,
        };
        let mut item = test_activity_item("Lightning address", 1_000, 1_000);
        item.completed_at_unix = 1_781_055_500;

        let assignments = zap_receipt_activity_assignments(&[older, newer], &[item]);

        assert_eq!(assignments, vec![(0, 1)]);
    }

    #[test]
    fn local_own_profile_picture_edit_seeds_profile_cache_row() {
        let cache_dir = tempfile::tempdir().expect("temp cache dir");
        let conn = open_profile_cache(cache_dir.path()).expect("profile cache");
        let pubkey_hex = "79ff3bfdd4e403159b9b0cba2cc9745eaa514637e1d4ec2ae166b743341be1af";
        let picture = "https://example.com/new-picture.jpg";
        let nostr = crate::NostrState {
            npub: Some(pubkey_hex.to_string()),
            name: "Rebel".to_string(),
            about: String::new(),
            picture: picture.to_string(),
            picture_display_url: picture.to_string(),
            lud16: String::new(),
            nip05: String::new(),
            deleted: false,
            contacts: Vec::new(),
        };

        save_own_profile_picture_remote_url(Some(&conn), pubkey_hex, &nostr);
        update_cached_picture(&conn, pubkey_hex, picture).expect("mark picture cached");

        let entry = load_profile(&conn, pubkey_hex)
            .expect("load profile")
            .expect("profile row");
        assert_eq!(entry.picture_remote_url, picture);
        assert_eq!(entry.picture_cached_url, picture);
    }

    #[test]
    fn local_own_profile_picture_edit_clears_stale_cached_url_when_remote_changes() {
        let cache_dir = tempfile::tempdir().expect("temp cache dir");
        let conn = open_profile_cache(cache_dir.path()).expect("profile cache");
        let pubkey_hex = "79ff3bfdd4e403159b9b0cba2cc9745eaa514637e1d4ec2ae166b743341be1af";
        save_profile(
            &conn,
            &ProfileCacheEntry {
                pubkey: pubkey_hex.to_string(),
                metadata_json: "{}".to_string(),
                name: "Rebel".to_string(),
                picture_remote_url: "https://example.com/old-picture.jpg".to_string(),
                picture_cached_url: "https://example.com/old-picture.jpg".to_string(),
                picture_cached_at: 42,
                lightning_address: String::new(),
                lnurl: String::new(),
                event_created_at: 7,
            },
        )
        .expect("seed profile row");
        let new_picture = "https://example.com/new-picture.jpg";
        let nostr = crate::NostrState {
            npub: Some(pubkey_hex.to_string()),
            name: "Rebel".to_string(),
            about: String::new(),
            picture: new_picture.to_string(),
            picture_display_url: new_picture.to_string(),
            lud16: String::new(),
            nip05: String::new(),
            deleted: false,
            contacts: Vec::new(),
        };

        save_own_profile_picture_remote_url(Some(&conn), pubkey_hex, &nostr);

        let entry = load_profile(&conn, pubkey_hex)
            .expect("load profile")
            .expect("profile row");
        assert_eq!(entry.picture_remote_url, new_picture);
        assert_eq!(entry.picture_cached_url, "");
        assert_eq!(entry.picture_cached_at, 0);
        assert_eq!(entry.event_created_at, 7);
    }

    #[tokio::test]
    #[ignore]
    async fn e2e_matches_real_wallet_zap_receipts_to_activity() {
        macro_rules! e2e_log {
            ($($arg:tt)*) => {
                if std::env::var_os("REBEL_WALLET_E2E_LOG").is_some() {
                    println!($($arg)*);
                }
            };
        }

        let expected_sender = NostrPublicKey::from_bech32(
            "nprofile1qqs8r0afe0uyzyx7v9lftyppkzxxj5j0e2ssx0laqc4t6zhzv4a6ynqjgyx99",
        )
        .expect("expected sender nprofile")
        .to_hex();
        let wrong_sender = NostrPublicKey::from_bech32(
            "npub1p4kg8zxukpym3h20erfa3samj00rm2gt4q5wfuyu3tg0x3jg3gesvncxf8",
        )
        .expect("wrong sender npub")
        .to_hex();
        e2e_log!("expected_sender={expected_sender}");
        e2e_log!("wrong_sender={wrong_sender}");
        let mnemonic = std::env::var("REBEL_WALLET_E2E_MNEMONIC")
            .expect("set REBEL_WALLET_E2E_MNEMONIC for this ignored test");
        let mnemonic = Mnemonic::from_str(&mnemonic).expect("valid mnemonic");
        let data_dir = tempfile::tempdir().expect("temp data dir");
        let wallet = open_bark_wallet(
            data_dir.path().to_path_buf(),
            &mnemonic,
            WalletOpenMode::Restore,
            ServerConfig::for_network(WalletNetwork::Mainnet),
        )
        .await
        .expect("open wallet")
        .wallet;
        wallet.sync().await;

        let keys = derive_nostr_keys_from_mnemonic(&mnemonic.to_string()).expect("nostr keys");
        e2e_log!(
            "derived_npub={}",
            keys.public_key().to_bech32().expect("derived npub")
        );
        let mut receipts = fetch_received_zap_receipts(keys.public_key())
            .await
            .expect("fetch derived zap receipts");
        let reported_pubkey = std::env::var("REBEL_WALLET_E2E_NPUB")
            .ok()
            .and_then(|npub| public_key_from_npub_or_hex(&npub).ok())
            .unwrap_or_else(|| {
                public_key_from_npub_or_hex(
                    "npub1u8lnhlw5usp3t9vmpz60ejpyt649z33hu82wc2hpv6m5xdqmuxhs46turz",
                )
                .expect("reported npub")
            });
        if reported_pubkey != keys.public_key() {
            let reported_receipts = fetch_received_zap_receipts(reported_pubkey)
                .await
                .expect("fetch reported zap receipts");
            e2e_log!("reported_pubkey_receipts={}", reported_receipts.len());
            receipts.extend(reported_receipts);
        }
        let client = nostr_client().await.expect("nostr client");
        for relay in [
            "wss://nos.lol",
            "wss://relay.nostr.band",
            "wss://nostr.mom",
            "wss://relay.snort.social",
            "wss://purplepag.es",
            "wss://relay.benthecarman.com",
        ] {
            let _ = client.add_relay(relay).await;
        }
        client.connect().await;
        for (label, tag) in [
            ("raw lowercase p", SingleLetterTag::lowercase(Alphabet::P)),
            ("raw uppercase P", SingleLetterTag::uppercase(Alphabet::P)),
        ] {
            let events = client
                .fetch_events(
                    Filter::new()
                        .kind(Kind::ZapReceipt)
                        .custom_tag(tag, reported_pubkey.to_hex())
                        .limit(200),
                )
                .timeout(Duration::from_secs(10))
                .await
                .expect("raw zap fetch");
            e2e_log!("{label} events={}", events.len());
            for event in events
                .into_iter()
                .filter(|event| event.created_at.as_secs() > 1_780_000_000)
            {
                let parsed = crate::zaps::zap_receipt_from_event(&event, &reported_pubkey);
                e2e_log!(
                    "{label} recent id={} created_at={} parsed={}",
                    event.id,
                    event.created_at.as_secs(),
                    parsed.is_some()
                );
            }
        }
        let history = wallet.history().await.expect("wallet history");
        let backing_ark_address = history
            .iter()
            .filter(|movement| {
                crate::activity::is_user_visible_movement(movement)
                    && movement.effective_balance.to_sat() > 0
            })
            .find_map(|movement| {
                movement
                    .received_on
                    .first()
                    .map(|destination| destination.destination.value_string())
            });
        e2e_log!("backing_ark_address={backing_ark_address:?}");
        for movement in history.iter().filter(|movement| {
            crate::activity::is_user_visible_movement(movement)
                && movement.effective_balance.to_sat() > 0
        }) {
            let movement_hash = movement
                .lightning_payment_hash()
                .map(|hash| hash.to_string());
            e2e_log!(
                "movement id={} effective_sat={} completed_at={:?} updated_at={} movement_hash={:?} input_vtxos={} output_vtxos={}",
                movement.id,
                movement.effective_balance.to_sat(),
                movement.time.completed_at,
                movement.time.updated_at,
                movement_hash,
                movement.input_vtxos.len(),
                movement.output_vtxos.len()
            );
            for id in movement
                .output_vtxos
                .iter()
                .chain(movement.input_vtxos.iter())
            {
                let Ok(vtxo) = wallet.get_full_vtxo(*id).await else {
                    e2e_log!("  vtxo id={id} unavailable");
                    continue;
                };
                let policy_hash = match vtxo.policy() {
                    VtxoPolicy::ServerHtlcSend(policy) => {
                        Some(("server_htlc_send", policy.payment_hash.to_string()))
                    }
                    VtxoPolicy::ServerHtlcSend_v0(policy) => {
                        Some(("server_htlc_send_v0", policy.payment_hash.to_string()))
                    }
                    VtxoPolicy::ServerHtlcRecv(policy) => {
                        Some(("server_htlc_recv", policy.payment_hash.to_string()))
                    }
                    VtxoPolicy::ServerHtlcRecv_v0(policy) => {
                        Some(("server_htlc_recv_v0", policy.payment_hash.to_string()))
                    }
                    VtxoPolicy::Pubkey(_) => None,
                };
                let witness_hashes = vtxo
                    .transactions()
                    .flat_map(|item| item.tx.input)
                    .flat_map(|input| input.witness.to_vec())
                    .filter(|element| element.len() == 32)
                    .filter_map(|element| Preimage::from_slice(&element).ok())
                    .map(|preimage| preimage.compute_payment_hash().to_string())
                    .collect::<Vec<_>>();
                e2e_log!(
                    "  vtxo id={id} policy_hash={policy_hash:?} witness_hashes={witness_hashes:?}"
                );
            }
        }
        let mut state = AppState::initial();
        state.lightning_address.backing_ark_address = backing_ark_address;
        state.refresh_derived();
        let lightning_address = state.lightning_address;
        let synced = wallet_synced_msg(&wallet, &[], &lightning_address, &[], &receipts)
            .await
            .expect("synced activity");
        let mut activity = synced.activity;
        for item in activity
            .iter_mut()
            .filter(|item| item.amount_sat > 0 && item.payment_amount_sat.unsigned_abs() == 1_000)
        {
            item.method_display = "Lightning address".to_string();
        }

        e2e_log!("receipts={}", receipts.len());
        for receipt in receipts.iter().filter(|receipt| {
            receipt.created_at > 1_780_000_000
                || receipt
                    .amount_msat
                    .is_some_and(|amount| amount == 1_000_000 || amount == 1_000)
        }) {
            e2e_log!(
                "receipt event={} created_at={} amount_msat={:?} lnurl={} hash={:?} sender={}",
                receipt.event_id,
                receipt.created_at,
                receipt.amount_msat,
                receipt.lnurl.is_some(),
                receipt.payment_hash,
                receipt.sender_pubkey
            );
        }

        let assignments = zap_receipt_activity_assignments(&receipts, &activity);
        let mut matched = 0;
        for item in activity.iter().filter(|item| item.amount_sat > 0) {
            let receipt = assignments
                .iter()
                .find(|(activity_index, _)| activity[*activity_index].id == item.id)
                .map(|(_, receipt_index)| &receipts[*receipt_index]);
            if receipt.is_some() {
                matched += 1;
            }
            let mut candidates = receipts
                .iter()
                .filter_map(|receipt| Some((zap_receipt_match_score(receipt, item)?, receipt)))
                .collect::<Vec<_>>();
            candidates.sort_by_key(|(score, _)| *score);
            for (score, receipt) in candidates.iter().take(8) {
                e2e_log!(
                    "  candidate score={score:?} event={} created_at={} amount_msat={:?} lnurl={} sender={}",
                    receipt.event_id,
                    receipt.created_at,
                    receipt.amount_msat,
                    receipt.lnurl.is_some(),
                    receipt.sender_pubkey
                );
            }
            e2e_log!(
                "activity id={} completed_at_unix={} amount_sat={} payment_amount_sat={} method={} hash={:?} invoice_present={} matched_sender={:?}",
                item.id,
                item.completed_at_unix,
                item.amount_sat,
                item.payment_amount_sat,
                item.method_display,
                item.lightning_payment_hash,
                item.lightning_invoice.is_some(),
                receipt.map(|receipt| receipt.sender_pubkey.as_str())
            );
        }

        e2e_log!("matched_inbound_count={matched}");
        let expected_match = assignments.iter().any(|(activity_index, receipt_index)| {
            let item = &activity[*activity_index];
            item.amount_sat > 0
                && item.payment_amount_sat.unsigned_abs() == 1_000
                && receipts[*receipt_index].sender_pubkey == expected_sender
        });
        let wrong_match = assignments.iter().any(|(activity_index, receipt_index)| {
            let item = &activity[*activity_index];
            item.amount_sat > 0
                && item.payment_amount_sat.unsigned_abs() == 1_000
                && receipts[*receipt_index].sender_pubkey == wrong_sender
        });
        assert!(
            expected_match,
            "expected a 1000-sat activity to pair with the requested nprofile"
        );
        assert!(
            !wrong_match,
            "a 1000-sat activity still pairs with the known wrong npub"
        );
        assert!(!activity.is_empty(), "expected synced wallet activity");
    }

    fn test_activity_item(
        method_display: &str,
        amount_sat: i64,
        payment_amount_sat: i64,
    ) -> ActivityItem {
        ActivityItem {
            id: "activity-1".to_string(),
            title: String::new(),
            subtitle: String::new(),
            display_primary_name: "Unknown".to_string(),
            display_verb: "sent".to_string(),
            display_secondary_name: "you".to_string(),
            label: None,
            message_text: None,
            method_icon: "bolt.fill".to_string(),
            method_display: method_display.to_string(),
            amount_sat,
            payment_amount_sat,
            amount_display: String::new(),
            amount_fiat_display: None,
            signed_amount_display: String::new(),
            icon_kind: ActivityIconKind::Received,
            status: String::new(),
            timestamp: String::new(),
            completed_at_unix: 0,
            counterparty: None,
            ark_address: None,
            lightning_invoice: None,
            lightning_offer: None,
            lightning_payment_hash: None,
            lightning_payment_preimage: None,
        }
    }

    struct TestSecretStore;

    impl SecretStore for TestSecretStore {
        fn get_secret(&self, _key: String) -> Option<String> {
            None
        }

        fn set_secret(&self, _key: String, _value: String) -> bool {
            true
        }

        fn delete_secret(&self, _key: String) -> bool {
            true
        }
    }

    pub(super) fn test_core() -> (tempfile::TempDir, tempfile::TempDir, AppCore) {
        let data_dir = tempfile::tempdir().expect("temp data dir");
        let cache_dir = tempfile::tempdir().expect("temp cache dir");
        let (tx, _rx) = flume::unbounded();
        let core = AppCore::new(
            data_dir.path().to_path_buf(),
            cache_dir.path().to_path_buf(),
            Arc::new(TestSecretStore),
            tx,
            Runtime::new().expect("tokio runtime"),
        );
        (data_dir, cache_dir, core)
    }

    struct FailingSecretStore {
        set_calls: std::sync::atomic::AtomicUsize,
    }

    impl SecretStore for FailingSecretStore {
        fn get_secret(&self, _key: String) -> Option<String> {
            None
        }

        fn set_secret(&self, _key: String, _value: String) -> bool {
            self.set_calls
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            false
        }

        fn delete_secret(&self, _key: String) -> bool {
            false
        }
    }

    fn failing_secret_core() -> (Arc<FailingSecretStore>, AppCore) {
        let data_dir = tempfile::tempdir().expect("temp data dir");
        let cache_dir = tempfile::tempdir().expect("temp cache dir");
        let (tx, _rx) = flume::unbounded();
        let store = Arc::new(FailingSecretStore {
            set_calls: std::sync::atomic::AtomicUsize::new(0),
        });
        let core = AppCore::new(
            data_dir.path().to_path_buf(),
            cache_dir.path().to_path_buf(),
            store.clone(),
            tx,
            Runtime::new().expect("tokio runtime"),
        );
        (store, core)
    }

    #[test]
    fn save_wallet_seed_retries_once_then_gives_up() {
        let (store, core) = failing_secret_core();

        assert!(!core.save_wallet_seed("abandon abandon abandon"));
        assert_eq!(store.set_calls.load(std::sync::atomic::Ordering::SeqCst), 2);
    }

    #[test]
    fn persist_nostr_secret_warns_when_keychain_write_fails() {
        let (store, mut core) = failing_secret_core();

        assert!(!core.persist_nostr_secret("nsec1example"));
        assert_eq!(store.set_calls.load(std::sync::atomic::Ordering::SeqCst), 2);
        let toast = core.state.toast.as_deref().expect("warning toast");
        assert!(toast.contains("Keychain"));
        assert!(core
            .pending_haptics
            .contains(&HapticFeedback::NotificationWarning));
    }

    #[derive(Default)]
    pub(super) struct RecordingSecretStore {
        deleted_keys: std::sync::Mutex<Vec<String>>,
    }

    impl RecordingSecretStore {
        pub(super) fn deleted_keys(&self) -> Vec<String> {
            self.deleted_keys.lock().expect("deleted keys").clone()
        }
    }

    impl SecretStore for RecordingSecretStore {
        fn get_secret(&self, _key: String) -> Option<String> {
            None
        }

        fn set_secret(&self, _key: String, _value: String) -> bool {
            true
        }

        fn delete_secret(&self, key: String) -> bool {
            self.deleted_keys.lock().expect("deleted keys").push(key);
            true
        }
    }

    pub(super) fn recording_secret_core(
        data_dir: &std::path::Path,
    ) -> (Arc<RecordingSecretStore>, AppCore) {
        let cache_dir = tempfile::tempdir().expect("temp cache dir");
        let (tx, _rx) = flume::unbounded();
        let store = Arc::new(RecordingSecretStore::default());
        let core = AppCore::new(
            data_dir.to_path_buf(),
            cache_dir.path().to_path_buf(),
            store.clone(),
            tx,
            Runtime::new().expect("tokio runtime"),
        );
        (store, core)
    }

    #[test]
    fn delete_wallet_removes_secrets_after_successful_cleanup() {
        let data_dir = tempfile::tempdir().expect("temp data dir");
        let (store, mut core) = recording_secret_core(data_dir.path());

        core.delete_wallet();

        assert_eq!(
            store.deleted_keys(),
            vec![WALLET_SEED_KEY.to_string(), NOSTR_SECRET_KEY.to_string()]
        );
        assert_eq!(
            core.state.toast.as_deref(),
            Some("Wallet deleted. Start over to create or restore.")
        );
    }

    #[test]
    fn delete_wallet_keeps_secrets_when_database_removal_fails() {
        let data_dir = tempfile::tempdir().expect("temp data dir");
        // A directory at the database path cannot be removed with remove_file,
        // which forces the database cleanup to fail.
        std::fs::create_dir(data_dir.path().join(WalletNetwork::Signet.db_file_name()))
            .expect("blocking directory");
        let (store, mut core) = recording_secret_core(data_dir.path());

        core.delete_wallet();

        assert!(store.deleted_keys().is_empty());
        let toast = core.state.toast.as_deref().expect("cleanup warning toast");
        assert!(toast.contains("cleanup warnings"));
    }

    #[cfg(unix)]
    #[test]
    fn wallet_deletion_removes_secrets_when_empty_nwc_ledger_cannot_reopen() {
        use std::os::unix::fs::PermissionsExt;

        let data_dir = tempfile::tempdir().expect("temp data dir");
        let (store, mut core) = recording_secret_core(data_dir.path());
        core.nwc.clear_manager_for_test();
        crate::wallet::remove_wallet_database_files(
            &nwc_mobile::NwcApplicationManager::database_path(data_dir.path()),
        )
        .expect("remove initial ledger");
        std::fs::set_permissions(data_dir.path(), std::fs::Permissions::from_mode(0o500))
            .expect("make data directory read-only");

        core.delete_wallet();

        std::fs::set_permissions(data_dir.path(), std::fs::Permissions::from_mode(0o700))
            .expect("restore data directory permissions");
        assert_eq!(
            store.deleted_keys(),
            vec![WALLET_SEED_KEY.to_string(), NOSTR_SECRET_KEY.to_string()]
        );
        let toast = core.state.toast.as_deref().expect("cleanup warning toast");
        assert!(toast.contains("could not reopen NWC authorization storage"));
    }

    #[test]
    fn export_nostr_secret_reveals_in_state_not_toast() {
        struct NostrSecretStore;

        impl SecretStore for NostrSecretStore {
            fn get_secret(&self, key: String) -> Option<String> {
                (key == NOSTR_SECRET_KEY).then(|| "nsec1verysecret".to_string())
            }

            fn set_secret(&self, _key: String, _value: String) -> bool {
                true
            }

            fn delete_secret(&self, _key: String) -> bool {
                true
            }
        }

        let data_dir = tempfile::tempdir().expect("temp data dir");
        let cache_dir = tempfile::tempdir().expect("temp cache dir");
        let (tx, _rx) = flume::unbounded();
        let mut core = AppCore::new(
            data_dir.path().to_path_buf(),
            cache_dir.path().to_path_buf(),
            Arc::new(NostrSecretStore),
            tx,
            Runtime::new().expect("tokio runtime"),
        );

        core.export_nostr_secret();

        assert_eq!(
            core.state.revealed_nostr_secret.as_deref(),
            Some("nsec1verysecret")
        );
        let toast = core.state.toast.as_deref().unwrap_or("");
        assert!(!toast.contains("nsec1verysecret"));

        core.handle(CoreMsg::Action(AppAction::ClearRevealedNostrSecret));

        assert_eq!(core.state.revealed_nostr_secret, None);
    }

    #[test]
    fn clear_recovery_phrase_removes_seed_from_state() {
        let (_data_dir, _cache_dir, mut core) = test_core();
        core.state.recovery_phrase = Some("abandon abandon abandon".to_string());

        core.handle(CoreMsg::Action(AppAction::ClearRecoveryPhrase));

        assert_eq!(core.state.recovery_phrase, None);
    }

    #[test]
    fn pay_destination_ignored_while_payment_is_sending() {
        let (_data_dir, _cache_dir, mut core) = test_core();
        core.state.send.destination = "not a valid destination".to_string();
        core.state.busy.sending_payment = true;

        core.pay_destination();

        assert!(core.state.toast.is_none());
        assert_eq!(core.state.send.phase, SendPhase::Drafting);
    }

    #[test]
    fn pay_destination_ignored_while_send_phase_is_sending() {
        let (_data_dir, _cache_dir, mut core) = test_core();
        core.state.send.destination = "not a valid destination".to_string();
        core.state.send.phase = SendPhase::Sending;

        core.pay_destination();

        assert!(core.state.toast.is_none());
        assert_eq!(core.state.send.phase, SendPhase::Sending);
    }

    #[test]
    fn confirm_registration_payment_requires_awaiting_payment_phase() {
        let (_data_dir, _cache_dir, mut core) = test_core();
        core.state.lightning_address.registration_phase = LightningAddressRegistrationPhase::Idle;

        core.confirm_lightning_address_registration_payment();

        assert!(core.state.toast.is_none());
        assert_eq!(
            core.state.lightning_address.registration_phase,
            LightningAddressRegistrationPhase::Idle
        );
    }

    #[test]
    fn registration_update_with_matching_name_is_activated() {
        let (_data_dir, _cache_dir, mut core) = test_core();
        core.state.lightning_address.custom_name = "alice".to_string();

        core.apply_lightning_address_registration_update(
            "alice".to_string(),
            "alice@signet.arkzap.me".to_string(),
            "tark1example".to_string(),
            Some("lnbc1invoice".to_string()),
            Some("42".to_string()),
            Some(10_000_000),
            true,
            true,
            true,
            false,
            None,
        );

        assert_eq!(
            core.state.lightning_address.custom_address.as_deref(),
            Some("alice@signet.arkzap.me")
        );
        assert_eq!(
            core.state.lightning_address.registration_phase,
            LightningAddressRegistrationPhase::Active
        );
        assert!(core.state.lightning_address.registration_error.is_none());
    }

    #[test]
    fn registration_update_with_mismatched_name_is_not_activated() {
        let (_data_dir, _cache_dir, mut core) = test_core();
        core.state.lightning_address.custom_name = "alice".to_string();

        core.apply_lightning_address_registration_update(
            "mallory".to_string(),
            "mallory@signet.arkzap.me".to_string(),
            "tark1example".to_string(),
            Some("lnbc1invoice".to_string()),
            Some("42".to_string()),
            Some(10_000_000),
            true,
            true,
            true,
            false,
            None,
        );

        assert!(core.state.lightning_address.custom_address.is_none());
        assert_ne!(
            core.state.lightning_address.registration_phase,
            LightningAddressRegistrationPhase::Active
        );
        assert!(core.state.lightning_address.registration_error.is_some());
        assert_eq!(core.state.lightning_address.custom_name, "alice");
    }

    #[test]
    fn registration_update_with_mismatched_address_is_not_activated() {
        let (_data_dir, _cache_dir, mut core) = test_core();
        core.state.lightning_address.custom_name = "alice".to_string();

        core.apply_lightning_address_registration_update(
            "alice".to_string(),
            "mallory@signet.arkzap.me".to_string(),
            "tark1example".to_string(),
            Some("lnbc1invoice".to_string()),
            Some("42".to_string()),
            Some(10_000_000),
            true,
            true,
            true,
            false,
            None,
        );

        assert!(core.state.lightning_address.custom_address.is_none());
        assert_ne!(
            core.state.lightning_address.registration_phase,
            LightningAddressRegistrationPhase::Active
        );
        assert!(core.state.lightning_address.registration_error.is_some());
    }
}
