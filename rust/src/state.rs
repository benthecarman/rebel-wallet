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
    pub busy: bool,
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
    pub pending_receive_sat: u64,
    pub pending_send_sat: u64,
    pub last_sync: Option<String>,
}

#[derive(uniffi::Record, Clone, Debug)]
pub struct ReceiveState {
    pub ark_address: Option<String>,
    pub lightning_invoice: Option<String>,
    pub lightning_payment_hash: Option<String>,
    pub lightning_status: String,
    pub lightning_paid: bool,
    pub amount_sat: u64,
    pub memo: String,
}

#[derive(uniffi::Record, Clone, Debug)]
pub struct SendState {
    pub destination: String,
    pub amount_sat: u64,
    pub memo: String,
    pub last_result: Option<String>,
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
    pub amount_sat: i64,
    pub status: String,
    pub timestamp: String,
    pub counterparty_name: String,
    pub counterparty_picture: String,
    pub counterparty_known: bool,
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
                pending_receive_sat: 0,
                pending_send_sat: 0,
                last_sync: None,
            },
            receive: ReceiveState {
                ark_address: None,
                lightning_invoice: None,
                lightning_payment_hash: None,
                lightning_status: "idle".to_string(),
                lightning_paid: false,
                amount_sat: 10_000,
                memo: "Rebel Wallet".to_string(),
            },
            send: SendState {
                destination: String::new(),
                amount_sat: 0,
                memo: String::new(),
                last_result: None,
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
            busy: false,
        }
    }
}
