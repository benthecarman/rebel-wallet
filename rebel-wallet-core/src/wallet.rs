use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Context;
use bark::lock_manager::memory::MemoryLockManager;
use bark::persist::{sqlite::SqliteClient, BarkPersister};
use bark::{Config, OpenWalletArgs, Wallet, WalletSeed};
use bip39::Mnemonic;

use crate::persistence::ServerConfig;

const VTXO_REFRESH_EXPIRY_THRESHOLD_BLOCKS: u32 = 144;

/// Client identifier sent to the Ark server on every RPC (`x-user-agent`).
/// Format is `<name>/<version>`; the name must be lowercase ASCII.
const USER_AGENT: &str = concat!("rebel-wallet/", env!("CARGO_PKG_VERSION"));

#[derive(Clone, Copy, Debug)]
pub(crate) enum WalletOpenMode {
    Create,
    OpenOrCreate,
    Restore,
    Replace,
}

pub(crate) async fn open_bark_wallet(
    data_dir: PathBuf,
    mnemonic: &Mnemonic,
    mode: WalletOpenMode,
    server_config: ServerConfig,
) -> anyhow::Result<Wallet> {
    std::fs::create_dir_all(&data_dir)?;
    let network = server_config.network.bitcoin_network();
    let db_path = data_dir.join(server_config.network.db_file_name());
    if matches!(mode, WalletOpenMode::Replace) {
        remove_wallet_database_files(&db_path)?;
    }
    let db: Arc<dyn BarkPersister> = Arc::new(SqliteClient::open(&db_path)?);
    let config = Config {
        server_address: server_config.server_address,
        server_access_token: server_config.server_access_token,
        esplora_address: Some(server_config.esplora_address),
        vtxo_refresh_expiry_threshold: VTXO_REFRESH_EXPIRY_THRESHOLD_BLOCKS,
        user_agent: Some(USER_AGENT.to_string()),
        ..Config::network_default(network)
    };
    let lock_manager = Box::new(MemoryLockManager::new());
    let seed = WalletSeed::new_from_mnemonic(network, mnemonic);

    // Create/Replace force a fresh wallet (Replace already wiped the database
    // files above); OpenOrCreate/Restore open an existing wallet or create one.
    if matches!(mode, WalletOpenMode::Create | WalletOpenMode::Replace) {
        Wallet::create(network, &seed, &config, &*db, &*lock_manager, false).await?;
    }

    let args = OpenWalletArgs {
        run_daemon: false,
        persister: Some(db),
        lock_manager: Some(lock_manager),
        create_if_not_exists: true,
        create_without_server: false,
        ..Default::default()
    };
    Wallet::open(network, seed, config, args).await
}

pub(crate) fn remove_wallet_database_files(db_path: &Path) -> anyhow::Result<()> {
    for suffix in ["", "-wal", "-shm"] {
        let path = PathBuf::from(format!("{}{}", db_path.display(), suffix));
        match std::fs::remove_file(&path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => {
                return Err(e).with_context(|| format!("failed to remove {}", path.display()))
            }
        }
    }
    Ok(())
}
