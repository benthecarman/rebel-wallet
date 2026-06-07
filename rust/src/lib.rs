use std::path::PathBuf;
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};
use std::thread;
use std::time::Duration;

use anyhow::Context;
use bark::ark::lightning::PaymentHash;
use bark::lightning_invoice::Bolt11Invoice;
use bark::Wallet;
use bip39::Mnemonic;
use bitcoin::Amount;
use flume::{Receiver, Sender};
use nostr_sdk::prelude::{
    nip04, Contact as NostrContact, ContactListBuilder, EventBuilder, EventBuilderTemplate, Filter,
    FinalizeEvent, JsonUtil, Keys, Kind, Metadata, PublicKey as NostrPublicKey, Tag, ToBech32,
};
use tokio::runtime::Runtime;

mod actions;
mod activity;
mod nostr_support;
mod payments;
mod persistence;
mod price;
mod state;
mod time;
mod updates;
mod wallet;

use activity::{activity_from_movement, truncate_middle};
use nostr_support::{
    contact_id, merge_contacts, metadata_from_state, nostr_client, public_key_from_npub_or_hex,
    upload_profile_picture,
};
use payments::{monitor_lightning_receive, parse_send_destination};
use persistence::{validate_server_url, PersistedAppData, PersistedPriceCurrency, ServerConfig};
use price::fetch_bitcoin_price;
use time::{now_label, now_unix};
use wallet::{open_bark_wallet, WalletOpenMode};

pub use actions::AppAction;
pub use state::{
    ActivityIconKind, ActivityItem, AppState, BusyState, CapabilityRequest, CapabilityRequestKind,
    Contact, CurrencyOption, MainTab, NostrMessage, NostrState, PriceCurrency, ReceiveMethod,
    ReceivePhase, ReceiveState, Router, Screen, SendDestinationKind, SendPhase, SendState,
    SetupState, WalletState,
};
pub use updates::AppUpdate;
pub(crate) use updates::{AsyncMsg, CoreMsg};

uniffi::setup_scaffolding!();

const WALLET_SEED_KEY: &str = "wallet_seed";
const NOSTR_SECRET_KEY: &str = "nostr_secret";
pub(crate) const SIGNET_SERVER: &str = "https://ark.signet.2nd.dev";
pub(crate) const SIGNET_ESPLORA: &str = "https://esplora.signet.2nd.dev";

#[uniffi::export(callback_interface)]
pub trait AppReconciler: Send + Sync + 'static {
    fn reconcile(&self, update: AppUpdate);
}

#[uniffi::export(callback_interface)]
pub trait SecretStore: Send + Sync + 'static {
    fn get_secret(&self, key: String) -> Option<String>;
    fn set_secret(&self, key: String, value: String) -> bool;
    fn delete_secret(&self, key: String) -> bool;
}

#[derive(uniffi::Object)]
pub struct FfiApp {
    core_tx: Sender<CoreMsg>,
    update_rx: Receiver<AppUpdate>,
    listening: AtomicBool,
    shared_state: Arc<RwLock<AppState>>,
}

#[uniffi::export]
impl FfiApp {
    #[uniffi::constructor]
    pub fn new(data_dir: String, secret_store: Box<dyn SecretStore>) -> Arc<Self> {
        let (update_tx, update_rx) = flume::unbounded();
        let (core_tx, core_rx) = flume::unbounded::<CoreMsg>();
        let shared_state = Arc::new(RwLock::new(AppState::initial()));
        let shared_for_core = shared_state.clone();
        let data_dir = PathBuf::from(data_dir);
        let secrets: Arc<dyn SecretStore> = Arc::from(secret_store);
        let tx_for_bootstrap = core_tx.clone();

        thread::spawn(move || {
            let rt = Runtime::new().expect("tokio runtime");
            let mut core = AppCore::new(data_dir, secrets, tx_for_bootstrap, rt);
            core.emit(&shared_for_core, &update_tx);

            while let Ok(msg) = core_rx.recv() {
                core.handle(msg);
                core.emit(&shared_for_core, &update_tx);
            }
        });

        Arc::new(Self {
            core_tx,
            update_rx,
            listening: AtomicBool::new(false),
            shared_state,
        })
    }

    pub fn state(&self) -> AppState {
        match self.shared_state.read() {
            Ok(g) => g.clone(),
            Err(poison) => poison.into_inner().clone(),
        }
    }

    pub fn dispatch(&self, action: AppAction) {
        let _ = self.core_tx.send(CoreMsg::Action(action));
    }

    pub fn listen_for_updates(&self, reconciler: Box<dyn AppReconciler>) {
        if self
            .listening
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return;
        }

        let rx = self.update_rx.clone();
        thread::spawn(move || {
            while let Ok(update) = rx.recv() {
                reconciler.reconcile(update);
            }
        });
    }
}

struct AppCore {
    state: AppState,
    data_dir: PathBuf,
    app_data_path: PathBuf,
    secrets: Arc<dyn SecretStore>,
    tx: Sender<CoreMsg>,
    rt: Runtime,
    wallet: Option<Wallet>,
    rev: u64,
    next_capability_id: u64,
}

impl AppCore {
    fn new(
        data_dir: PathBuf,
        secrets: Arc<dyn SecretStore>,
        tx: Sender<CoreMsg>,
        rt: Runtime,
    ) -> Self {
        Self {
            state: AppState::initial(),
            app_data_path: data_dir.join("rebel-app-data.json"),
            data_dir,
            secrets,
            tx,
            rt,
            wallet: None,
            rev: 0,
            next_capability_id: 0,
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
            AppAction::RefreshPrice => self.refresh_price(),
            AppAction::SetPriceCurrency { currency } => self.set_price_currency(currency),
            AppAction::ConfigureServers {
                server_address,
                esplora_address,
            } => self.configure_servers(server_address, esplora_address),
            AppAction::SelectTab { tab } => self.state.router.selected_tab = tab,
            AppAction::PushScreen { screen } => self.state.router.screen_stack.push(screen),
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
            AppAction::SetSendDestination { destination } => self.set_send_destination(destination),
            AppAction::SetSendAmount { amount_sat } => self.state.send.amount_sat = amount_sat,
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
                self.state.nostr.name = name;
                self.state.nostr.about = about;
                self.state.nostr.picture = picture;
                self.state.nostr.lud16 = lud16;
                self.state.nostr.nip05 = nip05;
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
                self.state.toast = Some("Rebel Wallet is ready.".to_string());
                let _ = self
                    .secrets
                    .set_secret(WALLET_SEED_KEY.to_string(), mnemonic);
                self.sync_wallet();
            }
            AsyncMsg::WalletSynced {
                balance_sat,
                pending_receive_sat,
                pending_send_sat,
                activity,
            } => {
                self.state.wallet.balance_sat = balance_sat;
                self.state.wallet.pending_receive_sat = pending_receive_sat;
                self.state.wallet.pending_send_sat = pending_send_sat;
                self.state.wallet.last_sync = Some(now_label());
                self.state.activity = activity;
            }
            AsyncMsg::ArkAddress(address) => {
                self.state.receive.ark_address = Some(address);
                self.state.receive.phase = ReceivePhase::ShowingRequest;
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
                self.state.toast = Some("Lightning receive claimed.".to_string());
                self.sync_wallet();
            }
            AsyncMsg::Paid(result) => {
                self.state.send.phase = SendPhase::Success;
                self.state.send.success_amount_display = self.state.send.amount_display.clone();
                self.state.send.last_result = Some(result.clone());
                self.state.toast = Some(result);
                self.sync_wallet();
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
                self.state.toast = Some("Nostr profile refreshed.".to_string());
                self.save_app_data();
            }
            AsyncMsg::NostrContactsLoaded(contacts) => {
                merge_contacts(&mut self.state.nostr.contacts, contacts);
                self.state.toast = Some("Nostr contacts refreshed.".to_string());
                self.save_app_data();
                self.sync_wallet();
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
            AsyncMsg::WalletSynced { .. } => self.state.busy.syncing_wallet = false,
            AsyncMsg::ArkAddress(_) | AsyncMsg::LightningInvoice { .. } => {
                self.state.busy.creating_invoice = false;
            }
            AsyncMsg::Paid(_) => self.state.busy.sending_payment = false,
            AsyncMsg::NostrProfilePictureUploaded(_) => {
                self.state.busy.uploading_profile_picture = false;
            }
            AsyncMsg::NostrPublished(_) => self.state.busy.publishing_nostr = false,
            AsyncMsg::NostrProfileLoaded(_) | AsyncMsg::NostrContactsLoaded(_) => {
                self.state.busy.refreshing_contacts = false;
            }
            AsyncMsg::Error(_) => self.state.busy = BusyState::default(),
            AsyncMsg::LightningReceiveStatus { .. }
            | AsyncMsg::LightningReceiveClaimed { .. }
            | AsyncMsg::Seed(_)
            | AsyncMsg::DirectMessagesLoaded(_)
            | AsyncMsg::DirectMessageSent(_)
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
            self.open_wallet(mnemonic, WalletOpenMode::Open);
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

    fn configure_servers(&mut self, server_address: String, esplora_address: String) {
        let server_address = server_address.trim().trim_end_matches('/').to_string();
        let esplora_address = esplora_address.trim().trim_end_matches('/').to_string();

        if let Err(message) = validate_server_url("Ark server", &server_address) {
            self.state.toast = Some(message);
            return;
        }
        if let Err(message) = validate_server_url("Esplora", &esplora_address) {
            self.state.toast = Some(message);
            return;
        }

        let changed = self.state.wallet.server_address != server_address
            || self.state.wallet.esplora_address != esplora_address;
        self.state.wallet.server_address = server_address;
        self.state.wallet.esplora_address = esplora_address;
        self.save_app_data();

        if changed {
            if let Some(seed) = self.secrets.get_secret(WALLET_SEED_KEY.to_string()) {
                self.wallet = None;
                self.state.busy.opening_wallet = true;
                self.open_wallet(seed, WalletOpenMode::Open);
                self.state.toast = Some("Servers saved. Reconnecting wallet.".to_string());
            } else {
                self.state.toast = Some("Servers saved.".to_string());
            }
        } else {
            self.state.toast = Some("Servers already up to date.".to_string());
        }
    }

    fn set_price_currency(&mut self, currency: PriceCurrency) {
        self.state.wallet.price_currency = currency;
        self.state.wallet.btc_price = None;
        self.save_app_data();
        self.refresh_price();
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
        self.rt.spawn(async move {
            let result = async {
                wallet.sync().await;
                let balance = wallet.balance().await.context("balance failed")?;
                let history = wallet.history().await.context("history failed")?;
                let activity = history
                    .into_iter()
                    .map(|movement| activity_from_movement(movement, &contacts))
                    .collect();
                Ok::<_, anyhow::Error>(AsyncMsg::WalletSynced {
                    balance_sat: balance.spendable.to_sat(),
                    pending_receive_sat: balance.claimable_lightning_receive.to_sat(),
                    pending_send_sat: balance.pending_lightning_send.to_sat(),
                    activity,
                })
            }
            .await
            .unwrap_or_else(|e| AsyncMsg::Error(format!("Sync failed: {e:#}")));
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
            let msg = match wallet.new_address().await {
                Ok(address) => AsyncMsg::ArkAddress(address.to_string()),
                Err(e) => AsyncMsg::Error(format!("Could not create Ark address: {e:#}")),
            };
            let _ = tx.send(CoreMsg::Async(msg));
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
        if self.state.send.amount_sat > self.state.wallet.balance_sat {
            self.state.toast = Some("Insufficient balance for this send.".to_string());
            return;
        }
        let lower = destination.to_lowercase();
        if lower.starts_with("lightning:") || lower.starts_with("ln") {
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
            self.state.send.destination = raw;
        }
        self.state.send.phase = SendPhase::Editing;
    }

    fn reset_send_draft(&mut self) {
        self.state.send.destination.clear();
        self.state.send.amount_sat = 0;
        self.state.send.memo.clear();
        self.state.send.last_result = None;
        self.state.send.phase = SendPhase::Drafting;
    }

    fn request_capability(&mut self, kind: CapabilityRequestKind) {
        self.next_capability_id += 1;
        self.state.capability_request = Some(CapabilityRequest {
            id: self.next_capability_id,
            kind,
        });
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
        self.state.nostr.npub = None;
        self.state.nostr.name = "Rebel".to_string();
        self.state.nostr.about.clear();
        self.state.nostr.picture.clear();
        self.state.nostr.lud16.clear();
        self.state.nostr.nip05.clear();
        self.state.nostr.contacts.clear();
        self.state.direct_messages.clear();
        self.state.toast = Some("Nostr key removed from Keychain.".to_string());
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
            }
        }
    }

    fn reset_nostr_identity(&mut self, npub: String) {
        self.state.nostr.npub = Some(npub);
        self.state.nostr.name = "Rebel".to_string();
        self.state.nostr.about.clear();
        self.state.nostr.picture.clear();
        self.state.nostr.lud16.clear();
        self.state.nostr.nip05.clear();
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
                    let metadata = Metadata::from_json(event.content.clone())?;
                    nostr.name = metadata
                        .display_name
                        .or(metadata.name)
                        .unwrap_or(nostr.name);
                    nostr.about = metadata.about.unwrap_or_default();
                    nostr.picture = metadata.picture.map(|u| u.to_string()).unwrap_or_default();
                    nostr.lud16 = metadata.lud16.unwrap_or_default();
                    nostr.nip05 = metadata.nip05.unwrap_or_default();
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
        self.state.busy.publishing_nostr = true;
        let tx = self.tx.clone();
        self.rt.spawn(async move {
            let result = async {
                let client = nostr_client().await?;
                let content = serde_json::json!({ "deleted": true }).to_string();
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
        let tx = self.tx.clone();
        self.rt.spawn(async move {
            let result = async {
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
            .unwrap_or_else(|e| AsyncMsg::Error(format!("Nostr contact refresh failed: {e:#}")));
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
                self.state.receive.amount_sat = data.receive_amount_sat;
                self.state.receive.memo = data.receive_memo;
                self.state.wallet.server_address = data.servers.server_address;
                self.state.wallet.esplora_address = data.servers.esplora_address;
                self.state.wallet.price_currency = data.price_currency.currency;
            }
            Err(e) => {
                self.state.toast = Some(format!("Could not load local app data: {e}"));
            }
        }
    }

    fn save_app_data(&self) {
        let data = PersistedAppData {
            nostr: self.state.nostr.clone(),
            receive_amount_sat: self.state.receive.amount_sat,
            receive_memo: self.state.receive.memo.clone(),
            servers: ServerConfig::from_wallet(&self.state.wallet),
            price_currency: PersistedPriceCurrency {
                currency: self.state.wallet.price_currency.clone(),
            },
        };
        if let Ok(raw) = serde_json::to_string_pretty(&data) {
            let _ = std::fs::create_dir_all(&self.data_dir);
            let _ = std::fs::write(&self.app_data_path, raw);
        }
    }
}
