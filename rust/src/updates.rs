use bark::Wallet;

use crate::{ActivityItem, AppAction, AppState, Contact, NostrMessage, NostrState};

#[derive(uniffi::Enum, Clone, Debug)]
pub enum AppUpdate {
    FullState(AppState),
}

pub(crate) enum CoreMsg {
    Action(AppAction),
    Async(AsyncMsg),
}

pub(crate) enum AsyncMsg {
    WalletReady {
        wallet: Wallet,
        mnemonic: String,
    },
    WalletSynced {
        balance_sat: u64,
        pending_receive_sat: u64,
        pending_send_sat: u64,
        activity: Vec<ActivityItem>,
    },
    ArkAddress(String),
    LightningInvoice {
        invoice: String,
        payment_hash: String,
    },
    LightningReceiveStatus {
        payment_hash: String,
        status: String,
        paid: bool,
    },
    LightningReceiveClaimed {
        payment_hash: String,
    },
    Paid(String),
    Seed(String),
    NostrProfileLoaded(NostrState),
    NostrContactsLoaded(Vec<Contact>),
    NostrProfilePictureUploaded(String),
    NostrPublished(String),
    DirectMessagesLoaded(Vec<NostrMessage>),
    DirectMessageSent(NostrMessage),
    Error(String),
}
