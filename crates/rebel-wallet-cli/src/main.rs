use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;

use rebel_wallet_core::{FfiApp, SecretStore};

fn main() {
    let data_dir = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("rebel-wallet-cli-smoke"));

    let app = FfiApp::new(
        data_dir.display().to_string(),
        Box::new(MemorySecretStore::default()),
    );
    let state = app.state();

    println!(
        "rebel-wallet core smoke: network={}, default_screen={:?}, contacts={}",
        state.wallet.network,
        state.router.default_screen,
        state.nostr.contacts.len()
    );
}

#[derive(Default)]
struct MemorySecretStore {
    values: Mutex<HashMap<String, String>>,
}

impl SecretStore for MemorySecretStore {
    fn get_secret(&self, key: String) -> Option<String> {
        self.values.lock().ok()?.get(&key).cloned()
    }

    fn set_secret(&self, key: String, value: String) -> bool {
        let Ok(mut values) = self.values.lock() else {
            return false;
        };
        values.insert(key, value);
        true
    }

    fn delete_secret(&self, key: String) -> bool {
        let Ok(mut values) = self.values.lock() else {
            return false;
        };
        values.remove(&key).is_some()
    }
}
