use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Context;
use bark::lock_manager::memory::MemoryLockManager;
use bark::persist::{sqlite::SqliteClient, BarkPersister};
use bark::{Config, Wallet};
use bip39::Mnemonic;

use crate::persistence::ServerConfig;

#[derive(Clone, Copy, Debug)]
pub(crate) enum WalletOpenMode {
    Create,
    Open,
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
    let db_path = data_dir.join("rebel-wallet.sqlite");
    if matches!(mode, WalletOpenMode::Replace) {
        remove_wallet_database_files(&db_path)?;
    }
    let db = Arc::new(SqliteClient::open(&db_path)?);
    let config = Config {
        server_address: server_config.server_address,
        esplora_address: Some(server_config.esplora_address),
        ..Config::network_default(bitcoin::Network::Signet)
    };
    let lock_manager = Box::new(MemoryLockManager::new());
    match mode {
        WalletOpenMode::Create | WalletOpenMode::Replace => {
            Wallet::create(
                mnemonic,
                bitcoin::Network::Signet,
                config,
                db,
                lock_manager,
                false,
            )
            .await
        }
        WalletOpenMode::Open => Wallet::open(mnemonic, db, config, lock_manager).await,
        WalletOpenMode::Restore => {
            if db.read_properties().await?.is_some() {
                Wallet::open(mnemonic, db, config, lock_manager).await
            } else {
                Wallet::create(
                    mnemonic,
                    bitcoin::Network::Signet,
                    config,
                    db,
                    lock_manager,
                    false,
                )
                .await
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
