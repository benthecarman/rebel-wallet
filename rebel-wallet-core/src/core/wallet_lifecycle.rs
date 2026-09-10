use std::path::Path;
use std::str::FromStr;

use anyhow::Context;
use bip39::Mnemonic;
use zeroize::Zeroizing;

use super::custom_address_flow::{
    lightning_address_local_part, pending_custom_lightning_address_matches_name,
};
use super::{AppCore, NOSTR_SECRET_KEY, WALLET_SEED_KEY};
use crate::custom_address::amount_msats_to_sat;
use crate::persistence::{PersistedAppData, PersistedPriceCurrency, ServerConfig};
use crate::profile_cache::{
    hydrate_contact_picture, hydrate_own_profile_picture, sanitize_persisted_contact_pictures,
};
use crate::updates::{AsyncMsg, CoreMsg, HapticFeedback};
use crate::wallet::{open_bark_wallet, remove_wallet_database_files, WalletOpenMode};
use crate::{AppState, LightningAddressRegistrationPhase, PriceCurrency, WalletNetwork};

impl AppCore {
    pub(super) fn bootstrap(&mut self) {
        self.load_app_data();
        self.refresh_cached_contact_profiles_on_startup();
        self.load_nostr_key();
        self.with_nwc(|nwc, context| nwc.sync_push_registrations(context));
        self.refresh_price();
        if let Some(mnemonic) = self.secrets.get_secret(WALLET_SEED_KEY.to_string()) {
            self.state.busy.bootstrapping = true;
            self.state.busy.opening_wallet = true;
            self.open_wallet(Zeroizing::new(mnemonic), WalletOpenMode::OpenOrCreate);
        }
    }

    pub(super) fn open_wallet(&mut self, mnemonic: Zeroizing<String>, mode: WalletOpenMode) {
        let generation = self.invalidate_wallet_session();
        let tx = self.tx.clone();
        let data_dir = self.data_dir.clone();
        let server_config = ServerConfig::from_wallet(&self.state.wallet);
        self.rt.spawn(async move {
            let result = async {
                let mnemonic =
                    Mnemonic::from_str(mnemonic.as_str()).context("invalid recovery phrase")?;
                let opened = open_bark_wallet(data_dir, &mnemonic, mode, server_config).await?;
                Ok::<_, anyhow::Error>((opened, Zeroizing::new(mnemonic.to_string())))
            }
            .await;
            let msg = match result {
                Ok((opened, mnemonic)) => AsyncMsg::WalletReady {
                    generation,
                    wallet: opened.wallet,
                    mnemonic,
                    recovery_notice: opened.recovery_notice,
                },
                Err(e) => AsyncMsg::WalletOpenFailed {
                    generation,
                    message: format!("Wallet setup failed: {e:#}"),
                },
            };
            let _ = tx.send(CoreMsg::Async(msg));
        });
    }

    pub(super) fn delete_wallet(&mut self) {
        self.invalidate_wallet_session();

        // Remove local data first. Only delete the secrets once the data they
        // unlock is confirmed gone, so a failed cleanup cannot orphan the
        // databases behind an already-deleted seed.
        let mut errors = Vec::new();
        for network in [
            WalletNetwork::Mainnet,
            WalletNetwork::Signet,
            WalletNetwork::Regtest,
        ] {
            let db_path = self.data_dir.join(network.db_file_name());
            if let Err(e) = remove_wallet_database_files(&db_path) {
                errors.push(format!("{e:#}"));
            }
        }
        let (nwc_cleanup, nwc_errors) =
            self.with_nwc(|nwc, context| nwc.remove_wallet_data(context));
        errors.extend(nwc_errors);

        match std::fs::remove_file(&self.app_data_path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => errors.push(format!(
                "failed to remove {}: {e}",
                self.app_data_path.display()
            )),
        }

        self.payment_annotations.clear();
        self.zap_receipts.clear();
        self.profile_picture_downloads.clear();
        self.profile_info_requests.clear();

        let mut state = AppState::initial();
        state.show_launch_splash = false;
        self.state = state;

        let mut warnings = self.nwc.reset_after_wallet_deletion();

        if errors.is_empty() {
            if !self.secrets.delete_secret(WALLET_SEED_KEY.to_string()) {
                errors.push("wallet seed".to_string());
            }
            let _ = self.secrets.delete_secret(NOSTR_SECRET_KEY.to_string());
            errors.extend(self.nwc.delete_wallet_secrets(nwc_cleanup));
        }

        warnings.extend(errors);

        if warnings.is_empty() {
            self.state.toast = Some("Wallet deleted. Start over to create or restore.".to_string());
            self.request_haptic(HapticFeedback::NotificationSuccess);
        } else {
            self.state.toast = Some(format!(
                "Wallet reset with cleanup warnings: {}",
                warnings.join(", ")
            ));
            self.request_haptic(HapticFeedback::NotificationWarning);
        }
    }

    pub(super) fn select_network(
        &mut self,
        network: WalletNetwork,
        server_address: Option<String>,
        esplora_address: Option<String>,
    ) {
        if supported_wallet_network(network) != network {
            self.state.toast = Some("Regtest is unavailable in this build.".to_string());
            self.request_haptic(HapticFeedback::NotificationWarning);
            return;
        }
        let server_config = match network {
            WalletNetwork::Regtest => {
                let server_address =
                    server_address.unwrap_or_else(|| network.server_address().to_string());
                let esplora_address =
                    esplora_address.unwrap_or_else(|| network.esplora_address().to_string());
                let server_address = match validate_server_url(&server_address, "ASP") {
                    Ok(address) => address,
                    Err(message) => {
                        self.state.toast = Some(message);
                        self.request_haptic(HapticFeedback::NotificationWarning);
                        return;
                    }
                };
                let esplora_address = match validate_server_url(&esplora_address, "Esplora") {
                    Ok(address) => address,
                    Err(message) => {
                        self.state.toast = Some(message);
                        self.request_haptic(HapticFeedback::NotificationWarning);
                        return;
                    }
                };
                ServerConfig {
                    network,
                    server_address,
                    esplora_address,
                }
            }
            WalletNetwork::Mainnet | WalletNetwork::Signet => ServerConfig::for_network(network),
        };
        let server_address = server_config.server_address;
        let esplora_address = server_config.esplora_address;

        let wallet_server_changed = self.state.wallet.server_address != server_address
            || self.state.wallet.esplora_address != esplora_address;
        let changed = self.state.wallet.network != network || wallet_server_changed;
        self.state.wallet.network = network;
        self.state.wallet.server_address = server_address;
        self.state.wallet.esplora_address = esplora_address;
        self.state.lightning_address.backing_ark_address = None;
        self.save_app_data();

        if wallet_server_changed {
            if let Some(seed) = self.secrets.get_secret(WALLET_SEED_KEY.to_string()) {
                self.state.busy.opening_wallet = true;
                self.open_wallet(Zeroizing::new(seed), WalletOpenMode::OpenOrCreate);
                self.state.toast = Some("Network changed. Reconnecting wallet.".to_string());
                self.request_haptic(HapticFeedback::NotificationSuccess);
            } else {
                self.state.toast = Some("Network changed.".to_string());
                self.request_haptic(HapticFeedback::NotificationSuccess);
            }
        } else if changed {
            self.ensure_lightning_address();
            self.state.toast = Some("Network changed.".to_string());
            self.request_haptic(HapticFeedback::NotificationSuccess);
        } else {
            self.state.toast = Some("Network already selected.".to_string());
            self.request_haptic(HapticFeedback::NotificationWarning);
        }
    }

    pub(super) fn set_price_currency(&mut self, currency: PriceCurrency) {
        self.state.wallet.price_currency = currency;
        self.state.wallet.btc_price = None;
        self.save_app_data();
        self.request_haptic(HapticFeedback::NotificationSuccess);
        self.refresh_price();
    }

    pub(super) fn load_lightning_address_ark_address(&self) -> Option<String> {
        load_wallet_metadata_value(
            &self.data_dir,
            self.state.wallet.network,
            "lightning_address_ark_address",
        )
    }

    pub(super) fn save_lightning_address_ark_address(&self, address: &str) {
        let _ = save_wallet_metadata_value(
            &self.data_dir,
            self.state.wallet.network,
            "lightning_address_ark_address",
            address,
        );
    }

    pub(super) fn load_app_data(&mut self) {
        let raw = match std::fs::read_to_string(&self.app_data_path) {
            Ok(raw) => raw,
            Err(_) => {
                self.with_nwc(|nwc, context| nwc.load_connections(context));
                return;
            }
        };
        match serde_json::from_str::<PersistedAppData>(&raw) {
            Ok(data) => {
                self.state.nostr = data.nostr;
                self.hydrate_cached_profile_pictures();
                self.sort_contacts();
                self.state.receive.amount_sat = data.receive_amount_sat;
                self.state.receive.memo = if data.receive_memo == "Rebel Wallet" {
                    String::new()
                } else {
                    data.receive_memo
                };
                self.state.wallet.network = supported_wallet_network(data.network);
                let server_config = if self.state.wallet.network == WalletNetwork::Regtest
                    && data.servers.network == WalletNetwork::Regtest
                {
                    data.servers
                } else {
                    ServerConfig::for_network(self.state.wallet.network)
                };
                self.state.wallet.server_address = server_config.server_address;
                self.state.wallet.esplora_address = server_config.esplora_address;
                self.state.wallet.price_currency = data.price_currency.currency;
                self.state.lightning_address.backing_ark_address = data
                    .lightning_address_ark_address
                    .filter(|address| !address.trim().is_empty());
                self.state.lightning_address.custom_address = data
                    .custom_lightning_address
                    .filter(|address| !address.trim().is_empty());
                self.state.lightning_address.custom_name = data.custom_lightning_address_name;
                let pending_custom_lightning_address = data
                    .pending_custom_lightning_address
                    .filter(pending_custom_lightning_address_matches_name);
                if let Some(pending) = pending_custom_lightning_address {
                    self.state.lightning_address.custom_name = pending.name;
                    self.state.lightning_address.backing_ark_address =
                        Some(pending.ark_address.clone());
                    self.state.lightning_address.registration_address =
                        Some(pending.lightning_address);
                    self.state
                        .lightning_address
                        .registration_payment_ark_address =
                        pending.payment_ark_address.or(Some(pending.ark_address));
                    self.state.lightning_address.registration_invoice = Some(pending.invoice);
                    self.state.lightning_address.registration_purchase_id =
                        Some(pending.purchase_id);
                    self.state.lightning_address.registration_amount_sat =
                        amount_msats_to_sat(pending.amount_msats).unwrap_or(0);
                    self.state.lightning_address.registration_phase =
                        LightningAddressRegistrationPhase::AwaitingPayment;
                    self.state.lightning_address.registration_status_text =
                        "Awaiting payment".to_string();
                } else if self
                    .state
                    .lightning_address
                    .custom_address
                    .as_ref()
                    .is_some_and(|address| !address.trim().is_empty())
                {
                    self.restore_active_custom_lightning_address_name();
                    self.state.lightning_address.registration_phase =
                        LightningAddressRegistrationPhase::Active;
                    self.state.lightning_address.registration_status_text = "Active".to_string();
                }
                self.payment_annotations = data.payment_annotations;
                self.zap_receipts = data.zap_receipts;
                self.with_nwc(|nwc, context| {
                    nwc.load_connections(context);
                    nwc.hydrate_icon_urls(context);
                    nwc.prefetch_icons(context);
                });
            }
            Err(e) => {
                self.state.toast = Some(format!("Could not load local app data: {e}"));
                self.with_nwc(|nwc, context| nwc.load_connections(context));
            }
        }
    }

    pub(super) fn hydrate_cached_profile_pictures(&mut self) {
        let cache_dir = self.cache_dir.clone();
        let profile_db = self.profile_db.as_ref();
        hydrate_own_profile_picture(profile_db, &cache_dir, &mut self.state.nostr);
        for contact in &mut self.state.nostr.contacts {
            hydrate_contact_picture(profile_db, &cache_dir, contact);
        }
        for contact in &mut self.state.send.global_search_results {
            hydrate_contact_picture(profile_db, &cache_dir, contact);
        }
    }

    pub(super) fn sort_contacts(&mut self) {
        crate::state::sort_contacts_by_name_npub(&mut self.state.nostr.contacts);
    }

    pub(super) fn save_app_data(&self) {
        let mut nostr = self.state.nostr.clone();
        sanitize_persisted_contact_pictures(self.profile_db.as_ref(), &mut nostr.contacts);
        let pending_custom_lightning_address = self.pending_custom_lightning_address();
        let custom_lightning_address_name = pending_custom_lightning_address
            .as_ref()
            .map(|pending| pending.name.clone())
            .or_else(|| {
                self.state
                    .lightning_address
                    .custom_address
                    .as_deref()
                    .and_then(lightning_address_local_part)
                    .map(str::to_string)
            })
            .unwrap_or_else(|| self.state.lightning_address.custom_name.clone());
        let data = PersistedAppData {
            nostr,
            receive_amount_sat: self.state.receive.amount_sat,
            receive_memo: self.state.receive.memo.clone(),
            network: self.state.wallet.network,
            servers: ServerConfig::from_wallet(&self.state.wallet),
            price_currency: PersistedPriceCurrency {
                currency: self.state.wallet.price_currency.clone(),
            },
            lightning_address_ark_address: None,
            custom_lightning_address: self.state.lightning_address.custom_address.clone(),
            custom_lightning_address_name,
            pending_custom_lightning_address,
            payment_annotations: self.payment_annotations.clone(),
            zap_receipts: self.zap_receipts.clone(),
        };
        if let Ok(raw) = serde_json::to_string_pretty(&data) {
            let _ = std::fs::create_dir_all(&self.data_dir);
            let _ = std::fs::write(&self.app_data_path, raw);
        }
    }
}

fn validate_server_url(value: &str, label: &str) -> Result<String, String> {
    let value = value.trim();
    let url = reqwest::Url::parse(value).map_err(|_| format!("Enter a valid {label} URL."))?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return Err(format!("Enter a valid HTTP or HTTPS {label} URL."));
    }
    if url.scheme() == "http" && !url.host_str().is_some_and(is_local_network_host) {
        return Err(format!(
            "Plain HTTP {label} URLs must use a loopback or private host."
        ));
    }
    Ok(value.trim_end_matches('/').to_string())
}

fn supported_wallet_network(network: WalletNetwork) -> WalletNetwork {
    if network == WalletNetwork::Regtest && !cfg!(feature = "regtest") {
        WalletNetwork::Mainnet
    } else {
        network
    }
}

fn is_local_network_host(host: &str) -> bool {
    if host.eq_ignore_ascii_case("localhost") {
        return true;
    }
    let host = host
        .strip_prefix('[')
        .and_then(|host| host.strip_suffix(']'))
        .unwrap_or(host);
    host.parse::<std::net::IpAddr>()
        .is_ok_and(|address| match address {
            std::net::IpAddr::V4(address) => address.is_loopback() || address.is_private(),
            std::net::IpAddr::V6(address) => address.is_loopback() || address.is_unique_local(),
        })
}

fn load_wallet_metadata_value(
    data_dir: &Path,
    network: WalletNetwork,
    key: &str,
) -> Option<String> {
    let db_path = data_dir.join(network.db_file_name());
    let conn = rusqlite::Connection::open(db_path).ok()?;
    ensure_wallet_metadata_table(&conn).ok()?;
    conn.query_row(
        "SELECT value FROM rebel_wallet_metadata WHERE key = ?1",
        [key],
        |row| row.get::<_, String>(0),
    )
    .ok()
    .filter(|value| !value.trim().is_empty())
}

fn save_wallet_metadata_value(
    data_dir: &Path,
    network: WalletNetwork,
    key: &str,
    value: &str,
) -> rusqlite::Result<()> {
    std::fs::create_dir_all(data_dir).ok();
    let db_path = data_dir.join(network.db_file_name());
    let conn = rusqlite::Connection::open(db_path)?;
    ensure_wallet_metadata_table(&conn)?;
    conn.execute(
        "INSERT INTO rebel_wallet_metadata (key, value)
         VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        (key, value),
    )?;
    Ok(())
}

fn ensure_wallet_metadata_table(conn: &rusqlite::Connection) -> rusqlite::Result<()> {
    conn.execute(
        "CREATE TABLE IF NOT EXISTS rebel_wallet_metadata (
            key TEXT PRIMARY KEY,
            value TEXT NOT NULL
        )",
        [],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{supported_wallet_network, validate_server_url};
    use crate::WalletNetwork;

    #[test]
    fn validates_and_normalizes_server_urls() {
        assert_eq!(
            validate_server_url("  http://192.168.1.10:3535/  ", "ASP"),
            Ok("http://192.168.1.10:3535".to_string())
        );
        assert!(validate_server_url("http://localhost:3535", "ASP").is_ok());
        assert!(validate_server_url("http://[::1]:3535", "ASP").is_ok());
        assert!(validate_server_url("http://203.0.113.10:3535", "ASP").is_err());
        assert!(validate_server_url("http://example.com", "ASP").is_err());
        assert!(validate_server_url("https://example.com", "ASP").is_ok());
        assert!(validate_server_url("ftp://example.com", "ASP").is_err());
        assert!(validate_server_url("not a url", "Esplora").is_err());
    }

    #[test]
    fn regtest_is_available_only_in_feature_builds() {
        let expected = if cfg!(feature = "regtest") {
            WalletNetwork::Regtest
        } else {
            WalletNetwork::Mainnet
        };
        assert_eq!(supported_wallet_network(WalletNetwork::Regtest), expected);
        assert_eq!(
            supported_wallet_network(WalletNetwork::Signet),
            WalletNetwork::Signet
        );
    }
}
