use std::path::{Path, PathBuf};
use std::sync::{mpsc, Arc};

use anyhow::Context;
use bark::lock_manager::memory::MemoryLockManager;
use bark::persist::{sqlite::SqliteClient, BarkPersister};
use bark::{Config, OpenWalletArgs, RecoveryReport, RecoveryStatus, Wallet, WalletSeed};
use bip39::Mnemonic;

use crate::persistence::ServerConfig;

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

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct WalletRecoveryNotice {
    pub(crate) message: String,
    pub(crate) warning: bool,
}

pub(crate) struct OpenedBarkWallet {
    pub(crate) wallet: Wallet,
    pub(crate) recovery_notice: Option<WalletRecoveryNotice>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct WalletRecoverySummary {
    recovered_vtxos: usize,
    recovered_sat: u64,
    accounted_vtxos: usize,
    unresolved_vtxos: usize,
}

impl WalletRecoverySummary {
    fn from_report(report: &RecoveryReport) -> Self {
        WalletRecoverySummary {
            recovered_vtxos: report.recovered().len(),
            recovered_sat: report.recovered().total_amount().to_sat(),
            accounted_vtxos: report.skipped().len() + report.exited().len(),
            unresolved_vtxos: report.failed().len() + report.foreign().len(),
        }
    }

    fn notice(self) -> WalletRecoveryNotice {
        let recovered = format!(
            "recovered {} {} ({})",
            self.recovered_vtxos,
            pluralize_vtxo(self.recovered_vtxos),
            crate::state::format_sats(self.recovered_sat),
        );

        if self.unresolved_vtxos > 0 {
            let accounted = if self.accounted_vtxos == 0 {
                String::new()
            } else {
                format!(
                    "; {} {} already spent or exited",
                    self.accounted_vtxos,
                    pluralize_vtxo(self.accounted_vtxos),
                )
            };
            WalletRecoveryNotice {
                message: format!(
                    "Wallet recovery incomplete: {recovered}{accounted}; {} {} could not be verified. Retry restore; funds may be missing.",
                    self.unresolved_vtxos,
                    pluralize_vtxo(self.unresolved_vtxos),
                ),
                warning: true,
            }
        } else {
            let accounted = if self.accounted_vtxos == 0 {
                String::new()
            } else {
                format!(
                    "; {} {} already spent or exited",
                    self.accounted_vtxos,
                    pluralize_vtxo(self.accounted_vtxos),
                )
            };
            WalletRecoveryNotice {
                message: format!("Wallet recovery complete: {recovered}{accounted}."),
                warning: false,
            }
        }
    }
}

fn pluralize_vtxo(count: usize) -> &'static str {
    if count == 1 {
        "VTXO"
    } else {
        "VTXOs"
    }
}

fn failed_recovery_notice(error: Option<&anyhow::Error>) -> WalletRecoveryNotice {
    let reason = match error {
        Some(e) => format!(" ({e:#})"),
        None => String::new(),
    };
    WalletRecoveryNotice {
        message: format!(
            "Wallet recovery failed before a report was available{reason}. Retry restore; funds may be missing."
        ),
        warning: true,
    }
}

/// Turns the outcome bark reports on wallet open into a user-facing notice.
/// `NotRun` means the wallet already existed, so there is nothing to say.
fn recovery_status_notice(status: RecoveryStatus) -> Option<WalletRecoveryNotice> {
    match status {
        RecoveryStatus::NotRun => None,
        RecoveryStatus::Failed(e) => Some(failed_recovery_notice(Some(&e))),
        RecoveryStatus::Completed(report) => {
            Some(WalletRecoverySummary::from_report(&report).notice())
        }
    }
}

pub(crate) async fn open_bark_wallet(
    data_dir: PathBuf,
    mnemonic: &Mnemonic,
    mode: WalletOpenMode,
    server_config: ServerConfig,
) -> anyhow::Result<OpenedBarkWallet> {
    std::fs::create_dir_all(&data_dir)?;
    let network = server_config.network.bitcoin_network();
    let db_path = data_dir.join(server_config.network.db_file_name());
    if matches!(mode, WalletOpenMode::Replace) {
        remove_wallet_database_files(&db_path)?;
    }
    let seed = WalletSeed::new_from_mnemonic(network, mnemonic);
    // Create and Restore only run during onboarding, when the Keychain holds
    // no seed. A database left behind for a different seed (a create that
    // failed before its seed was saved, or app data restored onto a device
    // without the Keychain item) would block setup forever, so set it aside.
    if matches!(mode, WalletOpenMode::Create | WalletOpenMode::Restore) {
        set_aside_orphaned_wallet_database(&db_path, &seed).await?;
    }
    let db: Arc<dyn BarkPersister> = Arc::new(SqliteClient::open(&db_path)?);
    let config = bark_config(server_config);
    let lock_manager = Box::new(MemoryLockManager::new());

    // A newly created wallet should not scan the seed mailbox. Replacement
    // wallets must be created by Wallet::open so Bark knows to run recovery.
    if matches!(mode, WalletOpenMode::Create) {
        Wallet::create(network, &seed, &config, &*db, &*lock_manager, false).await?;
    }

    let recovery_expected =
        !matches!(mode, WalletOpenMode::Create) && db.read_properties().await?.is_none();
    let (recovery_tx, recovery_rx) = mpsc::sync_channel(1);

    let args = OpenWalletArgs {
        run_daemon: false,
        persister: Some(db),
        lock_manager: Some(lock_manager),
        create_if_not_exists: true,
        create_without_server: false,
        on_recovery_finished: Some(Box::new(move |status| {
            let _ = recovery_tx.send(recovery_status_notice(status));
        })),
        ..Default::default()
    };
    let wallet = match Wallet::open(network, seed, config, args).await {
        Ok(wallet) => wallet,
        Err(e) => {
            // The seed of a wallet created above is never saved when opening
            // fails, so its database is unusable. Remove it so a retry starts
            // clean.
            if matches!(mode, WalletOpenMode::Create) {
                let _ = remove_wallet_database_files(&db_path);
            }
            return Err(e);
        }
    };
    // Bark calls the callback exactly once per successful open. If it somehow
    // did not and we expected a scan, treat that like a failed scan.
    let recovery_notice = match recovery_rx.try_recv() {
        Ok(notice) => notice,
        Err(_) => recovery_expected.then(|| failed_recovery_notice(None)),
    };
    Ok(OpenedBarkWallet {
        wallet,
        recovery_notice,
    })
}

fn bark_config(server_config: ServerConfig) -> Config {
    let network = server_config.network.bitcoin_network();
    Config {
        server_address: server_config.server_address,
        esplora_address: Some(server_config.esplora_address),
        user_agent: Some(USER_AGENT.to_string()),
        ..Config::network_default(network)
    }
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

/// Moves the wallet database aside when it belongs to a different seed.
/// Returns whether it did.
async fn set_aside_orphaned_wallet_database(
    db_path: &Path,
    seed: &WalletSeed,
) -> anyhow::Result<bool> {
    if !db_path.exists() {
        return Ok(false);
    }
    let properties = SqliteClient::open(db_path)?.read_properties().await?;
    let orphaned = properties.is_some_and(|p| p.fingerprint != seed.fingerprint());
    if orphaned {
        set_aside_wallet_database_files(db_path)?;
    }
    Ok(orphaned)
}

/// Renames the wallet database files out of the way instead of deleting them,
/// so a wallet whose seed is still written down somewhere is not destroyed.
fn set_aside_wallet_database_files(db_path: &Path) -> anyhow::Result<()> {
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or_default();
    for suffix in ["", "-wal", "-shm"] {
        let path = PathBuf::from(format!("{}{}", db_path.display(), suffix));
        let target = PathBuf::from(format!(
            "{}.orphaned-{timestamp}{}",
            db_path.display(),
            suffix
        ));
        match std::fs::rename(&path, &target) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => {
                return Err(e).with_context(|| format!("failed to move aside {}", path.display()))
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        bark_config, failed_recovery_notice, recovery_status_notice,
        set_aside_orphaned_wallet_database, WalletRecoverySummary,
    };
    use crate::persistence::ServerConfig;
    use crate::WalletNetwork;
    use bark::persist::{sqlite::SqliteClient, BarkPersister};
    use bark::{RecoveryStatus, WalletProperties, WalletSeed};
    use bip39::Mnemonic;

    fn test_seed() -> WalletSeed {
        let mnemonic = Mnemonic::generate(12).expect("mnemonic");
        WalletSeed::new_from_mnemonic(bitcoin::Network::Signet, &mnemonic)
    }

    async fn init_test_wallet_db(db_path: &std::path::Path, seed: &WalletSeed) {
        SqliteClient::open(db_path)
            .expect("open db")
            .init_wallet(&WalletProperties {
                network: bitcoin::Network::Signet,
                fingerprint: seed.fingerprint(),
                server_pubkey: None,
                server_mailbox_pubkey: None,
            })
            .await
            .expect("init wallet");
    }

    #[test]
    fn database_for_another_seed_is_set_aside() {
        let rt = tokio::runtime::Runtime::new().expect("runtime");
        rt.block_on(async {
            let dir = tempfile::tempdir().expect("temp dir");
            let db_path = dir.path().join("wallet.sqlite");
            init_test_wallet_db(&db_path, &test_seed()).await;

            assert!(set_aside_orphaned_wallet_database(&db_path, &test_seed())
                .await
                .expect("set aside"));
            assert!(!db_path.exists());
            let moved = std::fs::read_dir(dir.path())
                .expect("read dir")
                .filter_map(Result::ok)
                .any(|e| {
                    e.file_name()
                        .to_string_lossy()
                        .starts_with("wallet.sqlite.orphaned-")
                });
            assert!(moved);
        });
    }

    #[test]
    fn database_for_same_seed_is_kept() {
        let rt = tokio::runtime::Runtime::new().expect("runtime");
        rt.block_on(async {
            let dir = tempfile::tempdir().expect("temp dir");
            let db_path = dir.path().join("wallet.sqlite");
            let seed = test_seed();
            init_test_wallet_db(&db_path, &seed).await;

            assert!(!set_aside_orphaned_wallet_database(&db_path, &seed)
                .await
                .expect("check db"));
            assert!(db_path.exists());
        });
    }

    #[test]
    fn refresh_threshold_uses_bark_network_defaults() {
        let mainnet = bark_config(ServerConfig::for_network(WalletNetwork::Mainnet));
        let signet = bark_config(ServerConfig::for_network(WalletNetwork::Signet));

        assert_eq!(mainnet.vtxo_refresh_expiry_threshold, 144.into());
        assert_eq!(signet.vtxo_refresh_expiry_threshold, 12.into());
    }

    #[test]
    fn complete_recovery_notice_summarizes_recovered_and_accounted_vtxos() {
        let notice = WalletRecoverySummary {
            recovered_vtxos: 1,
            recovered_sat: 12_345,
            accounted_vtxos: 2,
            unresolved_vtxos: 0,
        }
        .notice();

        assert_eq!(
            notice.message,
            "Wallet recovery complete: recovered 1 VTXO (12,345 sats); 2 VTXOs already spent or exited."
        );
        assert!(!notice.warning);
    }

    #[test]
    fn incomplete_recovery_notice_warns_that_funds_may_be_missing() {
        let notice = WalletRecoverySummary {
            recovered_vtxos: 2,
            recovered_sat: 50_000,
            accounted_vtxos: 1,
            unresolved_vtxos: 3,
        }
        .notice();

        assert_eq!(
            notice.message,
            "Wallet recovery incomplete: recovered 2 VTXOs (50,000 sats); 1 VTXO already spent or exited; 3 VTXOs could not be verified. Retry restore; funds may be missing."
        );
        assert!(notice.warning);
    }

    #[test]
    fn missing_recovery_report_is_a_warning() {
        let notice = failed_recovery_notice(None);

        assert!(notice.message.contains("funds may be missing"));
        assert!(notice.warning);
    }

    #[test]
    fn failed_recovery_status_includes_the_error() {
        let notice =
            recovery_status_notice(RecoveryStatus::Failed(anyhow::anyhow!("mailbox down")))
                .expect("failed scan yields a notice");

        assert!(notice.message.contains("mailbox down"));
        assert!(notice.message.contains("funds may be missing"));
        assert!(notice.warning);
    }

    #[test]
    fn recovery_not_run_yields_no_notice() {
        assert_eq!(recovery_status_notice(RecoveryStatus::NotRun), None);
    }
}
