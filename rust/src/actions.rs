use crate::{MainTab, Screen};

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
    SetReceiveAmount {
        amount_sat: u64,
    },
    SetReceiveMemo {
        memo: String,
    },
    CreateArkAddress,
    CreateLightningInvoice,
    SetSendDestination {
        destination: String,
    },
    SetSendAmount {
        amount_sat: u64,
    },
    SetSendMemo {
        memo: String,
    },
    PayDestination,
    PayLightningInvoice {
        invoice: String,
        amount_sat: Option<u64>,
    },
    PayArkAddress {
        address: String,
        amount_sat: u64,
    },
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
    LoadDirectMessages {
        contact_id: String,
    },
    SendDirectMessage {
        contact_id: String,
        message: String,
    },
    ClearToast,
}
