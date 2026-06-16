use bark::Wallet;

use crate::nostr_support::FetchedProfileContact;
use crate::persistence::ZapReceiptRecord;
use crate::{
    ActivityItem, AppAction, AppState, NostrMessage, NostrState, PriceCurrency, SendDestinationKind,
};

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
    ReceiveRequest {
        uri: String,
        ark_address: String,
        lightning_invoice: String,
        payment_hash: String,
    },
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
    SendFeeEstimateDue {
        request_id: u64,
        destination: String,
        amount_sat: u64,
        estimate_amount_sat: u64,
        kind: SendDestinationKind,
    },
    SendFeeEstimated {
        request_id: u64,
        destination: String,
        amount_sat: u64,
        fee_sat: u64,
        total_sat: u64,
    },
    SendFeeEstimateFailed {
        request_id: u64,
        destination: String,
        amount_sat: u64,
        error: String,
    },
    Paid {
        result: String,
        annotation: Option<crate::persistence::PaymentAnnotation>,
    },
    ZapAvailabilityChecked {
        contact_id: String,
        available: bool,
    },
    ZapReceiptsLoaded {
        receipts: Vec<ZapReceiptRecord>,
        records: Vec<FetchedProfileContact>,
    },
    Seed(String),
    NostrProfileLoaded {
        nostr: NostrState,
        profile: Option<FetchedProfileContact>,
    },
    NostrContactsLoaded(Vec<FetchedProfileContact>),
    PrimalContactsLoaded {
        records: Vec<FetchedProfileContact>,
        show_toast: bool,
    },
    NostrSearchLoaded {
        query: String,
        contacts: Vec<FetchedProfileContact>,
    },
    PrimalProfilesLoaded {
        records: Vec<FetchedProfileContact>,
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
