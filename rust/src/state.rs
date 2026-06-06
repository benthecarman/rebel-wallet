use serde::{Deserialize, Serialize};

use crate::SIGNET_SERVER;

#[derive(uniffi::Record, Clone, Debug)]
pub struct AppState {
    pub rev: u64,
    pub router: Router,
    pub setup: SetupState,
    pub wallet: WalletState,
    pub receive: ReceiveState,
    pub send: SendState,
    pub nostr: NostrState,
    pub direct_messages: Vec<NostrMessage>,
    pub activity: Vec<ActivityItem>,
    pub recovery_phrase: Option<String>,
    pub toast: Option<String>,
    pub busy: BusyState,
    pub capability_request: Option<CapabilityRequest>,
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
    ContactDetail { contact_id: String },
}

#[derive(uniffi::Enum, Clone, Debug)]
pub enum SetupState {
    NeedsSetup,
    Ready,
    Error { message: String },
}

#[derive(uniffi::Record, Clone, Debug)]
pub struct WalletState {
    pub network: String,
    pub server_address: String,
    pub balance_sat: u64,
    pub balance_display: String,
    pub pending_receive_sat: u64,
    pub pending_receive_display: String,
    pub pending_send_sat: u64,
    pub pending_send_display: String,
    pub last_sync: Option<String>,
}

#[derive(uniffi::Record, Clone, Debug)]
pub struct ReceiveState {
    pub method: ReceiveMethod,
    pub phase: ReceivePhase,
    pub ark_address: Option<String>,
    pub lightning_invoice: Option<String>,
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
    pub amount_sat: u64,
    pub amount_display: String,
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
    pub amount_sat: i64,
    pub amount_display: String,
    pub signed_amount_display: String,
    pub icon_kind: ActivityIconKind,
    pub status: String,
    pub timestamp: String,
    pub counterparty_name: String,
    pub counterparty_picture: String,
    pub counterparty_known: bool,
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
            router: Router {
                default_screen: Screen::Setup,
                screen_stack: vec![],
                selected_tab: MainTab::Home,
            },
            setup: SetupState::NeedsSetup,
            wallet: WalletState {
                network: "Signet".to_string(),
                server_address: SIGNET_SERVER.to_string(),
                balance_sat: 0,
                balance_display: format_sats(0),
                pending_receive_sat: 0,
                pending_receive_display: format_sats(0),
                pending_send_sat: 0,
                pending_send_display: format_sats(0),
                last_sync: None,
            },
            receive: ReceiveState {
                method: ReceiveMethod::Lightning,
                phase: ReceivePhase::Editing,
                ark_address: None,
                lightning_invoice: None,
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
                amount_sat: 0,
                amount_display: format_sats(0),
                memo: String::new(),
                last_result: None,
                success_amount_display: format_sats(0),
                can_submit: false,
                error_text: None,
            },
            nostr: NostrState {
                npub: None,
                name: "Rebel".to_string(),
                about: String::new(),
                picture: String::new(),
                lud16: String::new(),
                nip05: String::new(),
                contacts: vec![],
            },
            direct_messages: vec![],
            activity: vec![],
            recovery_phrase: None,
            toast: None,
            busy: BusyState::default(),
            capability_request: None,
        }
    }

    pub(crate) fn refresh_derived(&mut self) {
        self.wallet.balance_display = format_sats(self.wallet.balance_sat);
        self.wallet.pending_receive_display = format_sats(self.wallet.pending_receive_sat);
        self.wallet.pending_send_display = format_sats(self.wallet.pending_send_sat);

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
        if self.send.destination.trim().is_empty() && self.send.phase == SendPhase::Editing {
            self.send.phase = SendPhase::Drafting;
        }
        self.send.destination_kind = send_destination_kind(&self.send.destination);
        self.send.error_text = send_error_text(
            self.send.destination_kind.clone(),
            self.send.amount_sat,
            self.wallet.balance_sat,
        );
        self.send.can_submit = !self.send.destination.trim().is_empty()
            && self.send.phase != SendPhase::Sending
            && self.send.error_text.is_none()
            && (self.send.destination_kind == SendDestinationKind::Lightning
                || self.send.amount_sat > 0);
    }
}

pub(crate) fn format_sats(amount: u64) -> String {
    format!("{} sats", grouped_digits(amount))
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
    let lower = destination.trim().to_ascii_lowercase();
    if lower.is_empty() {
        SendDestinationKind::Unknown
    } else if lower.starts_with("lightning:") || lower.starts_with("ln") {
        SendDestinationKind::Lightning
    } else {
        SendDestinationKind::Ark
    }
}

fn send_error_text(
    destination_kind: SendDestinationKind,
    amount_sat: u64,
    balance_sat: u64,
) -> Option<String> {
    if amount_sat > balance_sat {
        return Some("Insufficient balance for this send.".to_string());
    }
    if destination_kind == SendDestinationKind::Ark && amount_sat == 0 {
        return Some("Enter an amount before sending to an Ark address.".to_string());
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
