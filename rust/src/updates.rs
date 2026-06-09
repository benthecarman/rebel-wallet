use bark::Wallet;

use crate::nostr_support::PrimalProfileContact;
use crate::{ActivityItem, AppAction, AppState, Contact, NostrMessage, NostrState, PriceCurrency};

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
        pending_refresh_sat: u64,
        maintenance_checked: bool,
        activity: Vec<ActivityItem>,
    },
    ArkAddress(String),
    ArkReceiveConfirmed {
        address: String,
        amount_sat: u64,
    },
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
    LightningAddressReady(String),
    SendFeeEstimated {
        destination: String,
        amount_sat: u64,
        fee_sat: u64,
        total_sat: u64,
    },
    SendFeeEstimateFailed {
        destination: String,
        amount_sat: u64,
        error: String,
    },
    Paid(String),
    Seed(String),
    NostrProfileLoaded(NostrState),
    NostrContactsLoaded(Vec<Contact>),
    PrimalContactsLoaded {
        records: Vec<PrimalProfileContact>,
        show_toast: bool,
    },
    NostrSearchLoaded {
        query: String,
        contacts: Vec<PrimalProfileContact>,
    },
    PrimalProfilesLoaded {
        records: Vec<PrimalProfileContact>,
    },
    PrimalProfilesFailed {
        pubkeys: Vec<String>,
    },
    ProfilePictureCached {
        pubkey: String,
        remote_url: String,
    },
    ProfilePictureCacheFailed {
        pubkey: String,
        remote_url: String,
    },
    NostrProfilePictureUploaded(String),
    NostrPublished(String),
    DirectMessagesLoaded(Vec<NostrMessage>),
    DirectMessageSent(NostrMessage),
    PriceUpdated {
        currency: PriceCurrency,
        price: f64,
    },
    PriceFailed,
    Error(String),
}
