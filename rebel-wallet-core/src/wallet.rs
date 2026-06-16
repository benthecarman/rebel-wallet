use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Context;
use bark::lock_manager::memory::MemoryLockManager;
use bark::persist::{sqlite::SqliteClient, BarkPersister};
use bark::{Config, Wallet};
use bip39::Mnemonic;

use crate::persistence::ServerConfig;

const VTXO_REFRESH_EXPIRY_THRESHOLD_BLOCKS: u32 = 144;

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
    let db = Arc::new(SqliteClient::open(&db_path)?);
    let config = Config {
        server_address: server_config.server_address,
        server_access_token: server_config.server_access_token,
        esplora_address: Some(server_config.esplora_address),
        vtxo_refresh_expiry_threshold: VTXO_REFRESH_EXPIRY_THRESHOLD_BLOCKS,
        ..Config::network_default(network)
    };
    let lock_manager = Box::new(MemoryLockManager::new());
    match mode {
        WalletOpenMode::Create | WalletOpenMode::Replace => {
            Wallet::create(mnemonic, network, config, db, lock_manager, false).await
        }
        WalletOpenMode::OpenOrCreate | WalletOpenMode::Restore => {
            if db.read_properties().await?.is_some() {
                Wallet::open(mnemonic, db, config, lock_manager).await
            } else {
                Wallet::create(mnemonic, network, config, db, lock_manager, false).await
            }
        }
    }
}

fn remove_wallet_database_files(db_path: &Path) -> anyhow::Result<()> {
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
