use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};
use std::thread;

use flume::{Receiver, Sender};

mod actions;
mod activity;
mod core;
mod nostr_support;
mod payments;
mod persistence;
mod price;
mod profile_cache;
mod state;
mod time;
mod updates;
mod wallet;

pub use actions::AppAction;
pub use state::{
    ActivityIconKind, ActivityItem, AppState, BusyState, CapabilityRequest, CapabilityRequestKind,
    Contact, CurrencyOption, LightningAddressState, MainTab, NetworkOption, NostrMessage,
    NostrState, PriceCurrency, ReceiveMethod, ReceivePhase, ReceiveState, Router, Screen,
    SendDestinationKind, SendPhase, SendState, SetupState, WalletNetwork, WalletState,
};
pub use updates::AppUpdate;
pub(crate) use updates::{AsyncMsg, CoreMsg};

uniffi::setup_scaffolding!();

pub(crate) const SIGNET_SERVER: &str = "https://ark.signet.2nd.dev";
pub(crate) const SIGNET_ESPLORA: &str = "https://esplora.signet.2nd.dev";
pub(crate) const MAINNET_SERVER: &str = "https://ark.second.tech";
pub(crate) const MAINNET_ESPLORA: &str = "https://mempool.second.tech/api";
pub(crate) const MAINNET_SERVER_ACCESS_TOKEN: &str = "M8RIRSU3X5156RABFMJQKO6VYXK2N371";

#[uniffi::export(callback_interface)]
pub trait AppReconciler: Send + Sync + 'static {
    fn reconcile(&self, update: AppUpdate);
}

#[uniffi::export(callback_interface)]
pub trait SecretStore: Send + Sync + 'static {
    fn get_secret(&self, key: String) -> Option<String>;
    fn set_secret(&self, key: String, value: String) -> bool;
    fn delete_secret(&self, key: String) -> bool;
}

#[derive(uniffi::Object)]
pub struct FfiApp {
    core_tx: Sender<CoreMsg>,
    update_rx: Receiver<AppUpdate>,
    listening: AtomicBool,
    shared_state: Arc<RwLock<AppState>>,
}

#[uniffi::export]
impl FfiApp {
    #[uniffi::constructor]
    pub fn new(data_dir: String, secret_store: Box<dyn SecretStore>) -> Arc<Self> {
        let (update_tx, update_rx) = flume::unbounded();
        let (core_tx, core_rx) = flume::unbounded::<CoreMsg>();
        let shared_state = Arc::new(RwLock::new(AppState::initial()));
        let shared_for_core = shared_state.clone();
        let data_dir = PathBuf::from(data_dir);
        let secrets: Arc<dyn SecretStore> = Arc::from(secret_store);
        let tx_for_bootstrap = core_tx.clone();

        core::spawn_actor(
            data_dir,
            secrets,
            tx_for_bootstrap,
            core_rx,
            shared_for_core,
            update_tx,
        );

        Arc::new(Self {
            core_tx,
            update_rx,
            listening: AtomicBool::new(false),
            shared_state,
        })
    }

    pub fn state(&self) -> AppState {
        match self.shared_state.read() {
            Ok(g) => g.clone(),
            Err(poison) => poison.into_inner().clone(),
        }
    }

    pub fn dispatch(&self, action: AppAction) {
        let _ = self.core_tx.send(CoreMsg::Action(action));
    }

    pub fn listen_for_updates(&self, reconciler: Box<dyn AppReconciler>) {
        if self
            .listening
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return;
        }

        let rx = self.update_rx.clone();
        thread::spawn(move || {
            while let Ok(update) = rx.recv() {
                reconciler.reconcile(update);
            }
        });
    }
}
