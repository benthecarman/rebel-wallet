use crate::{MainTab, PriceCurrency, ReceiveMethod, Screen};

#[derive(uniffi::Enum, Clone, Debug)]
pub enum AppAction {
    Bootstrap,
    CreateWallet,
    RestoreWallet {
        mnemonic: String,
    },
    ReplaceWallet {
        mnemonic: String,
    },
    ShowSeed,
    SyncWallet,
    MaintainVtxos,
    RefreshPrice,
    SetPriceCurrency {
        currency: PriceCurrency,
    },
    ConfigureServers {
        server_address: String,
        esplora_address: String,
        lnurl_server_address: String,
    },
    SelectTab {
        tab: MainTab,
    },
    PushScreen {
        screen: Screen,
    },
    PopScreen,
    UpdateScreenStack {
        stack: Vec<Screen>,
    },
    SelectReceiveMethod {
        method: ReceiveMethod,
    },
    SetReceiveAmount {
        amount_sat: u64,
    },
    SetReceiveMemo {
        memo: String,
    },
    EditReceiveRequest,
    BeginReceiveRequest,
    CreateArkAddress,
    CreateLightningInvoice,
    SetSendSearchQuery {
        query: String,
    },
    ContinueSendSearch,
    SelectSendContact {
        contact_id: String,
    },
    PrefetchProfilePictures {
        contact_ids: Vec<String>,
    },
    SetSendDestination {
        destination: String,
    },
    SetSendAmount {
        amount_sat: u64,
    },
    SetSendMemo {
        memo: String,
    },
    SetLightningAddressName {
        name: String,
    },
    RegisterLightningAddress,
    UseLightningAddressForNostr,
    PayDestination,
    PayLightningInvoice {
        invoice: String,
        amount_sat: Option<u64>,
    },
    PayArkAddress {
        address: String,
        amount_sat: u64,
    },
    DismissPaymentSuccess,
    ResetSendDraft,
    RequestQrScan,
    RequestClipboardRead,
    RequestPhotoPick,
    CompleteQrScan {
        value: Option<String>,
    },
    CompleteClipboardRead {
        value: Option<String>,
    },
    CompletePhotoPick {
        image_base64: Option<String>,
    },
    CancelCapabilityRequest,
    GenerateNostrKey,
    ImportNostrSecret {
        nsec_or_hex: String,
    },
    ExportNostrSecret,
    ClearNostrKey,
    EditNostrProfile {
        name: String,
        about: String,
        picture: String,
        lud16: String,
        nip05: String,
    },
    UploadNostrProfilePicture {
        image_base64: String,
    },
    AddContact {
        npub: String,
        name: String,
        lightning_address: String,
        lnurl: String,
        picture: String,
    },
    EditContact {
        contact_id: String,
        name: String,
        npub: String,
        lightning_address: String,
        lnurl: String,
        picture: String,
    },
    FollowContact {
        contact_id: String,
    },
    UnfollowContact {
        contact_id: String,
    },
    DeleteContact {
        contact_id: String,
    },
    PublishNostrProfile,
    RefreshNostrProfile,
    DeleteNostrProfile,
    PublishContactList,
    RefreshContactList,
    ClearNostrProfileCache,
    LoadDirectMessages {
        contact_id: String,
    },
    SendDirectMessage {
        contact_id: String,
        message: String,
    },
    ClearToast,
}
