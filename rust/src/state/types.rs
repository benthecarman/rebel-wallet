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

    pub(super) fn caption(&self) -> &'static str {
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

    pub(super) fn display_name(&self) -> &'static str {
        match self {
            Self::BTC => "Bitcoin",
            Self::USD => "US Dollar",
            Self::EUR => "Euro",
            Self::GBP => "British Pound",
        }
    }

    pub(super) fn symbol(&self) -> &'static str {
        match self {
            Self::BTC => "₿",
            Self::USD => "$",
            Self::EUR => "€",
            Self::GBP => "£",
        }
    }

    pub(super) fn max_fractional_digits(&self) -> usize {
        match self {
            Self::BTC => 8,
            Self::USD | Self::EUR | Self::GBP => 2,
        }
    }

    pub(super) fn approximate(&self) -> &'static str {
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
    Ark,
}

#[derive(uniffi::Record, Clone, Debug, Serialize, Deserialize)]
pub struct NostrState {
    pub npub: Option<String>,
    pub name: String,
    pub about: String,
    pub picture: String,
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
