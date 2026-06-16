use std::str::FromStr;

use bark::ark::Address as ArkAddress;
use bitcoin::Address as BitcoinAddress;
use serde::{Deserialize, Serialize};

use crate::{MAINNET_ESPLORA, MAINNET_SERVER, SIGNET_ESPLORA, SIGNET_SERVER};

#[derive(uniffi::Record, Clone, Debug)]
pub struct AppState {
    pub rev: u64,
    pub show_launch_splash: bool,
    pub router: Router,
    pub setup: SetupState,
    pub wallet: WalletState,
    pub supported_networks: Vec<NetworkOption>,
    pub supported_price_currencies: Vec<CurrencyOption>,
    pub receive: ReceiveState,
    pub send: SendState,
    pub lightning_address: LightningAddressState,
    pub nostr: NostrState,
    pub direct_messages: Vec<NostrMessage>,
    pub activity: Vec<ActivityItem>,
    pub recovery_phrase: Option<String>,
    pub toast: Option<String>,
    pub busy: BusyState,
    pub capability_request: Option<CapabilityRequest>,
}

#[derive(uniffi::Record, Clone, Debug)]
pub struct CurrencyOption {
    pub currency: PriceCurrency,
    pub code: String,
    pub name: String,
}

#[derive(uniffi::Record, Clone, Debug)]
pub struct NetworkOption {
    pub network: WalletNetwork,
    pub name: String,
    pub caption: String,
}

#[derive(uniffi::Record, Clone, Debug, Default)]
pub struct BusyState {
    pub bootstrapping: bool,
    pub opening_wallet: bool,
    pub syncing_wallet: bool,
    pub creating_invoice: bool,
    pub sending_payment: bool,
    pub uploading_profile_picture: bool,
    pub publishing_nostr: bool,
    pub maintaining_vtxos: bool,
    pub refreshing_contacts: bool,
}

#[derive(uniffi::Record, Clone, Debug, PartialEq)]
pub struct CapabilityRequest {
    pub id: u64,
    pub kind: CapabilityRequestKind,
}

#[derive(uniffi::Enum, Clone, Debug, PartialEq)]
pub enum CapabilityRequestKind {
    QrScan,
    ClipboardRead,
    PhotoPick,
}

#[derive(uniffi::Record, Clone, Debug, PartialEq)]
pub struct Router {
    pub default_screen: Screen,
    pub screen_stack: Vec<Screen>,
    pub selected_tab: MainTab,
}

#[derive(uniffi::Enum, Clone, Debug, PartialEq)]
pub enum MainTab {
    Home,
    Activity,
    Contacts,
    Settings,
}

#[derive(uniffi::Enum, Clone, Debug, PartialEq)]
pub enum Screen {
    Setup,
    Home,
    Send,
    Receive,
    Profile,
    Backup,
    Restore,
    Network,
    Currency,
    ContactDetail { contact_id: String },
}

#[derive(uniffi::Enum, Clone, Debug, PartialEq, Eq)]
pub enum SetupState {
    NeedsSetup,
    Ready,
    Error { message: String },
}

#[derive(uniffi::Enum, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PriceCurrency {
    BTC,
    USD,
    EUR,
    GBP,
}

#[derive(uniffi::Enum, Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum WalletNetwork {
    Mainnet,
    Signet,
}

impl WalletNetwork {
    pub(crate) fn display_name(&self) -> &'static str {
        match self {
            Self::Mainnet => "Mainnet",
            Self::Signet => "Signet",
        }
    }

    fn caption(&self) -> &'static str {
        match self {
            Self::Mainnet => "Real bitcoin network",
            Self::Signet => "Test bitcoin network",
        }
    }

    pub(crate) fn bitcoin_network(&self) -> bitcoin::Network {
        match self {
            Self::Mainnet => bitcoin::Network::Bitcoin,
            Self::Signet => bitcoin::Network::Signet,
        }
    }

    pub(crate) fn db_file_name(&self) -> &'static str {
        match self {
            Self::Mainnet => "rebel-wallet-mainnet.sqlite",
            Self::Signet => "rebel-wallet-signet.sqlite",
        }
    }

    pub(crate) fn server_address(&self) -> &'static str {
        match self {
            Self::Mainnet => MAINNET_SERVER,
            Self::Signet => SIGNET_SERVER,
        }
    }

    pub(crate) fn server_access_token(&self) -> Option<&'static str> {
        None
    }

    pub(crate) fn esplora_address(&self) -> &'static str {
        match self {
            Self::Mainnet => MAINNET_ESPLORA,
            Self::Signet => SIGNET_ESPLORA,
        }
    }
}

impl PriceCurrency {
    pub(crate) fn code(&self) -> &'static str {
        match self {
            Self::BTC => "BTC",
            Self::USD => "USD",
            Self::EUR => "EUR",
            Self::GBP => "GBP",
        }
    }

    fn display_name(&self) -> &'static str {
        match self {
            Self::BTC => "Bitcoin",
            Self::USD => "US Dollar",
            Self::EUR => "Euro",
            Self::GBP => "British Pound",
        }
    }

    fn symbol(&self) -> &'static str {
        match self {
            Self::BTC => "₿",
            Self::USD => "$",
            Self::EUR => "€",
            Self::GBP => "£",
        }
    }

    fn max_fractional_digits(&self) -> usize {
        match self {
            Self::BTC => 8,
            Self::USD | Self::EUR | Self::GBP => 2,
        }
    }

    fn approximate(&self) -> &'static str {
        match self {
            Self::BTC | Self::USD => "",
            Self::EUR | Self::GBP => "~",
        }
    }
}

#[derive(uniffi::Record, Clone, Debug)]
pub struct WalletState {
    pub network: WalletNetwork,
    pub network_name: String,
    pub default_server_address: String,
    pub default_esplora_address: String,
    pub server_address: String,
    pub esplora_address: String,
    pub price_currency: PriceCurrency,
    pub price_currency_code: String,
    pub price_currency_name: String,
    pub btc_price: Option<f64>,
    pub balance_sat: u64,
    pub balance_display: String,
    pub balance_fiat_display: Option<String>,
    pub pending_receive_sat: u64,
    pub pending_receive_display: String,
    pub pending_receive_fiat_display: Option<String>,
    pub pending_send_sat: u64,
    pub pending_send_display: String,
    pub pending_send_fiat_display: Option<String>,
    pub pending_refresh_sat: u64,
    pub pending_refresh_display: String,
    pub pending_refresh_fiat_display: Option<String>,
    pub last_sync: Option<String>,
}

#[derive(uniffi::Record, Clone, Debug)]
pub struct LightningAddressState {
    pub address: Option<String>,
    pub backing_ark_address: Option<String>,
}

#[derive(uniffi::Record, Clone, Debug)]
pub struct ReceiveState {
    pub method: ReceiveMethod,
    pub phase: ReceivePhase,
    pub ark_address: Option<String>,
    pub lightning_invoice: Option<String>,
    pub receive_request: Option<String>,
    pub lightning_payment_hash: Option<String>,
    pub lightning_status: String,
    pub lightning_status_display: String,
    pub lightning_paid: bool,
    pub amount_sat: u64,
    pub amount_display: String,
    pub memo: String,
}

#[derive(uniffi::Enum, Clone, Debug, PartialEq)]
pub enum ReceiveMethod {
    Lightning,
    Ark,
}

#[derive(uniffi::Enum, Clone, Debug, PartialEq)]
pub enum ReceivePhase {
    Editing,
    Creating,
    ShowingRequest,
    Success,
}

#[derive(uniffi::Record, Clone, Debug)]
pub struct SendState {
    pub destination: String,
    pub destination_kind: SendDestinationKind,
    pub phase: SendPhase,
    pub search_query: String,
    pub search_results: Vec<Contact>,
    pub global_search_results: Vec<Contact>,
    pub can_continue_search: bool,
    pub selected_contact_id: Option<String>,
    pub zap_enabled: bool,
    pub zap_available: bool,
    pub amount_sat: u64,
    pub amount_display: String,
    pub estimating_fee: bool,
    pub fee_estimate_sat: Option<u64>,
    pub fee_estimate_display: Option<String>,
    pub fee_estimate_fiat_display: Option<String>,
    pub total_cost_sat: Option<u64>,
    pub total_cost_display: Option<String>,
    pub total_cost_fiat_display: Option<String>,
    pub fee_estimate_error: Option<String>,
    pub memo: String,
    pub last_result: Option<String>,
    pub success_amount_display: String,
    pub can_submit: bool,
    pub error_text: Option<String>,
}

#[derive(uniffi::Enum, Clone, Debug, PartialEq)]
pub enum SendPhase {
    Drafting,
    Editing,
    Sending,
    Success,
}

#[derive(uniffi::Enum, Clone, Debug, PartialEq)]
pub enum SendDestinationKind {
    Unknown,
    Lightning,
    OnChain,
    Ark,
}

#[derive(uniffi::Record, Clone, Debug, Serialize, Deserialize)]
pub struct NostrState {
    pub npub: Option<String>,
    pub name: String,
    pub about: String,
    /// Remote profile picture URL from Nostr metadata. This is the value that
    /// gets republished in kind-0 metadata.
    pub picture: String,
    /// Render-ready profile picture URL. Rust may point this at a normalized
    /// cached `file://` image while keeping `picture` as the remote source.
    #[serde(default)]
    pub picture_display_url: String,
    pub lud16: String,
    pub nip05: String,
    #[serde(default)]
    pub deleted: bool,
    pub contacts: Vec<Contact>,
}

#[derive(uniffi::Record, Clone, Debug, Serialize, Deserialize)]
pub struct Contact {
    pub id: String,
    pub npub: String,
    pub name: String,
    pub followed: bool,
    pub picture: String,
    pub lightning_address: String,
    pub lnurl: String,
    pub last_used: u64,
}

#[derive(uniffi::Record, Clone, Debug)]
pub struct NostrMessage {
    pub id: String,
    pub contact_id: String,
    pub body: String,
    pub inbound: bool,
    pub timestamp: String,
}

#[derive(uniffi::Record, Clone, Debug)]
pub struct ActivityItem {
    pub id: String,
    pub title: String,
    pub subtitle: String,
    pub display_primary_name: String,
    pub display_verb: String,
    pub display_secondary_name: String,
    pub message_text: Option<String>,
    pub method_icon: String,
    pub method_display: String,
    pub amount_sat: i64,
    pub payment_amount_sat: i64,
    pub amount_display: String,
    pub amount_fiat_display: Option<String>,
    pub signed_amount_display: String,
    pub icon_kind: ActivityIconKind,
    pub status: String,
    pub timestamp: String,
    pub completed_at_unix: u64,
    pub counterparty: Option<Contact>,
    pub ark_address: Option<String>,
    pub lightning_invoice: Option<String>,
    pub lightning_payment_hash: Option<String>,
    pub lightning_payment_preimage: Option<String>,
}

#[derive(uniffi::Enum, Clone, Debug, PartialEq)]
pub enum ActivityIconKind {
    Sent,
    Received,
}

impl AppState {
    pub(crate) fn initial() -> Self {
        Self {
            rev: 0,
            show_launch_splash: true,
            router: Router {
                default_screen: Screen::Setup,
                screen_stack: vec![],
                selected_tab: MainTab::Home,
            },
            setup: SetupState::NeedsSetup,
            wallet: WalletState {
                network: WalletNetwork::Signet,
                network_name: WalletNetwork::Signet.display_name().to_string(),
                default_server_address: WalletNetwork::Signet.server_address().to_string(),
                default_esplora_address: WalletNetwork::Signet.esplora_address().to_string(),
                server_address: WalletNetwork::Signet.server_address().to_string(),
                esplora_address: WalletNetwork::Signet.esplora_address().to_string(),
                price_currency: PriceCurrency::BTC,
                price_currency_code: PriceCurrency::BTC.code().to_string(),
                price_currency_name: PriceCurrency::BTC.display_name().to_string(),
                btc_price: None,
                balance_sat: 0,
                balance_display: format_sats(0),
                balance_fiat_display: None,
                pending_receive_sat: 0,
                pending_receive_display: format_sats(0),
                pending_receive_fiat_display: None,
                pending_send_sat: 0,
                pending_send_display: format_sats(0),
                pending_send_fiat_display: None,
                pending_refresh_sat: 0,
                pending_refresh_display: format_sats(0),
                pending_refresh_fiat_display: None,
                last_sync: None,
            },
            receive: ReceiveState {
                method: ReceiveMethod::Lightning,
                phase: ReceivePhase::Editing,
                ark_address: None,
                lightning_invoice: None,
                receive_request: None,
                lightning_payment_hash: None,
                lightning_status: "idle".to_string(),
                lightning_status_display: "Waiting".to_string(),
                lightning_paid: false,
                amount_sat: 10_000,
                amount_display: format_sats(10_000),
                memo: "Rebel Wallet".to_string(),
            },
            send: SendState {
                destination: String::new(),
                destination_kind: SendDestinationKind::Unknown,
                phase: SendPhase::Drafting,
                search_query: String::new(),
                search_results: vec![],
                global_search_results: vec![],
                can_continue_search: false,
                selected_contact_id: None,
                zap_enabled: false,
                zap_available: false,
                amount_sat: 0,
                amount_display: format_sats(0),
                estimating_fee: false,
                fee_estimate_sat: None,
                fee_estimate_display: None,
                fee_estimate_fiat_display: None,
                total_cost_sat: None,
                total_cost_display: None,
                total_cost_fiat_display: None,
                fee_estimate_error: None,
                memo: String::new(),
                last_result: None,
                success_amount_display: format_sats(0),
                can_submit: false,
                error_text: None,
            },
            lightning_address: LightningAddressState {
                address: None,
                backing_ark_address: None,
            },
            nostr: NostrState {
                npub: None,
                name: "Rebel".to_string(),
                about: String::new(),
                picture: String::new(),
                picture_display_url: String::new(),
                lud16: String::new(),
                nip05: String::new(),
                deleted: false,
                contacts: vec![],
            },
            supported_networks: supported_networks(),
            supported_price_currencies: supported_price_currencies(),
            direct_messages: vec![],
            activity: vec![],
            recovery_phrase: None,
            toast: None,
            busy: BusyState::default(),
            capability_request: None,
        }
    }

    pub(crate) fn refresh_derived(&mut self) {
        self.show_launch_splash = should_show_launch_splash(self);
        self.supported_networks = supported_networks();
        self.wallet.network_name = self.wallet.network.display_name().to_string();
        self.wallet.default_server_address = self.wallet.network.server_address().to_string();
        self.wallet.default_esplora_address = self.wallet.network.esplora_address().to_string();
        self.supported_price_currencies = supported_price_currencies();
        self.wallet.price_currency_code = self.wallet.price_currency.code().to_string();
        self.wallet.price_currency_name = self.wallet.price_currency.display_name().to_string();
        self.wallet.balance_display = format_sats(self.wallet.balance_sat);
        self.wallet.pending_receive_display = format_sats(self.wallet.pending_receive_sat);
        self.wallet.pending_send_display = format_sats(self.wallet.pending_send_sat);
        self.wallet.pending_refresh_display = format_sats(self.wallet.pending_refresh_sat);
        self.wallet.balance_fiat_display = format_fiat_sats(
            self.wallet.balance_sat,
            self.wallet.btc_price,
            &self.wallet.price_currency,
        );
        self.wallet.pending_receive_fiat_display = format_fiat_sats(
            self.wallet.pending_receive_sat,
            self.wallet.btc_price,
            &self.wallet.price_currency,
        );
        self.wallet.pending_send_fiat_display = format_fiat_sats(
            self.wallet.pending_send_sat,
            self.wallet.btc_price,
            &self.wallet.price_currency,
        );
        self.wallet.pending_refresh_fiat_display = format_fiat_sats(
            self.wallet.pending_refresh_sat,
            self.wallet.btc_price,
            &self.wallet.price_currency,
        );
        for item in &mut self.activity {
            item.amount_display = format_sats(item.amount_sat.unsigned_abs());
            item.amount_fiat_display = format_fiat_sats(
                item.amount_sat.unsigned_abs(),
                self.wallet.btc_price,
                &self.wallet.price_currency,
            );
            item.signed_amount_display = format_signed_sats(item.amount_sat, true);
        }

        self.receive.amount_display = format_sats(self.receive.amount_sat);
        self.receive.lightning_status_display = if self.receive.lightning_paid {
            "Paid".to_string()
        } else {
            match self.receive.lightning_status.as_str() {
                "claiming" => "Claiming".to_string(),
                "claimable" => "Claimable".to_string(),
                "paid" => "Paid".to_string(),
                _ => "Waiting".to_string(),
            }
        };

        self.send.amount_display = format_sats(self.send.amount_sat);
        self.send.fee_estimate_display = self.send.fee_estimate_sat.map(format_sats);
        self.send.total_cost_display = self.send.total_cost_sat.map(format_sats);
        self.send.fee_estimate_fiat_display = self.send.fee_estimate_sat.and_then(|amount| {
            format_non_btc_fiat_sats(amount, self.wallet.btc_price, &self.wallet.price_currency)
        });
        self.send.total_cost_fiat_display = self.send.total_cost_sat.and_then(|amount| {
            format_non_btc_fiat_sats(amount, self.wallet.btc_price, &self.wallet.price_currency)
        });
        self.send.search_results = send_search_results(
            &self.send.search_query,
            &self.nostr.contacts,
            &self.send.global_search_results,
            self.nostr.npub.as_deref(),
        );
        self.send.can_continue_search = is_sendable_search_query(&self.send.search_query);
        if self.send.destination.trim().is_empty() && self.send.phase == SendPhase::Editing {
            self.send.phase = SendPhase::Drafting;
        }
        self.send.destination_kind = send_destination_kind(&self.send.destination);
        self.send.error_text = send_error_text(
            self.send.destination_kind.clone(),
            self.send.amount_sat,
            self.send.total_cost_sat,
            self.wallet.balance_sat,
        );
        self.send.can_submit = !self.send.destination.trim().is_empty()
            && self.send.phase != SendPhase::Sending
            && !self.send.estimating_fee
            && self.send.error_text.is_none()
            && match self.send.destination_kind {
                SendDestinationKind::Lightning => true,
                SendDestinationKind::Ark | SendDestinationKind::OnChain => self.send.amount_sat > 0,
                SendDestinationKind::Unknown => false,
            };
        if !self.send.zap_available {
            self.send.zap_enabled = false;
        }

        self.lightning_address.address = self
            .lightning_address
            .backing_ark_address
            .as_ref()
            .filter(|address| !address.trim().is_empty())
            .map(|address| arkzap_lightning_address(address));
    }

    pub(crate) fn reset_receive_draft(&mut self) {
        self.receive.method = ReceiveMethod::Lightning;
        self.receive.phase = ReceivePhase::Editing;
        self.receive.ark_address = None;
        self.receive.lightning_invoice = None;
        self.receive.receive_request = None;
        self.receive.lightning_payment_hash = None;
        self.receive.lightning_status = "idle".to_string();
        self.receive.lightning_paid = false;
        self.receive.amount_sat = 0;
    }
}

pub(crate) fn arkzap_lightning_address(ark_address: &str) -> String {
    let ark_address = ark_address.trim();
    let domain = if ark_address.starts_with("tark") {
        "signet.arkzap.me"
    } else {
        "arkzap.me"
    };
    format!("{ark_address}@{domain}")
}

fn supported_price_currencies() -> Vec<CurrencyOption> {
    [
        PriceCurrency::BTC,
        PriceCurrency::USD,
        PriceCurrency::EUR,
        PriceCurrency::GBP,
    ]
    .into_iter()
    .map(|currency| CurrencyOption {
        code: currency.code().to_string(),
        name: currency.display_name().to_string(),
        currency,
    })
    .collect()
}

fn supported_networks() -> Vec<NetworkOption> {
    [WalletNetwork::Signet, WalletNetwork::Mainnet]
        .into_iter()
        .map(|network| NetworkOption {
            name: network.display_name().to_string(),
            caption: network.caption().to_string(),
            network,
        })
        .collect()
}

fn should_show_launch_splash(state: &AppState) -> bool {
    if state.rev == 0 {
        return true;
    }
    if state.busy.bootstrapping || state.busy.opening_wallet {
        return true;
    }
    state.setup == SetupState::Ready
        && state.busy.syncing_wallet
        && state.wallet.last_sync.is_none()
}

pub(crate) fn format_sats(amount: u64) -> String {
    format!("{} sats", grouped_digits(amount))
}

fn format_fiat_sats(
    amount_sat: u64,
    btc_price: Option<f64>,
    currency: &PriceCurrency,
) -> Option<String> {
    let price = btc_price?;
    let value = amount_sat as f64 / 100_000_000.0 * price;
    Some(format_fiat(value, currency))
}

fn format_non_btc_fiat_sats(
    amount_sat: u64,
    btc_price: Option<f64>,
    currency: &PriceCurrency,
) -> Option<String> {
    if *currency == PriceCurrency::BTC {
        return None;
    }
    format_fiat_sats(amount_sat, btc_price, currency)
}

fn format_fiat(value: f64, currency: &PriceCurrency) -> String {
    let max_fraction_digits = currency.max_fractional_digits();
    let number = if value == 0.0 {
        "0".to_string()
    } else {
        format!("{value:.max_fraction_digits$}")
    };
    format!(
        "{}{}{} {}",
        currency.approximate(),
        currency.symbol(),
        number,
        currency.code()
    )
}

pub(crate) fn format_signed_sats(amount: i64, signed: bool) -> String {
    let magnitude = amount.unsigned_abs();
    let prefix = if amount < 0 {
        "-"
    } else if signed && amount > 0 {
        "+"
    } else {
        ""
    };
    format!("{prefix}{} sats", grouped_digits(magnitude))
}

pub(crate) fn send_destination_kind(destination: &str) -> SendDestinationKind {
    let destination = destination.trim();
    let lower = destination.to_ascii_lowercase();
    if lower.is_empty() {
        SendDestinationKind::Unknown
    } else if lower.starts_with("lightning:")
        || lower.starts_with("ln")
        || is_valid_lightning_address(destination)
    {
        SendDestinationKind::Lightning
    } else if BitcoinAddress::from_str(destination).is_ok() {
        SendDestinationKind::OnChain
    } else if ArkAddress::from_str(destination).is_ok() {
        SendDestinationKind::Ark
    } else {
        SendDestinationKind::Unknown
    }
}

fn send_search_results(
    query: &str,
    contacts: &[Contact],
    global_results: &[Contact],
    own_npub: Option<&str>,
) -> Vec<Contact> {
    let needle = normalize_search(query);
    let own_npub = own_npub.map(normalize_search);
    let mut contacts = contacts
        .iter()
        .cloned()
        .chain(global_results.iter().cloned())
        .map(|mut contact| {
            contact.name = contact.name.trim().to_string();
            contact
        })
        .fold(Vec::<Contact>::new(), |mut out, contact| {
            if !out.iter().any(|c| c.npub == contact.npub) {
                out.push(contact);
            }
            out
        });
    contacts.sort_by(|a, b| {
        contact_has_lightning_address(b)
            .cmp(&contact_has_lightning_address(a))
            .then_with(|| normalize_search(&a.name).cmp(&normalize_search(&b.name)))
            .then_with(|| normalize_search(&a.npub).cmp(&normalize_search(&b.npub)))
            .then_with(|| a.id.cmp(&b.id))
    });

    contacts
        .into_iter()
        .filter(|contact| {
            if let Some(own_npub) = &own_npub {
                if normalize_search(&contact.npub) == *own_npub {
                    return false;
                }
            }
            contact_has_lightning_address(contact)
        })
        .filter(|contact| {
            needle.is_empty()
                || normalize_search(&contact.name).contains(&needle)
                || normalize_search(&contact.npub).contains(&needle)
                || normalize_search(&contact.lightning_address).contains(&needle)
                || normalize_search(&contact.lnurl).contains(&needle)
        })
        .collect()
}

pub(crate) fn sort_contacts_by_name_npub(contacts: &mut [Contact]) {
    contacts.sort_by(|a, b| {
        normalize_search(&a.name)
            .cmp(&normalize_search(&b.name))
            .then_with(|| normalize_search(&a.npub).cmp(&normalize_search(&b.npub)))
            .then_with(|| a.id.cmp(&b.id))
    });
}

fn contact_has_lightning_address(contact: &Contact) -> bool {
    is_valid_lightning_address(&contact.lightning_address)
}

fn is_valid_lightning_address(address: &str) -> bool {
    let address = address.trim();
    let Some((local, domain)) = address.split_once('@') else {
        return false;
    };
    if local.is_empty() || domain.is_empty() || domain.contains('@') {
        return false;
    }
    if !local
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '+'))
    {
        return false;
    }
    let domain = domain.to_ascii_lowercase();
    if !domain.contains('.') || domain.starts_with('.') || domain.ends_with('.') {
        return false;
    }
    domain.split('.').all(|label| {
        !label.is_empty()
            && !label.starts_with('-')
            && !label.ends_with('-')
            && label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
    })
}

fn is_sendable_search_query(query: &str) -> bool {
    let trimmed = query.trim();
    let lower = trimmed.to_ascii_lowercase();
    trimmed.len() >= 6
        && (ArkAddress::from_str(trimmed).is_ok()
            || BitcoinAddress::from_str(trimmed).is_ok()
            || lower.starts_with("lightning:")
            || lower.starts_with("lnbc")
            || lower.starts_with("lntb")
            || lower.starts_with("lnurl")
            || trimmed.contains('@')
            || lower.starts_with("http://")
            || lower.starts_with("https://"))
}

fn normalize_search(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}

fn send_error_text(
    destination_kind: SendDestinationKind,
    amount_sat: u64,
    total_cost_sat: Option<u64>,
    balance_sat: u64,
) -> Option<String> {
    if total_cost_sat.unwrap_or(amount_sat) > balance_sat {
        return Some("Insufficient balance for this send.".to_string());
    }
    if matches!(
        destination_kind,
        SendDestinationKind::Ark | SendDestinationKind::OnChain
    ) && amount_sat == 0
    {
        return Some(format!(
            "Enter an amount before sending to {}.",
            match destination_kind {
                SendDestinationKind::OnChain => "an on-chain address",
                _ => "an Ark address",
            }
        ));
    }
    None
}

fn grouped_digits(amount: u64) -> String {
    let digits = amount.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    let first_group_len = digits.len() % 3;

    for (idx, ch) in digits.chars().enumerate() {
        if idx > 0
            && (idx == first_group_len
                || (idx > first_group_len && (idx - first_group_len) % 3 == 0))
        {
            out.push(',');
        }
        out.push(ch);
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_unsigned_and_signed_sats() {
        assert_eq!(format_sats(0), "0 sats");
        assert_eq!(format_sats(1_234_567), "1,234,567 sats");
        assert_eq!(format_signed_sats(42, true), "+42 sats");
        assert_eq!(format_signed_sats(-42, true), "-42 sats");
        assert_eq!(format_signed_sats(42, false), "42 sats");
    }

    #[test]
    fn derives_send_destination_kind_and_validation() {
        const ARK_ADDR: &str = "tark1pwh9vsmezqqpharv69q4z8m6x364d5m5prnmcalcalq9pdmzw0y7mpveck4pcfhezqypczkrrj3lkx5ue4qrf4jc7ztpt9htdttmh2judhqnu7aue8p0y9mq47jn9z";

        let mut state = AppState::initial();
        state.wallet.balance_sat = 1_000;

        state.send.destination = "lightning:lnbc1example".to_string();
        state.refresh_derived();
        assert_eq!(state.send.destination_kind, SendDestinationKind::Lightning);
        assert!(state.send.can_submit);
        assert_eq!(state.send.error_text, None);

        state.send.destination = ARK_ADDR.to_string();
        state.send.search_query = ARK_ADDR.to_string();
        state.send.amount_sat = 0;
        state.refresh_derived();
        assert_eq!(state.send.destination_kind, SendDestinationKind::Ark);
        assert!(!state.send.can_submit);
        assert!(state.send.can_continue_search);
        assert_eq!(
            state.send.error_text.as_deref(),
            Some("Enter an amount before sending to an Ark address.")
        );

        state.send.amount_sat = 2_000;
        state.refresh_derived();
        assert!(!state.send.can_submit);
        assert_eq!(
            state.send.error_text.as_deref(),
            Some("Insufficient balance for this send.")
        );

        state.send.destination = "ark1example".to_string();
        state.send.search_query = "ark1example".to_string();
        state.send.amount_sat = 1;
        state.send.total_cost_sat = None;
        state.refresh_derived();
        assert_eq!(state.send.destination_kind, SendDestinationKind::Unknown);
        assert!(!state.send.can_submit);
        assert!(!state.send.can_continue_search);

        state.send.destination = " Alice@Example.com ".to_string();
        state.send.amount_sat = 1;
        state.refresh_derived();
        assert_eq!(state.send.destination_kind, SendDestinationKind::Lightning);
        assert!(state.send.can_submit);

        let onchain_address = "bc1qrrz8r05xuyjh667a2nfgvh96d5x47aug0prxwm";
        state.send.destination = onchain_address.to_string();
        state.send.search_query = onchain_address.to_string();
        state.send.amount_sat = 0;
        state.refresh_derived();
        assert_eq!(state.send.destination_kind, SendDestinationKind::OnChain);
        assert!(!state.send.can_submit);
        assert!(state.send.can_continue_search);
        assert_eq!(
            state.send.error_text.as_deref(),
            Some("Enter an amount before sending to an on-chain address.")
        );

        state.send.amount_sat = 1;
        state.refresh_derived();
        assert!(state.send.can_submit);

        state.send.amount_sat = 900;
        state.send.total_cost_sat = Some(1_001);
        state.refresh_derived();
        assert!(!state.send.can_submit);
        assert_eq!(
            state.send.error_text.as_deref(),
            Some("Insufficient balance for this send.")
        );
    }

    #[test]
    fn hides_zap_when_not_available() {
        let mut state = AppState::initial();
        state.send.destination = "lnbc1example".to_string();
        state.send.zap_available = false;
        state.send.zap_enabled = true;

        state.refresh_derived();

        assert!(!state.send.zap_available);
        assert!(!state.send.zap_enabled);
    }

    #[test]
    fn derives_receive_status_display() {
        let mut state = AppState::initial();
        state.receive.lightning_status = "claimable".to_string();
        state.refresh_derived();
        assert_eq!(state.receive.lightning_status_display, "Claimable");

        state.receive.lightning_paid = true;
        state.refresh_derived();
        assert_eq!(state.receive.lightning_status_display, "Paid");
    }

    #[test]
    fn preserves_generated_receive_request() {
        let mut state = AppState::initial();
        state.receive.receive_request =
            Some("bitcoin:?amount=0.0005&ark=tark1fdafa&lightning=lnbc1example".to_string());
        state.receive.ark_address = Some("tark1fdafa".to_string());
        state.receive.lightning_invoice = Some("lnbc1example".to_string());
        state.refresh_derived();
        assert_eq!(
            state.receive.receive_request.as_deref(),
            Some("bitcoin:?amount=0.0005&ark=tark1fdafa&lightning=lnbc1example")
        );
    }

    #[test]
    fn reset_receive_draft_restores_default_method() {
        let mut state = AppState::initial();
        state.receive.method = ReceiveMethod::Ark;
        state.receive.phase = ReceivePhase::ShowingRequest;
        state.receive.ark_address = Some("tark1fdafa".to_string());
        state.receive.receive_request = Some("tark1fdafa".to_string());
        state.receive.amount_sat = 50_000;

        state.reset_receive_draft();

        assert_eq!(state.receive.method, ReceiveMethod::Lightning);
        assert_eq!(state.receive.phase, ReceivePhase::Editing);
        assert_eq!(state.receive.ark_address, None);
        assert_eq!(state.receive.receive_request, None);
        assert_eq!(state.receive.amount_sat, 0);
    }

    #[test]
    fn derives_platform_render_metadata() {
        let mut state = AppState::initial();
        state.rev = 1;
        state.busy.bootstrapping = false;
        state.busy.opening_wallet = false;
        state.refresh_derived();

        assert!(!state.show_launch_splash);
        assert_eq!(state.wallet.price_currency_code, "BTC");
        assert_eq!(state.wallet.price_currency_name, "Bitcoin");
        assert_eq!(state.supported_price_currencies.len(), 4);

        state.setup = SetupState::Ready;
        state.busy.syncing_wallet = true;
        state.wallet.last_sync = None;
        state.refresh_derived();
        assert!(state.show_launch_splash);
    }

    #[test]
    fn derives_network_profiles() {
        let mut state = AppState::initial();
        state.refresh_derived();

        assert_eq!(state.supported_networks.len(), 2);
        assert!(state
            .supported_networks
            .iter()
            .any(|network| network.network == WalletNetwork::Signet));
        assert!(state
            .supported_networks
            .iter()
            .any(|network| network.network == WalletNetwork::Mainnet));
        assert_eq!(
            WalletNetwork::Signet.db_file_name(),
            "rebel-wallet-signet.sqlite"
        );
        assert_eq!(
            WalletNetwork::Mainnet.db_file_name(),
            "rebel-wallet-mainnet.sqlite"
        );
        assert_eq!(WalletNetwork::Signet.server_access_token(), None);
        assert_eq!(WalletNetwork::Mainnet.server_access_token(), None);
    }

    #[test]
    fn derives_activity_fiat_displays_from_selected_currency() {
        let mut state = AppState::initial();
        state.wallet.price_currency = PriceCurrency::USD;
        state.wallet.btc_price = Some(100_000.0);
        state.activity = vec![ActivityItem {
            id: "activity-1".to_string(),
            title: String::new(),
            subtitle: String::new(),
            display_primary_name: "You".to_string(),
            display_verb: "sent".to_string(),
            display_secondary_name: "Alice".to_string(),
            message_text: None,
            method_icon: "bolt.fill".to_string(),
            method_display: "Lightning".to_string(),
            amount_sat: -50_000,
            payment_amount_sat: -50_000,
            amount_display: String::new(),
            amount_fiat_display: None,
            signed_amount_display: String::new(),
            icon_kind: ActivityIconKind::Sent,
            status: "complete".to_string(),
            timestamp: String::new(),
            completed_at_unix: 0,
            counterparty: None,
            ark_address: None,
            lightning_invoice: None,
            lightning_payment_hash: None,
            lightning_payment_preimage: None,
        }];

        state.refresh_derived();

        let item = &state.activity[0];
        assert_eq!(item.amount_display, "50,000 sats");
        assert_eq!(item.amount_fiat_display.as_deref(), Some("$50.00 USD"));
        assert_eq!(item.signed_amount_display, "-50,000 sats");
    }

    #[test]
    fn derives_send_search_results_from_contacts() {
        let mut state = AppState::initial();
        state.nostr.contacts = vec![
            Contact {
                id: "1".to_string(),
                npub: "npubalice".to_string(),
                name: "Alice".to_string(),
                followed: true,
                picture: String::new(),
                lightning_address: "alice@example.com".to_string(),
                lnurl: String::new(),
                last_used: 10,
            },
            Contact {
                id: "2".to_string(),
                npub: "npubbob".to_string(),
                name: "Bob".to_string(),
                followed: true,
                picture: String::new(),
                lightning_address: String::new(),
                lnurl: "lnurl1bob".to_string(),
                last_used: 20,
            },
            Contact {
                id: "4".to_string(),
                npub: "npubdave".to_string(),
                name: "dave".to_string(),
                followed: true,
                picture: String::new(),
                lightning_address: "not-a-lightning-address".to_string(),
                lnurl: String::new(),
                last_used: 30,
            },
        ];
        state.send.global_search_results = vec![Contact {
            id: "3".to_string(),
            npub: "npubcarol".to_string(),
            name: "Carol".to_string(),
            followed: false,
            picture: String::new(),
            lightning_address: "carol@example.com".to_string(),
            lnurl: String::new(),
            last_used: 0,
        }];
        state.send.search_query = "ali".to_string();
        state.refresh_derived();

        assert_eq!(state.send.search_results.len(), 1);
        assert_eq!(state.send.search_results[0].name, "Alice");
        assert!(!state.send.can_continue_search);

        state.send.search_query = "alice@example.com".to_string();
        state.refresh_derived();
        assert!(state.send.can_continue_search);

        state.send.search_query = "car".to_string();
        state.refresh_derived();
        assert_eq!(state.send.search_results.len(), 1);
        assert_eq!(state.send.search_results[0].name, "Carol");

        state.send.search_query = "bob".to_string();
        state.refresh_derived();
        assert!(state.send.search_results.is_empty());

        state.send.search_query = "dave".to_string();
        state.refresh_derived();
        assert!(state.send.search_results.is_empty());
    }

    #[test]
    fn send_search_results_include_full_contact_list() {
        let mut state = AppState::initial();
        state.nostr.contacts = (0..75)
            .map(|idx| Contact {
                id: format!("contact-{idx}"),
                npub: format!("npub{idx}"),
                name: format!("Contact {idx}"),
                followed: true,
                picture: String::new(),
                lightning_address: format!("contact{idx}@example.com"),
                lnurl: String::new(),
                last_used: idx,
            })
            .collect();

        state.refresh_derived();

        assert_eq!(state.send.search_results.len(), 75);
    }

    #[test]
    fn send_search_results_sort_by_npub_not_mutable_profile_fields() {
        let mut state = AppState::initial();
        state.nostr.contacts = vec![
            Contact {
                id: "1".to_string(),
                npub: "npub-b".to_string(),
                name: "bravo".to_string(),
                followed: true,
                picture: String::new(),
                lightning_address: "bravo@example.com".to_string(),
                lnurl: String::new(),
                last_used: 100,
            },
            Contact {
                id: "2".to_string(),
                npub: "npub-a".to_string(),
                name: "Alpha".to_string(),
                followed: true,
                picture: String::new(),
                lightning_address: "alpha@example.com".to_string(),
                lnurl: String::new(),
                last_used: 0,
            },
        ];

        state.refresh_derived();

        assert_eq!(state.send.search_results[0].npub, "npub-a");
        assert_eq!(state.send.search_results[1].npub, "npub-b");
    }

    #[test]
    fn send_search_results_trim_names_and_finalize_sort_by_npub() {
        let mut state = AppState::initial();
        state.nostr.contacts = vec![
            Contact {
                id: "1".to_string(),
                npub: "npub-b".to_string(),
                name: "  Same  ".to_string(),
                followed: true,
                picture: String::new(),
                lightning_address: "b@example.com".to_string(),
                lnurl: String::new(),
                last_used: 0,
            },
            Contact {
                id: "2".to_string(),
                npub: "npub-a".to_string(),
                name: "same".to_string(),
                followed: true,
                picture: String::new(),
                lightning_address: "a@example.com".to_string(),
                lnurl: String::new(),
                last_used: 0,
            },
        ];

        state.refresh_derived();

        assert_eq!(state.send.search_results[0].npub, "npub-a");
        assert_eq!(state.send.search_results[0].name, "same");
        assert_eq!(state.send.search_results[1].npub, "npub-b");
        assert_eq!(state.send.search_results[1].name, "Same");
    }

    #[test]
    fn sorts_contacts_by_name_then_npub() {
        let mut state = AppState::initial();
        state.nostr.contacts = vec![
            Contact {
                id: "1".to_string(),
                npub: "npub-b".to_string(),
                name: "  Same  ".to_string(),
                followed: true,
                picture: String::new(),
                lightning_address: "b@example.com".to_string(),
                lnurl: String::new(),
                last_used: 0,
            },
            Contact {
                id: "2".to_string(),
                npub: "npub-c".to_string(),
                name: "alpha".to_string(),
                followed: true,
                picture: String::new(),
                lightning_address: "c@example.com".to_string(),
                lnurl: String::new(),
                last_used: 0,
            },
            Contact {
                id: "3".to_string(),
                npub: "npub-a".to_string(),
                name: "same".to_string(),
                followed: true,
                picture: String::new(),
                lightning_address: "a@example.com".to_string(),
                lnurl: String::new(),
                last_used: 0,
            },
        ];

        sort_contacts_by_name_npub(&mut state.nostr.contacts);

        assert_eq!(
            state
                .nostr
                .contacts
                .iter()
                .map(|contact| contact.npub.as_str())
                .collect::<Vec<_>>(),
            vec!["npub-c", "npub-a", "npub-b"]
        );
    }

    #[test]
    fn derives_arkzap_lightning_address() {
        let mut state = AppState::initial();
        state.nostr.lud16 = "saved@example.com".to_string();
        state.lightning_address.backing_ark_address = Some("tark1example".to_string());
        state.refresh_derived();
        assert_eq!(
            state.lightning_address.address.as_deref(),
            Some("tark1example@signet.arkzap.me")
        );
        assert_eq!(state.nostr.lud16, "saved@example.com");

        state.wallet.network = WalletNetwork::Mainnet;
        state.lightning_address.backing_ark_address = Some("ark1example".to_string());
        state.refresh_derived();
        assert_eq!(
            state.lightning_address.address.as_deref(),
            Some("ark1example@arkzap.me")
        );
        assert_eq!(state.nostr.lud16, "saved@example.com");
    }
}
