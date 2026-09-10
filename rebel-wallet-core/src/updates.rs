use std::fmt;

use bark::Wallet;
use nwc_mobile::WakeDisposition;
use zeroize::Zeroizing;

use crate::nostr_support::FetchedProfileContact;
use crate::persistence::ZapReceiptRecord;
use crate::wallet::WalletRecoveryNotice;
use crate::{
    ActivityItem, AppAction, AppState, NostrMessage, NostrState, NwcWakeRequest, PriceCurrency,
    SendDestinationKind,
};

#[allow(clippy::large_enum_variant)]
#[derive(uniffi::Enum, Clone)]
pub enum AppUpdate {
    FullState(AppState),
    Haptic(HapticFeedback),
    OpenUrl {
        rev: u64,
        url: String,
    },
    NwcConnectionExportReady {
        rev: u64,
        connection_id: String,
        name: String,
        uri: String,
        copy_to_clipboard: bool,
        present_qr: bool,
    },
}

impl fmt::Debug for AppUpdate {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::FullState(_) => formatter
                .debug_tuple("FullState")
                .field(&"<redacted>")
                .finish(),
            Self::Haptic(feedback) => formatter.debug_tuple("Haptic").field(feedback).finish(),
            Self::OpenUrl { rev, .. } => formatter
                .debug_struct("OpenUrl")
                .field("rev", rev)
                .field("url", &"<redacted>")
                .finish(),
            Self::NwcConnectionExportReady {
                rev,
                connection_id,
                copy_to_clipboard,
                present_qr,
                ..
            } => formatter
                .debug_struct("NwcConnectionExportReady")
                .field("rev", rev)
                .field("connection_id", connection_id)
                .field("name", &"<redacted>")
                .field("uri", &"<redacted>")
                .field("copy_to_clipboard", copy_to_clipboard)
                .field("present_qr", present_qr)
                .finish(),
        }
    }
}

#[derive(uniffi::Enum, Clone, Debug, PartialEq, Eq)]
pub enum HapticFeedback {
    Selection,
    ImpactLight,
    ImpactMedium,
    NotificationSuccess,
    NotificationWarning,
    NotificationError,
}

#[allow(clippy::large_enum_variant)]
pub(crate) enum CoreMsg {
    Action(AppAction),
    Async(AsyncMsg),
}

pub(crate) struct WalletSnapshot {
    pub(crate) balance_sat: u64,
    pub(crate) pending_receive_sat: u64,
    pub(crate) stuck_receive_sat: u64,
    pub(crate) pending_send_sat: u64,
    /// Amount committed to a round funding transaction and no longer spendable.
    /// `None` preserves the last known value when Bark could not resolve every
    /// pending round's current server status.
    pub(crate) pending_refresh_sat: Option<u64>,
    /// Includes queued delegated rounds whose inputs are still spendable. This
    /// drives reconciliation polling but is intentionally not rendered as busy.
    pub(crate) has_pending_rounds: bool,
    pub(crate) activity: Vec<ActivityItem>,
}

#[allow(clippy::large_enum_variant)]
pub(crate) enum AsyncMsg {
    WalletDiagnosticsLoaded {
        generation: u64,
        report: String,
    },
    WalletReady {
        generation: u64,
        wallet: Wallet,
        mnemonic: Zeroizing<String>,
        recovery_notice: Option<WalletRecoveryNotice>,
    },
    WalletOpenFailed {
        generation: u64,
        message: String,
    },
    WalletWorkFinished {
        generation: u64,
        operation_id: u64,
        result: Result<WalletSnapshot, String>,
    },
    WalletRefreshPollDue {
        generation: u64,
        nonce: u64,
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
    LightningReceivesClaimed {
        payment_hashes: Vec<String>,
    },
    LightningAddressReady(String),
    LightningAddressRegistrationUpdated {
        name: String,
        lightning_address: String,
        payment_ark_address: String,
        invoice: Option<String>,
        purchase_id: Option<String>,
        amount_msats: Option<u64>,
        active: bool,
        paid: bool,
        paid_from_wallet: bool,
        requires_confirmation: bool,
        annotation: Option<crate::persistence::PaymentAnnotation>,
        warning: Option<String>,
    },
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
    ZapReceiptsLoaded {
        receipts: Vec<ZapReceiptRecord>,
        records: Vec<FetchedProfileContact>,
    },
    Seed(Zeroizing<String>),
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
    NwcIconCached {
        remote_url: String,
    },
    NwcIconCacheFailed {
        remote_url: String,
    },
    NostrProfilePictureUploaded(String),
    NostrPublished(String),
    DirectMessagesLoaded(Vec<NostrMessage>),
    DirectMessageSent(NostrMessage),
    NwcWakeEngineFinished {
        generation: u64,
        request: NwcWakeRequest,
        disposition: WakeDisposition,
    },
    NwcWakeRequestFailed {
        generation: u64,
        event_id: String,
        error: String,
    },
    NwcWakeRetryDue {
        generation: u64,
        event_id: String,
    },
    NwcInfoEventPublished {
        client_pubkey: String,
        relay: String,
    },
    NwcInfoEventFailed {
        client_pubkey: String,
        relay: String,
        error: String,
    },
    NwcPushRegistrationFinished {
        applied: usize,
        deferred: usize,
        next_attempt_at: Option<u64>,
        error: Option<String>,
    },
    NwcPushRetryDue {
        nonce: u64,
    },
    PriceUpdated {
        currency: PriceCurrency,
        price: f64,
    },
    PriceFailed,
    Error(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_update_debug_redacts_sensitive_urls() {
        let export = AppUpdate::NwcConnectionExportReady {
            rev: 1,
            connection_id: "connection".to_string(),
            name: "Private client".to_string(),
            uri: "nostr+walletconnect://secret".to_string(),
            copy_to_clipboard: false,
            present_qr: true,
        };
        let callback = AppUpdate::OpenUrl {
            rev: 2,
            url: "https://client.example/callback?secret=value".to_string(),
        };

        let export_debug = format!("{export:?}");
        assert!(!export_debug.contains("nostr+walletconnect"));
        assert!(!export_debug.contains("Private client"));
        assert!(export_debug.contains("<redacted>"));

        let callback_debug = format!("{callback:?}");
        assert!(!callback_debug.contains("client.example"));
        assert!(!callback_debug.contains("secret=value"));
        assert!(callback_debug.contains("<redacted>"));
    }
}
