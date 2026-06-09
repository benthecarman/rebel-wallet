use std::collections::HashSet;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::{Arc, RwLock};
use std::thread;
use std::time::Duration;

use anyhow::Context;
use bark::ark::lightning::PaymentHash;
use bark::lightning_invoice::Bolt11Invoice;
use bark::Wallet;
use bip39::Mnemonic;
use bitcoin::{
    bip32::{DerivationPath, Xpriv},
    Amount,
};
use flume::Sender;
use nostr_sdk::prelude::{
    nip04, Contact as NostrContact, ContactListBuilder, EventBuilder, EventBuilderTemplate, Filter,
    FinalizeEvent, JsonUtil, Keys, Kind, Metadata, PublicKey as NostrPublicKey, Tag, ToBech32,
};
use tokio::runtime::Runtime;

use crate::activity::{activity_from_movement, is_user_visible_movement, truncate_middle};
use crate::nostr_support::{
    apply_metadata_content, contact_id, deleted_profile_content, mark_profile_deleted,
    merge_contacts, metadata_from_state, nostr_client, primal_follow_contacts,
    primal_profile_contacts, primal_search_profiles, public_key_from_npub_or_hex,
    upload_profile_picture, PrimalProfileContact,
};
use crate::payments::{
    is_lnurl_pay_destination, monitor_ark_receive, monitor_lightning_receive,
    parse_send_destination, resolve_lnurl_pay_invoice,
};
use crate::persistence::{PersistedAppData, PersistedPriceCurrency, ServerConfig};
use crate::price::fetch_bitcoin_price;
use crate::profile_cache::{
    clear_profile_cache, clear_profile_picture_dir, download_profile_picture,
    ensure_profile_picture_dir, load_profile, new_profile_picture_download_semaphore,
    open_profile_cache, profile_picture_file_url, save_profile, update_cached_picture,
    ProfileCacheEntry,
};
use crate::time::{now_label, now_unix};
use crate::updates::{AppUpdate, AsyncMsg, CoreMsg};
use crate::wallet::{open_bark_wallet, WalletOpenMode};
use crate::{
    AppAction, AppState, BusyState, CapabilityRequest, CapabilityRequestKind, Contact, MainTab,
    NostrMessage, PriceCurrency, ReceiveMethod, ReceivePhase, Screen, SecretStore, SendPhase,
    SetupState, WalletNetwork,
};

const WALLET_SEED_KEY: &str = "wallet_seed";
const NOSTR_SECRET_KEY: &str = "nostr_secret";
const NOSTR_DERIVATION_PATH: &str = "m/44'/1237'/0'/0/0";
const SEND_FEE_ESTIMATE_DEBOUNCE: Duration = Duration::from_millis(350);

fn profile_picture_download_key(pubkey: &str, remote_url: &str) -> String {
    format!("{pubkey}:{remote_url}")
}

fn msats_to_display_sats(msats: u64) -> String {
    if msats % 1_000 == 0 {
        (msats / 1_000).to_string()
    } else {
        format!("{:.3}", msats as f64 / 1_000.0)
    }
}

fn send_fee_estimate_request(destination: &str, amount_sat: u64) -> Option<(u64, bool)> {
    let destination = destination.trim();
    if destination.is_empty() {
        return None;
    }

    if is_lnurl_pay_destination(destination) {
        return (amount_sat > 0).then_some((amount_sat, true));
    }

    let lower = destination.to_lowercase();
    if lower.starts_with("lightning:") || lower.starts_with("ln") {
        if amount_sat > 0 {
            return Some((amount_sat, true));
        }

        let invoice = destination
            .strip_prefix("lightning:")
            .or_else(|| destination.strip_prefix("LIGHTNING:"))
            .unwrap_or(destination);
        let invoice = Bolt11Invoice::from_str(invoice).ok()?;
        let invoice_msat = invoice.amount_milli_satoshis()?;
        let invoice_sat = invoice_msat.checked_add(999)? / 1_000;
        return (invoice_sat > 0).then_some((invoice_sat, true));
    }

    (amount_sat > 0).then_some((amount_sat, false))
}

fn derive_nostr_keys_from_mnemonic(mnemonic: &str) -> anyhow::Result<Keys> {
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
    let secret_hex = child.private_key.display_secret().to_string();
    Keys::parse(&secret_hex).context("derived invalid Nostr key")
}

async fn wallet_synced_msg(
    wallet: &Wallet,
    contacts: &[Contact],
    lightning_address: &crate::LightningAddressState,
    maintenance_checked: bool,
) -> anyhow::Result<AsyncMsg> {
    let balance = wallet.balance().await.context("balance failed")?;
    let history = wallet.history().await.context("history failed")?;
    let activity = history
        .into_iter()
        .filter(is_user_visible_movement)
        .map(|movement| {
            activity_from_movement(
                movement,
                contacts,
                lightning_address.address.as_deref(),
                lightning_address.backing_ark_address.as_deref(),
            )
        })
        .collect();
    Ok(AsyncMsg::WalletSynced {
        balance_sat: balance.spendable.to_sat(),
        pending_receive_sat: balance.claimable_lightning_receive.to_sat(),
        pending_send_sat: balance.pending_lightning_send.to_sat(),
        pending_refresh_sat: balance.pending_in_round.to_sat(),
        maintenance_checked,
        activity,
    })
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
    profile_db: Option<rusqlite::Connection>,
    profile_picture_downloads: HashSet<String>,
    profile_picture_download_semaphore: Arc<tokio::sync::Semaphore>,
    profile_info_requests: HashSet<String>,
    rev: u64,
    next_capability_id: u64,
    send_fee_estimate_request_id: u64,
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
            profile_picture_downloads: HashSet::new(),
            profile_picture_download_semaphore: new_profile_picture_download_semaphore(),
            profile_info_requests: HashSet::new(),
            rev: 0,
            next_capability_id: 0,
            send_fee_estimate_request_id: 0,
        }
    }

    fn handle(&mut self, msg: CoreMsg) {
        match msg {
            CoreMsg::Action(action) => self.handle_action(action),
            CoreMsg::Async(msg) => self.handle_async(msg),
        }
        self.rev += 1;
        self.state.rev = self.rev;
        self.state.refresh_derived();
    }

    fn handle_action(&mut self, action: AppAction) {
        self.state.refresh_derived();
        match action {
            AppAction::Bootstrap => self.bootstrap(),
            AppAction::CreateWallet => {
                self.state.busy.opening_wallet = true;
                let mnemonic = Mnemonic::generate(12).expect("valid mnemonic").to_string();
                self.open_wallet(mnemonic, WalletOpenMode::Create);
            }
            AppAction::RestoreWallet { mnemonic } => {
                self.state.busy.opening_wallet = true;
                self.open_wallet(mnemonic.trim().to_string(), WalletOpenMode::Restore);
            }
            AppAction::ReplaceWallet { mnemonic } => {
                self.wallet = None;
                self.state.busy.opening_wallet = true;
                self.state.activity.clear();
                self.state.wallet.balance_sat = 0;
                self.state.wallet.pending_receive_sat = 0;
                self.state.wallet.pending_send_sat = 0;
                self.state.wallet.pending_refresh_sat = 0;
                self.open_wallet(mnemonic.trim().to_string(), WalletOpenMode::Replace);
            }
            AppAction::ShowSeed => {
                if let Some(seed) = self.secrets.get_secret(WALLET_SEED_KEY.to_string()) {
                    let _ = self.tx.send(CoreMsg::Async(AsyncMsg::Seed(seed)));
                } else {
                    self.state.toast = Some("Recovery phrase not found.".to_string());
                }
            }
            AppAction::SyncWallet => self.sync_wallet(),
            AppAction::MaintainVtxos => self.maintain_vtxos(),
            AppAction::RefreshPrice => self.refresh_price(),
            AppAction::SetPriceCurrency { currency } => self.set_price_currency(currency),
            AppAction::SelectNetwork { network } => self.select_network(network),
            AppAction::SelectTab { tab } => self.state.router.selected_tab = tab,
            AppAction::PushScreen { screen } => {
                if screen == Screen::Receive {
                    self.state.reset_receive_draft();
                }
                self.state.router.screen_stack.push(screen);
            }
            AppAction::PopScreen => {
                self.state.router.screen_stack.pop();
            }
            AppAction::UpdateScreenStack { stack } => self.state.router.screen_stack = stack,
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
            AppAction::BeginReceiveRequest => match self.state.receive.method {
                ReceiveMethod::Lightning => self.create_lightning_invoice(),
                ReceiveMethod::Ark => self.create_ark_address(),
            },
            AppAction::CreateArkAddress => self.create_ark_address(),
            AppAction::CreateLightningInvoice => self.create_lightning_invoice(),
            AppAction::SetSendSearchQuery { query } => {
                self.state.send.search_query = query;
                self.search_nostr_profiles();
            }
            AppAction::ContinueSendSearch => {
                let query = self.state.send.search_query.clone();
                self.set_send_destination(query);
            }
            AppAction::SelectSendContact { contact_id } => self.select_send_contact(contact_id),
            AppAction::PrefetchProfilePictures { contact_ids } => {
                self.prefetch_profile_pictures(contact_ids)
            }
            AppAction::SetSendDestination { destination } => self.set_send_destination(destination),
            AppAction::SetSendAmount { amount_sat } => {
                self.state.send.amount_sat = amount_sat;
                self.request_send_fee_estimate();
            }
            AppAction::SetSendMemo { memo } => self.state.send.memo = memo,
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
                    self.set_send_destination(value);
                    if self.state.router.screen_stack.last() != Some(&Screen::Send) {
                        self.state.router.screen_stack.push(Screen::Send);
                    }
                }
            }
            AppAction::CompleteClipboardRead { value } => {
                self.state.capability_request = None;
                if let Some(value) = value.filter(|v| !v.trim().is_empty()) {
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
                self.state.nostr.picture = picture;
                self.state.nostr.lud16 = lud16;
                self.state.nostr.nip05 = nip05;
                self.state.nostr.deleted = false;
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
        }
    }

    fn handle_async(&mut self, msg: AsyncMsg) {
        self.clear_busy_for_async(&msg);
        match msg {
            AsyncMsg::WalletReady { wallet, mnemonic } => {
                self.wallet = Some(wallet);
                self.state.setup = SetupState::Ready;
                self.state.router.default_screen = Screen::Home;
                self.state.router.selected_tab = MainTab::Home;
                self.state.router.screen_stack.clear();
                let _ = self
                    .secrets
                    .set_secret(WALLET_SEED_KEY.to_string(), mnemonic);
                self.ensure_wallet_derived_nostr_key();
                self.ensure_lightning_address();
                self.maintain_vtxos();
            }
            AsyncMsg::WalletSynced {
                balance_sat,
                pending_receive_sat,
                pending_send_sat,
                pending_refresh_sat,
                maintenance_checked: _,
                activity,
            } => {
                self.state.wallet.balance_sat = balance_sat;
                self.state.wallet.pending_receive_sat = pending_receive_sat;
                self.state.wallet.pending_send_sat = pending_send_sat;
                self.state.wallet.pending_refresh_sat = pending_refresh_sat;
                self.state.wallet.last_sync = Some(now_label());
                self.state.activity = activity;
            }
            AsyncMsg::ArkAddress(address) => {
                self.state.receive.ark_address = Some(address);
                self.state.receive.phase = ReceivePhase::ShowingRequest;
            }
            AsyncMsg::ArkReceiveConfirmed {
                address,
                amount_sat,
            } => {
                if self.state.receive.method == ReceiveMethod::Ark
                    && self.state.receive.phase == ReceivePhase::ShowingRequest
                    && self.state.receive.ark_address.as_deref() == Some(address.as_str())
                {
                    self.state.receive.amount_sat = amount_sat;
                    self.state.receive.phase = ReceivePhase::Success;
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
                }
                self.maintain_vtxos();
            }
            AsyncMsg::LightningAddressReady(ark_address) => {
                self.state.lightning_address.backing_ark_address = Some(ark_address.clone());
                self.save_lightning_address_ark_address(&ark_address);
                self.save_app_data();
            }
            AsyncMsg::SendFeeEstimateDue {
                request_id,
                destination,
                amount_sat,
                estimate_amount_sat,
                is_lightning,
            } => {
                if self.send_fee_estimate_is_current(request_id, &destination, amount_sat) {
                    self.perform_send_fee_estimate(
                        request_id,
                        destination,
                        amount_sat,
                        estimate_amount_sat,
                        is_lightning,
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
            AsyncMsg::Paid(result) => {
                self.state.send.phase = SendPhase::Success;
                self.state.send.success_amount_display = self.state.send.amount_display.clone();
                self.state.send.last_result = Some(result);
                self.maintain_vtxos();
            }
            AsyncMsg::Seed(seed) => {
                self.state.recovery_phrase = Some(seed);
            }
            AsyncMsg::NostrProfileLoaded(nostr) => {
                self.state.nostr.name = nostr.name;
                self.state.nostr.about = nostr.about;
                self.state.nostr.picture = nostr.picture;
                self.state.nostr.lud16 = nostr.lud16;
                self.state.nostr.nip05 = nostr.nip05;
                self.state.nostr.deleted = nostr.deleted;
                self.save_app_data();
            }
            AsyncMsg::NostrContactsLoaded(contacts) => {
                merge_contacts(&mut self.state.nostr.contacts, contacts);
                self.state.toast = Some("Nostr contacts refreshed from Primal.".to_string());
                self.save_app_data();
                self.sync_wallet();
            }
            AsyncMsg::PrimalContactsLoaded {
                records,
                show_toast,
            } => {
                let contacts = self.cache_primal_profile_contacts(records);
                merge_contacts(&mut self.state.nostr.contacts, contacts);
                if show_toast {
                    self.state.toast = Some("Nostr contacts refreshed from Primal.".to_string());
                }
                self.save_app_data();
                self.sync_wallet();
            }
            AsyncMsg::NostrSearchLoaded { query, contacts } => {
                if self.state.send.search_query.trim() == query {
                    self.state.send.global_search_results =
                        self.cache_primal_profile_contacts(contacts);
                }
            }
            AsyncMsg::PrimalProfilesLoaded { records } => {
                for record in &records {
                    self.profile_info_requests.remove(&record.pubkey_hex);
                }
                let contacts = self.cache_primal_profile_contacts(records);
                let contact_ids = contacts
                    .iter()
                    .map(|contact| contact.id.clone())
                    .collect::<Vec<_>>();
                merge_contacts(&mut self.state.nostr.contacts, contacts);
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
            }
            AsyncMsg::ProfilePictureCacheFailed { pubkey, remote_url } => {
                self.profile_picture_downloads
                    .remove(&profile_picture_download_key(&pubkey, &remote_url));
            }
            AsyncMsg::NostrProfilePictureUploaded(url) => {
                self.state.nostr.picture = url;
                self.state.toast = Some("Profile picture uploaded.".to_string());
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
                if matches!(self.state.setup, SetupState::NeedsSetup) {
                    self.state.setup = SetupState::Error {
                        message: message.clone(),
                    };
                }
                self.state.toast = Some(message);
            }
        }
    }

    fn clear_busy_for_async(&mut self, msg: &AsyncMsg) {
        match msg {
            AsyncMsg::WalletReady { .. } => {
                self.state.busy.bootstrapping = false;
                self.state.busy.opening_wallet = false;
            }
            AsyncMsg::WalletSynced {
                maintenance_checked,
                ..
            } => {
                if *maintenance_checked {
                    self.state.busy.syncing_wallet = false;
                    self.state.busy.maintaining_vtxos = false;
                } else if !self.state.busy.maintaining_vtxos {
                    self.state.busy.syncing_wallet = false;
                }
            }
            AsyncMsg::ArkAddress(_) | AsyncMsg::LightningInvoice { .. } => {
                self.state.busy.creating_invoice = false;
            }
            AsyncMsg::Paid(_) => self.state.busy.sending_payment = false,
            AsyncMsg::LightningAddressReady(_)
            | AsyncMsg::SendFeeEstimateDue { .. }
            | AsyncMsg::SendFeeEstimated { .. }
            | AsyncMsg::SendFeeEstimateFailed { .. } => {}
            AsyncMsg::NostrProfilePictureUploaded(_) => {
                self.state.busy.uploading_profile_picture = false;
            }
            AsyncMsg::NostrPublished(_) => self.state.busy.publishing_nostr = false,
            AsyncMsg::NostrProfileLoaded(_)
            | AsyncMsg::NostrContactsLoaded(_)
            | AsyncMsg::PrimalContactsLoaded { .. } => self.state.busy.refreshing_contacts = false,
            AsyncMsg::Error(_) => self.state.busy = BusyState::default(),
            AsyncMsg::ArkReceiveConfirmed { .. }
            | AsyncMsg::LightningReceiveStatus { .. }
            | AsyncMsg::LightningReceiveClaimed { .. }
            | AsyncMsg::Seed(_)
            | AsyncMsg::DirectMessagesLoaded(_)
            | AsyncMsg::DirectMessageSent(_)
            | AsyncMsg::NostrSearchLoaded { .. }
            | AsyncMsg::PrimalProfilesLoaded { .. }
            | AsyncMsg::PrimalProfilesFailed { .. }
            | AsyncMsg::ProfilePictureCached { .. }
            | AsyncMsg::ProfilePictureCacheFailed { .. }
            | AsyncMsg::PriceUpdated { .. }
            | AsyncMsg::PriceFailed => {}
        }
    }

    fn emit(&self, shared: &Arc<RwLock<AppState>>, tx: &Sender<AppUpdate>) {
        let mut snapshot = self.state.clone();
        snapshot.refresh_derived();
        match shared.write() {
            Ok(mut g) => *g = snapshot.clone(),
            Err(poison) => *poison.into_inner() = snapshot.clone(),
        }
        let _ = tx.send(AppUpdate::FullState(snapshot));
    }

    fn bootstrap(&mut self) {
        self.load_app_data();
        self.load_nostr_key();
        self.refresh_price();
        if let Some(mnemonic) = self.secrets.get_secret(WALLET_SEED_KEY.to_string()) {
            self.state.busy.bootstrapping = true;
            self.state.busy.opening_wallet = true;
            self.open_wallet(mnemonic, WalletOpenMode::OpenOrCreate);
        }
    }

    fn open_wallet(&self, mnemonic: String, mode: WalletOpenMode) {
        let tx = self.tx.clone();
        let data_dir = self.data_dir.clone();
        let server_config = ServerConfig::from_wallet(&self.state.wallet);
        self.rt.spawn(async move {
            let result = async {
                let mnemonic = Mnemonic::from_str(&mnemonic).context("invalid recovery phrase")?;
                let wallet = open_bark_wallet(data_dir, &mnemonic, mode, server_config).await?;
                Ok::<_, anyhow::Error>((wallet, mnemonic.to_string()))
            }
            .await;
            let msg = match result {
                Ok((wallet, mnemonic)) => AsyncMsg::WalletReady { wallet, mnemonic },
                Err(e) => AsyncMsg::Error(format!("Wallet setup failed: {e:#}")),
            };
            let _ = tx.send(CoreMsg::Async(msg));
        });
    }

    fn select_network(&mut self, network: WalletNetwork) {
        let server_config = ServerConfig::for_network(network.clone());
        let server_address = server_config.server_address;
        let esplora_address = server_config.esplora_address;

        let wallet_server_changed = self.state.wallet.server_address != server_address
            || self.state.wallet.esplora_address != esplora_address;
        let changed = self.state.wallet.network != network || wallet_server_changed;
        self.state.wallet.network = network;
        self.state.wallet.server_address = server_address;
        self.state.wallet.esplora_address = esplora_address;
        self.state.lightning_address.backing_ark_address = None;
        self.save_app_data();

        if wallet_server_changed {
            if let Some(seed) = self.secrets.get_secret(WALLET_SEED_KEY.to_string()) {
                self.wallet = None;
                self.state.busy.opening_wallet = true;
                self.open_wallet(seed, WalletOpenMode::OpenOrCreate);
                self.state.toast = Some("Network changed. Reconnecting wallet.".to_string());
            } else {
                self.state.toast = Some("Network changed.".to_string());
            }
        } else if changed {
            self.ensure_lightning_address();
            self.state.toast = Some("Network changed.".to_string());
        } else {
            self.state.toast = Some("Network already selected.".to_string());
        }
    }

    fn set_price_currency(&mut self, currency: PriceCurrency) {
        self.state.wallet.price_currency = currency;
        self.state.wallet.btc_price = None;
        self.save_app_data();
        self.refresh_price();
    }

    fn ensure_lightning_address(&mut self) {
        if let Some(address) = self.load_lightning_address_ark_address() {
            self.state.lightning_address.backing_ark_address = Some(address);
            return;
        }
        if self
            .state
            .lightning_address
            .backing_ark_address
            .as_ref()
            .is_some_and(|address| !address.trim().is_empty())
        {
            if let Some(address) = self.state.lightning_address.backing_ark_address.as_ref() {
                self.save_lightning_address_ark_address(address);
            }
            return;
        }
        let Some(wallet) = self.wallet.clone() else {
            return;
        };
        let tx = self.tx.clone();
        self.rt.spawn(async move {
            let msg = match wallet.new_address().await {
                Ok(address) => AsyncMsg::LightningAddressReady(address.to_string()),
                Err(e) => AsyncMsg::Error(format!("Could not create Arkzap address: {e:#}")),
            };
            let _ = tx.send(CoreMsg::Async(msg));
        });
    }

    fn load_lightning_address_ark_address(&self) -> Option<String> {
        load_wallet_metadata_value(
            &self.data_dir,
            self.state.wallet.network,
            "lightning_address_ark_address",
        )
    }

    fn save_lightning_address_ark_address(&self, address: &str) {
        let _ = save_wallet_metadata_value(
            &self.data_dir,
            self.state.wallet.network,
            "lightning_address_ark_address",
            address,
        );
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
        let Some(wallet) = self.wallet.clone() else {
            return;
        };
        self.state.busy.syncing_wallet = true;
        let tx = self.tx.clone();
        let contacts = self.state.nostr.contacts.clone();
        let lightning_address = self.state.lightning_address.clone();
        self.rt.spawn(async move {
            let result = async {
                wallet.sync().await;
                wallet_synced_msg(&wallet, &contacts, &lightning_address, false).await
            }
            .await
            .unwrap_or_else(|e| AsyncMsg::Error(format!("Sync failed: {e:#}")));
            let _ = tx.send(CoreMsg::Async(result));
        });
    }

    fn maintain_vtxos(&mut self) {
        let Some(wallet) = self.wallet.clone() else {
            return;
        };
        if self.state.busy.maintaining_vtxos || self.state.busy.sending_payment {
            return;
        }
        if self
            .state
            .router
            .screen_stack
            .iter()
            .any(|screen| matches!(screen, Screen::Send))
            && self.state.send.phase != SendPhase::Success
        {
            self.sync_wallet();
            return;
        }

        self.state.busy.syncing_wallet = true;
        self.state.busy.maintaining_vtxos = true;
        let tx = self.tx.clone();
        let contacts = self.state.nostr.contacts.clone();
        let lightning_address = self.state.lightning_address.clone();
        self.rt.spawn(async move {
            let result = async {
                wallet.sync().await;
                let _ = wallet.progress_pending_rounds(None).await;

                let pending_round_balance = wallet
                    .pending_round_balance()
                    .await
                    .context("pending round balance failed")?;
                if pending_round_balance == Amount::ZERO
                    && !wallet
                        .get_vtxos_to_refresh()
                        .await
                        .context("refresh candidate check failed")?
                        .is_empty()
                {
                    let _ = wallet
                        .maybe_schedule_maintenance_refresh_delegated()
                        .await
                        .context("delegated refresh scheduling failed")?;
                }

                wallet_synced_msg(&wallet, &contacts, &lightning_address, true).await
            }
            .await
            .unwrap_or_else(|e| AsyncMsg::Error(format!("VTXO refresh check failed: {e:#}")));
            let _ = tx.send(CoreMsg::Async(result));
        });
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

    fn create_lightning_invoice(&mut self) {
        let Some(wallet) = self.wallet.clone() else {
            return;
        };
        self.state.receive.phase = ReceivePhase::Creating;
        self.state.busy.creating_invoice = true;
        let amount_sat = self.state.receive.amount_sat;
        let memo = self.state.receive.memo.clone();
        let tx = self.tx.clone();
        self.rt.spawn(async move {
            match wallet
                .bolt11_invoice(Amount::from_sat(amount_sat), Some(memo))
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

    fn pay_destination(&mut self) {
        let destination = self.state.send.destination.trim().to_string();
        if destination.is_empty() {
            self.state.toast = Some("Enter a destination first.".to_string());
            return;
        }
        let total_cost_sat = self
            .state
            .send
            .total_cost_sat
            .unwrap_or(self.state.send.amount_sat);
        if total_cost_sat > self.state.wallet.balance_sat {
            self.state.toast = Some("Insufficient balance for this send.".to_string());
            return;
        }
        let lower = destination.to_lowercase();
        if is_lnurl_pay_destination(&destination) {
            self.pay_lnurl_destination(destination, self.state.send.amount_sat);
        } else if lower.starts_with("lightning:") || lower.starts_with("ln") {
            let invoice = destination
                .strip_prefix("lightning:")
                .or_else(|| destination.strip_prefix("LIGHTNING:"))
                .unwrap_or(&destination)
                .to_string();
            self.pay_lightning_invoice(invoice, Some(self.state.send.amount_sat));
        } else {
            self.pay_ark_address(destination, self.state.send.amount_sat);
        }
    }

    fn set_send_destination(&mut self, destination: String) {
        let raw = destination.trim().to_string();
        if raw.is_empty() {
            self.reset_send_draft();
            return;
        }

        let parsed = self
            .wallet
            .clone()
            .and_then(|wallet| self.rt.block_on(parse_send_destination(wallet, &raw)));
        if let Some(parsed) = parsed {
            self.state.send.destination = parsed.destination;
            if let Some(amount_sat) = parsed.amount_sat {
                self.state.send.amount_sat = amount_sat;
            }
            if let Some(memo) = parsed.memo.filter(|m| !m.trim().is_empty()) {
                self.state.send.memo = memo;
            }
            if let Some(toast) = parsed.toast {
                self.state.toast = Some(toast);
            }
        } else {
            self.state.send.destination = raw.clone();
        }
        self.state.send.search_query = raw;
        self.state.send.phase = SendPhase::Editing;
        self.request_send_fee_estimate();
    }

    fn select_send_contact(&mut self, contact_id: String) {
        if !self
            .state
            .nostr
            .contacts
            .iter()
            .any(|contact| contact.id == contact_id)
        {
            if let Some(contact) = self
                .state
                .send
                .search_results
                .iter()
                .find(|contact| contact.id == contact_id)
                .cloned()
            {
                self.state.nostr.contacts.push(contact);
                self.save_app_data();
            }
        }

        let Some(contact) = self
            .state
            .nostr
            .contacts
            .iter_mut()
            .find(|contact| contact.id == contact_id)
        else {
            self.state.toast = Some("Contact not found.".to_string());
            return;
        };

        let destination = if !contact.lightning_address.trim().is_empty() {
            contact.lightning_address.clone()
        } else {
            contact.lnurl.clone()
        };

        if destination.trim().is_empty() {
            self.state.toast = Some("This contact does not have a Lightning address.".to_string());
            return;
        }

        contact.last_used = now_unix();
        self.save_app_data();
        self.set_send_destination(destination);
    }

    fn reset_send_draft(&mut self) {
        self.state.send.destination.clear();
        self.state.send.search_query.clear();
        self.state.send.global_search_results.clear();
        self.state.send.amount_sat = 0;
        self.state.send.memo.clear();
        self.state.send.last_result = None;
        self.state.send.phase = SendPhase::Drafting;
        self.clear_send_fee_estimate();
    }

    fn request_send_fee_estimate(&mut self) {
        self.send_fee_estimate_request_id = self.send_fee_estimate_request_id.saturating_add(1);

        let destination = self.state.send.destination.trim().to_string();
        if destination.is_empty() {
            self.clear_send_fee_estimate();
            return;
        }

        let amount_sat = self.state.send.amount_sat;
        let Some((estimate_amount_sat, is_lightning)) =
            send_fee_estimate_request(&destination, amount_sat)
        else {
            self.clear_send_fee_estimate();
            return;
        };

        self.state.send.estimating_fee = true;
        let tx = self.tx.clone();
        let request_id = self.send_fee_estimate_request_id;
        self.rt.spawn(async move {
            tokio::time::sleep(SEND_FEE_ESTIMATE_DEBOUNCE).await;
            let _ = tx.send(CoreMsg::Async(AsyncMsg::SendFeeEstimateDue {
                request_id,
                destination,
                amount_sat,
                estimate_amount_sat,
                is_lightning,
            }));
        });
    }

    fn perform_send_fee_estimate(
        &mut self,
        request_id: u64,
        destination: String,
        amount_sat: u64,
        estimate_amount_sat: u64,
        is_lightning: bool,
    ) {
        let Some(wallet) = self.wallet.clone() else {
            self.clear_send_fee_estimate();
            return;
        };

        let tx = self.tx.clone();
        self.rt.spawn(async move {
            let estimate_amount = Amount::from_sat(estimate_amount_sat);
            let result = if is_lightning {
                wallet.estimate_lightning_send_fee(estimate_amount).await
            } else {
                wallet.estimate_arkoor_payment_fee(estimate_amount).await
            };
            let msg = match result {
                Ok(estimate) => AsyncMsg::SendFeeEstimated {
                    request_id,
                    destination,
                    amount_sat,
                    fee_sat: estimate.fee.to_sat(),
                    total_sat: estimate.gross_amount.to_sat(),
                },
                Err(e) => AsyncMsg::SendFeeEstimateFailed {
                    request_id,
                    destination,
                    amount_sat,
                    error: format!("{e:#}"),
                },
            };
            let _ = tx.send(CoreMsg::Async(msg));
        });
    }

    fn clear_send_fee_estimate(&mut self) {
        self.send_fee_estimate_request_id = self.send_fee_estimate_request_id.saturating_add(1);
        self.state.send.estimating_fee = false;
        self.state.send.fee_estimate_sat = None;
        self.state.send.total_cost_sat = None;
        self.state.send.fee_estimate_error = None;
    }

    fn send_fee_estimate_is_current(
        &self,
        request_id: u64,
        destination: &str,
        amount_sat: u64,
    ) -> bool {
        self.send_fee_estimate_request_id == request_id
            && self.state.send.destination.trim() == destination
            && self.state.send.amount_sat == amount_sat
    }

    fn request_capability(&mut self, kind: CapabilityRequestKind) {
        self.next_capability_id += 1;
        self.state.capability_request = Some(CapabilityRequest {
            id: self.next_capability_id,
            kind,
        });
    }

    fn cache_primal_profile_contacts(
        &mut self,
        records: Vec<PrimalProfileContact>,
    ) -> Vec<Contact> {
        records
            .into_iter()
            .map(|record| self.cache_primal_profile_contact(record))
            .collect()
    }

    fn cache_primal_profile_contact(&mut self, record: PrimalProfileContact) -> Contact {
        let mut contact = record.contact;
        let cached = self
            .profile_db
            .as_ref()
            .and_then(|conn| load_profile(conn, &record.pubkey_hex).ok().flatten());
        let cached_file_url = cached
            .as_ref()
            .filter(|entry| {
                !record.picture_remote_url.is_empty()
                    && entry.picture_cached_url == record.picture_remote_url
            })
            .and_then(|_| profile_picture_file_url(&self.cache_dir, &record.pubkey_hex));

        if let Some(file_url) = cached_file_url {
            contact.picture = file_url;
        } else if !record.picture_remote_url.is_empty() {
            contact.picture = record.picture_remote_url.clone();
        }

        if let Some(conn) = self.profile_db.as_ref() {
            let previous = cached.unwrap_or_else(|| ProfileCacheEntry {
                pubkey: record.pubkey_hex.clone(),
                metadata_json: "{}".to_string(),
                name: String::new(),
                picture_remote_url: String::new(),
                picture_cached_url: String::new(),
                picture_cached_at: 0,
                lightning_address: String::new(),
                lnurl: String::new(),
                event_created_at: 0,
            });
            let same_remote = previous.picture_remote_url == record.picture_remote_url;
            let entry = ProfileCacheEntry {
                pubkey: record.pubkey_hex,
                metadata_json: record.metadata_json,
                name: contact.name.clone(),
                picture_remote_url: record.picture_remote_url.clone(),
                picture_cached_url: if same_remote {
                    previous.picture_cached_url
                } else {
                    String::new()
                },
                picture_cached_at: if same_remote {
                    previous.picture_cached_at
                } else {
                    0
                },
                lightning_address: contact.lightning_address.clone(),
                lnurl: contact.lnurl.clone(),
                event_created_at: record.event_created_at,
            };
            let _ = save_profile(conn, &entry);
        }

        contact
    }

    fn prefetch_profile_pictures(&mut self, contact_ids: Vec<String>) {
        let mut missing_profile_pubkeys = Vec::new();
        for contact_id in contact_ids.into_iter().take(80) {
            let Some(contact) = self
                .state
                .send
                .search_results
                .iter()
                .chain(self.state.send.global_search_results.iter())
                .chain(self.state.nostr.contacts.iter())
                .find(|contact| contact.id == contact_id)
                .cloned()
            else {
                continue;
            };

            let Ok(pubkey) = public_key_from_npub_or_hex(&contact.npub) else {
                continue;
            };
            let pubkey_hex = pubkey.to_hex();
            let mut remote_url = contact.picture;
            if remote_url.starts_with("file://") {
                continue;
            }
            if remote_url.trim().is_empty() {
                remote_url = self
                    .profile_db
                    .as_ref()
                    .and_then(|conn| load_profile(conn, &pubkey_hex).ok().flatten())
                    .map(|entry| entry.picture_remote_url)
                    .unwrap_or_default();
            }

            if remote_url.trim().is_empty() {
                if self.profile_info_requests.insert(pubkey_hex.clone()) {
                    missing_profile_pubkeys.push(pubkey);
                }
                continue;
            }
            let cached_file_url = self
                .profile_db
                .as_ref()
                .and_then(|conn| load_profile(conn, &pubkey_hex).ok().flatten())
                .filter(|entry| entry.picture_cached_url == remote_url)
                .and_then(|_| profile_picture_file_url(&self.cache_dir, &pubkey_hex));
            if cached_file_url.is_some() {
                self.refresh_contact_picture_for_pubkey(&pubkey_hex);
                continue;
            }

            self.spawn_profile_picture_download(pubkey_hex, remote_url);
        }

        self.spawn_primal_profile_prefetch(missing_profile_pubkeys);
    }

    fn spawn_primal_profile_prefetch(&self, pubkeys: Vec<NostrPublicKey>) {
        if pubkeys.is_empty() {
            return;
        }

        let tx = self.tx.clone();
        self.rt.spawn(async move {
            let pubkey_hexes = pubkeys.iter().map(|key| key.to_hex()).collect::<Vec<_>>();
            match primal_profile_contacts(pubkeys, true).await {
                Ok(records) => {
                    let _ = tx.send(CoreMsg::Async(AsyncMsg::PrimalProfilesLoaded { records }));
                }
                Err(_) => {
                    let _ = tx.send(CoreMsg::Async(AsyncMsg::PrimalProfilesFailed {
                        pubkeys: pubkey_hexes,
                    }));
                }
            }
        });
    }

    fn spawn_profile_picture_download(&mut self, pubkey: String, remote_url: String) {
        if remote_url.trim().is_empty() {
            return;
        }
        let Some(scheme) = remote_url.split(':').next().map(|s| s.to_ascii_lowercase()) else {
            return;
        };
        if scheme != "https" && scheme != "http" {
            return;
        }

        let download_key = profile_picture_download_key(&pubkey, &remote_url);
        if !self.profile_picture_downloads.insert(download_key) {
            return;
        }

        let tx = self.tx.clone();
        let cache_dir = self.cache_dir.clone();
        let semaphore = self.profile_picture_download_semaphore.clone();
        self.rt.spawn(async move {
            let client = reqwest::Client::new();
            let failed_pubkey = pubkey.clone();
            let failed_remote_url = remote_url.clone();
            match download_profile_picture(client, cache_dir, pubkey, remote_url, semaphore).await {
                Ok((pubkey, remote_url)) => {
                    let _ = tx.send(CoreMsg::Async(AsyncMsg::ProfilePictureCached {
                        pubkey,
                        remote_url,
                    }));
                }
                Err(err) => {
                    let _ = err;
                    let _ = tx.send(CoreMsg::Async(AsyncMsg::ProfilePictureCacheFailed {
                        pubkey: failed_pubkey,
                        remote_url: failed_remote_url,
                    }));
                }
            }
        });
    }

    fn refresh_contact_picture_for_pubkey(&mut self, pubkey: &str) {
        let Some(file_url) = profile_picture_file_url(&self.cache_dir, pubkey) else {
            return;
        };
        for contact in &mut self.state.nostr.contacts {
            let Ok(contact_pubkey) = public_key_from_npub_or_hex(&contact.npub) else {
                continue;
            };
            if contact_pubkey.to_hex() == pubkey {
                contact.picture = file_url.clone();
            }
        }
        for contact in &mut self.state.send.global_search_results {
            let Ok(contact_pubkey) = public_key_from_npub_or_hex(&contact.npub) else {
                continue;
            };
            if contact_pubkey.to_hex() == pubkey {
                contact.picture = file_url.clone();
            }
        }
        self.save_app_data();
    }

    fn pay_lightning_invoice(&mut self, invoice: String, amount_sat: Option<u64>) {
        if let Some(amount_sat) = amount_sat.filter(|amount| *amount > 0) {
            if amount_sat > self.state.wallet.balance_sat {
                self.state.toast =
                    Some("Insufficient balance for this Lightning payment.".to_string());
                return;
            }
        }
        let Some(wallet) = self.wallet.clone() else {
            self.state.toast = Some("Wallet is not ready yet.".to_string());
            return;
        };
        self.state.busy.sending_payment = true;
        self.state.send.phase = SendPhase::Sending;
        self.state.send.last_result = None;
        let tx = self.tx.clone();
        self.rt.spawn(async move {
            let user_amount = amount_sat.filter(|a| *a > 0).map(Amount::from_sat);
            let parsed = Bolt11Invoice::from_str(&invoice);
            let msg = match parsed {
                Ok(invoice) => match wallet
                    .pay_lightning_invoice(invoice, user_amount, true)
                    .await
                {
                    Ok(_) => AsyncMsg::Paid("Lightning invoice paid.".to_string()),
                    Err(e) => AsyncMsg::Error(format!("Lightning payment failed: {e:#}")),
                },
                Err(e) => AsyncMsg::Error(format!("Invalid Lightning invoice: {e}")),
            };
            let _ = tx.send(CoreMsg::Async(msg));
        });
    }

    fn pay_lnurl_destination(&mut self, destination: String, amount_sat: u64) {
        if amount_sat == 0 {
            self.state.toast =
                Some("Enter an amount before sending to this Lightning address.".to_string());
            return;
        }
        if amount_sat > self.state.wallet.balance_sat {
            self.state.toast = Some("Insufficient balance for this Lightning payment.".to_string());
            return;
        }
        let Some(wallet) = self.wallet.clone() else {
            self.state.toast = Some("Wallet is not ready yet.".to_string());
            return;
        };
        self.state.busy.sending_payment = true;
        self.state.send.phase = SendPhase::Sending;
        self.state.send.last_result = None;
        let tx = self.tx.clone();
        self.rt.spawn(async move {
            let msg = match resolve_lnurl_pay_invoice(&destination, amount_sat).await {
                Ok(invoice) => match Bolt11Invoice::from_str(&invoice) {
                    Ok(invoice) => match amount_sat.checked_mul(1_000) {
                        Some(requested_msat) => match invoice.amount_milli_satoshis() {
                            Some(invoice_msat) if invoice_msat == requested_msat => match wallet
                                .pay_lightning_invoice(
                                    invoice,
                                    Some(Amount::from_sat(amount_sat)),
                                    true,
                                )
                                .await
                            {
                                Ok(_) => {
                                    AsyncMsg::Paid("Lightning address payment sent.".to_string())
                                }
                                Err(e) => {
                                    AsyncMsg::Error(format!("Lightning payment failed: {e:#}"))
                                }
                            },
                            Some(invoice_msat) => AsyncMsg::Error(format!(
                                "LNURL invoice amount mismatch: requested {} sats, got {} sats.",
                                amount_sat,
                                msats_to_display_sats(invoice_msat)
                            )),
                            None => AsyncMsg::Error(
                                "LNURL invoice did not include an amount.".to_string(),
                            ),
                        },
                        None => AsyncMsg::Error(
                            "LNURL payment failed: send amount is too large.".to_string(),
                        ),
                    },
                    Err(e) => AsyncMsg::Error(format!("Invalid LNURL invoice: {e}")),
                },
                Err(e) => AsyncMsg::Error(format!("LNURL payment failed: {e:#}")),
            };
            let _ = tx.send(CoreMsg::Async(msg));
        });
    }

    fn pay_ark_address(&mut self, address: String, amount_sat: u64) {
        if amount_sat == 0 {
            self.state.toast = Some("Enter an amount before sending.".to_string());
            return;
        }
        if amount_sat > self.state.wallet.balance_sat {
            self.state.toast = Some("Insufficient balance for this Ark payment.".to_string());
            return;
        }
        let Some(wallet) = self.wallet.clone() else {
            self.state.toast = Some("Wallet is not ready yet.".to_string());
            return;
        };
        self.state.busy.sending_payment = true;
        self.state.send.phase = SendPhase::Sending;
        self.state.send.last_result = None;
        let tx = self.tx.clone();
        self.rt.spawn(async move {
            let msg = match address.parse() {
                Ok(address) => match wallet
                    .send_arkoor_payment(&address, Amount::from_sat(amount_sat))
                    .await
                {
                    Ok(_) => AsyncMsg::Paid("Ark payment sent.".to_string()),
                    Err(e) => AsyncMsg::Error(format!("Ark payment failed: {e:#}")),
                },
                Err(e) => AsyncMsg::Error(format!("Invalid Ark address: {e}")),
            };
            let _ = tx.send(CoreMsg::Async(msg));
        });
    }

    fn generate_nostr_key(&mut self) {
        let keys = Keys::generate();
        match (keys.secret_key().to_bech32(), keys.public_key().to_bech32()) {
            (Ok(nsec), Ok(npub)) => {
                let _ = self.secrets.set_secret(NOSTR_SECRET_KEY.to_string(), nsec);
                self.reset_nostr_identity(npub);
                self.state.toast = Some("Nostr key generated in Keychain.".to_string());
                self.save_app_data();
                self.sync_primal_follow_contacts(false);
            }
            _ => {
                self.state.toast = Some("Could not encode generated Nostr key.".to_string());
            }
        }
    }

    fn import_nostr_secret(&mut self, nsec_or_hex: String) {
        let value = nsec_or_hex.trim().to_string();
        if value.is_empty() {
            self.state.toast = Some("Enter a Nostr secret key.".to_string());
            return;
        }
        match Keys::parse(&value) {
            Ok(keys) => match (keys.secret_key().to_bech32(), keys.public_key().to_bech32()) {
                (Ok(nsec), Ok(npub)) => {
                    let _ = self.secrets.set_secret(NOSTR_SECRET_KEY.to_string(), nsec);
                    self.reset_nostr_identity(npub);
                    self.state.toast =
                        Some("Nostr key imported. Refreshing profile...".to_string());
                    self.save_app_data();
                    self.refresh_nostr_profile();
                    self.sync_primal_follow_contacts(false);
                }
                _ => {
                    self.state.toast = Some("Could not encode imported Nostr key.".to_string());
                }
            },
            Err(e) => {
                self.state.toast = Some(format!("Invalid Nostr secret key: {e}"));
            }
        }
    }

    fn export_nostr_secret(&mut self) {
        self.state.toast = self
            .secrets
            .get_secret(NOSTR_SECRET_KEY.to_string())
            .or_else(|| Some("No Nostr secret key found.".to_string()));
    }

    fn clear_nostr_key(&mut self) {
        let _ = self.secrets.delete_secret(NOSTR_SECRET_KEY.to_string());
        if !self.ensure_wallet_derived_nostr_key() {
            self.state.nostr.npub = None;
            self.state.nostr.name = "Rebel".to_string();
            self.state.nostr.about.clear();
            self.state.nostr.picture.clear();
            self.state.nostr.lud16.clear();
            self.state.nostr.nip05.clear();
            self.state.nostr.deleted = false;
            self.state.nostr.contacts.clear();
            self.state.direct_messages.clear();
        }
        self.save_app_data();
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
        self.state.send.global_search_results.clear();
        self.state.toast = Some("Nostr profile cache cleared.".to_string());
        self.save_app_data();
    }

    fn load_nostr_key(&mut self) {
        if let Some(secret) = self.secrets.get_secret(NOSTR_SECRET_KEY.to_string()) {
            if let Ok(keys) = Keys::parse(&secret) {
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
            return false;
        }

        let Some(mnemonic) = self.secrets.get_secret(WALLET_SEED_KEY.to_string()) else {
            return false;
        };

        let Ok(keys) = derive_nostr_keys_from_mnemonic(&mnemonic) else {
            return false;
        };

        match (keys.secret_key().to_bech32(), keys.public_key().to_bech32()) {
            (Ok(nsec), Ok(npub)) => {
                let _ = self.secrets.set_secret(NOSTR_SECRET_KEY.to_string(), nsec);
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
            .context("Nostr secret key not found")?;
        Keys::parse(&secret).context("invalid Nostr secret key")
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
                if let Some(event) = events.iter().max_by_key(|event| event.created_at.as_secs()) {
                    apply_metadata_content(&mut nostr, &event.content)?;
                }
                Ok::<_, anyhow::Error>(AsyncMsg::NostrProfileLoaded(nostr))
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
                            name: fields
                                .get(3)
                                .cloned()
                                .unwrap_or_else(|| truncate_middle(&npub, 18)),
                            followed: true,
                            picture: String::new(),
                            lightning_address: String::new(),
                            lnurl: String::new(),
                            last_used: now_unix(),
                        });
                    }
                    if !pubkeys.is_empty() {
                        let metadata_filter = Filter::new()
                            .authors(pubkeys)
                            .kind(Kind::Metadata)
                            .limit(250);
                        let metadata_events = client
                            .fetch_events(metadata_filter)
                            .timeout(Duration::from_secs(10))
                            .await?;
                        for metadata_event in metadata_events.iter() {
                            let npub = metadata_event.pubkey.to_bech32().unwrap();
                            let Some(contact) = contacts.iter_mut().find(|c| c.npub == npub) else {
                                continue;
                            };
                            let Ok(metadata) = Metadata::from_json(metadata_event.content.clone())
                            else {
                                continue;
                            };
                            contact.name = metadata
                                .display_name
                                .or(metadata.name)
                                .unwrap_or_else(|| contact.name.clone());
                            contact.picture = metadata
                                .picture
                                .map(|u| u.to_string())
                                .unwrap_or_else(|| contact.picture.clone());
                            contact.lightning_address = metadata
                                .lud16
                                .unwrap_or_else(|| contact.lightning_address.clone());
                        }
                    }
                }
                Ok::<_, anyhow::Error>(AsyncMsg::NostrContactsLoaded(contacts))
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

    fn load_app_data(&mut self) {
        let Ok(raw) = std::fs::read_to_string(&self.app_data_path) else {
            return;
        };
        match serde_json::from_str::<PersistedAppData>(&raw) {
            Ok(data) => {
                self.state.nostr = data.nostr;
                self.hydrate_cached_profile_pictures();
                self.state.receive.amount_sat = data.receive_amount_sat;
                self.state.receive.memo = data.receive_memo;
                self.state.wallet.network = data.network;
                let server_config = ServerConfig::for_network(self.state.wallet.network.clone());
                self.state.wallet.server_address = server_config.server_address;
                self.state.wallet.esplora_address = server_config.esplora_address;
                self.state.wallet.price_currency = data.price_currency.currency;
                self.state.lightning_address.backing_ark_address = data
                    .lightning_address_ark_address
                    .filter(|address| !address.trim().is_empty());
            }
            Err(e) => {
                self.state.toast = Some(format!("Could not load local app data: {e}"));
            }
        }
    }

    fn hydrate_cached_profile_pictures(&mut self) {
        let cache_dir = self.cache_dir.clone();
        let profile_db = self.profile_db.as_ref();
        for contact in &mut self.state.nostr.contacts {
            hydrate_contact_picture(profile_db, &cache_dir, contact);
        }
        for contact in &mut self.state.send.global_search_results {
            hydrate_contact_picture(profile_db, &cache_dir, contact);
        }
    }

    fn save_app_data(&self) {
        let mut nostr = self.state.nostr.clone();
        sanitize_persisted_contact_pictures(self.profile_db.as_ref(), &mut nostr.contacts);
        let data = PersistedAppData {
            nostr,
            receive_amount_sat: self.state.receive.amount_sat,
            receive_memo: self.state.receive.memo.clone(),
            network: self.state.wallet.network.clone(),
            servers: ServerConfig::from_wallet(&self.state.wallet),
            price_currency: PersistedPriceCurrency {
                currency: self.state.wallet.price_currency.clone(),
            },
            lightning_address_ark_address: None,
        };
        if let Ok(raw) = serde_json::to_string_pretty(&data) {
            let _ = std::fs::create_dir_all(&self.data_dir);
            let _ = std::fs::write(&self.app_data_path, raw);
        }
    }
}

fn hydrate_contact_picture(
    profile_db: Option<&rusqlite::Connection>,
    data_dir: &PathBuf,
    contact: &mut Contact,
) {
    if contact.picture.starts_with("file://") {
        contact.picture.clear();
    }
    let Ok(pubkey) = public_key_from_npub_or_hex(&contact.npub) else {
        return;
    };
    let pubkey_hex = pubkey.to_hex();
    let Some(entry) = profile_db.and_then(|conn| load_profile(conn, &pubkey_hex).ok().flatten())
    else {
        return;
    };
    if !entry.picture_remote_url.is_empty() && entry.picture_cached_url == entry.picture_remote_url
    {
        if let Some(file_url) = profile_picture_file_url(data_dir, &pubkey_hex) {
            contact.picture = file_url;
            return;
        }
    }
    if contact.picture.is_empty() {
        contact.picture = entry.picture_remote_url;
    }
}

fn sanitize_persisted_contact_pictures(
    profile_db: Option<&rusqlite::Connection>,
    contacts: &mut [Contact],
) {
    for contact in contacts {
        if !contact.picture.starts_with("file://") {
            continue;
        }
        let Ok(pubkey) = public_key_from_npub_or_hex(&contact.npub) else {
            contact.picture.clear();
            continue;
        };
        contact.picture = profile_db
            .and_then(|conn| load_profile(conn, &pubkey.to_hex()).ok().flatten())
            .map(|entry| entry.picture_remote_url)
            .unwrap_or_default();
    }
}

fn load_wallet_metadata_value(
    data_dir: &PathBuf,
    network: WalletNetwork,
    key: &str,
) -> Option<String> {
    let db_path = data_dir.join(network.db_file_name());
    let conn = rusqlite::Connection::open(db_path).ok()?;
    ensure_wallet_metadata_table(&conn).ok()?;
    conn.query_row(
        "SELECT value FROM rebel_wallet_metadata WHERE key = ?1",
        [key],
        |row| row.get::<_, String>(0),
    )
    .ok()
    .filter(|value| !value.trim().is_empty())
}

fn save_wallet_metadata_value(
    data_dir: &PathBuf,
    network: WalletNetwork,
    key: &str,
    value: &str,
) -> rusqlite::Result<()> {
    std::fs::create_dir_all(data_dir).ok();
    let db_path = data_dir.join(network.db_file_name());
    let conn = rusqlite::Connection::open(db_path)?;
    ensure_wallet_metadata_table(&conn)?;
    conn.execute(
        "INSERT INTO rebel_wallet_metadata (key, value)
         VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        (key, value),
    )?;
    Ok(())
}

fn ensure_wallet_metadata_table(conn: &rusqlite::Connection) -> rusqlite::Result<()> {
    conn.execute(
        "CREATE TABLE IF NOT EXISTS rebel_wallet_metadata (
            key TEXT PRIMARY KEY,
            value TEXT NOT NULL
        )",
        [],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use nostr_sdk::prelude::SecretKey as NostrSecretKey;

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
}
