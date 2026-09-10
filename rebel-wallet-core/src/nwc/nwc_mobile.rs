use std::collections::{BTreeSet, HashSet};
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use bark::Wallet;
use flume::Sender;
use nostr_sdk::prelude::{Keys, PublicKey as NostrPublicKey};
use nwc_mobile::{
    parse_connection_relays, registration_retry_delay, standard_nwc_methods,
    ApplicationConnectionMetadata, ApplicationIconCache, ApplicationIconUrl,
    ApplicationRegistrationCompletion, ApplicationRegistrationPass, ForegroundWakeCoordinator,
    ForegroundWakeDecision, ForegroundWakeOutcome, ForegroundWakeRetryCause, HostError,
    HostErrorKind, NeverCancelled, Nip98SigningKey, NwcApplicationManager, NwcEncryption,
    NwcMethod, NwcNotificationType, NwcSecretKey, ProtectedSecretStore, RegistrationStart,
    StoredNwcSecrets, UnixTimestamp, WakeDisposition, WakeEnvelope, WalletConnectionRequest,
    DEFAULT_MAXIMUM_CONNECTION_RELAYS,
};
pub(crate) use nwc_mobile_http::ApnsWakeRegistrationConfig as NwcPushConfig;
use nwc_mobile_http::{InvoiceSettlementCompletion, InvoiceSettlementMonitorConfig};
use nwc_mobile_nostr::NostrRelayTransport;
use nwc_mobile_tokio::{LightningNodeProvider, NwcMobile, NwcMobileConfig, NwcMobileWakeKind};
use nwc_mobile_uniffi::{
    execute_native_extension_wake, MobileCancellation, MobileConnectionMetadata,
    MobileConnectionView, MobileWakeEnvelope,
};
pub use nwc_mobile_uniffi::{NwcExtensionWakeExecution, NwcSettlementNotificationStatus};
use zeroize::Zeroizing;

use crate::core::NOSTR_SECRET_KEY;
use crate::nostr_support::public_key_from_npub_or_hex;
use crate::profile_cache::normalize_profile_picture_to_jpeg;
use crate::updates::{AppUpdate, AsyncMsg, CoreMsg, HapticFeedback};
use crate::wallet::remove_wallet_database_files;
use crate::{
    AppState, NwcBudgetInterval, NwcPermission, NwcProcessedWakeRequest, NwcWakeRequest,
    SecretStore,
};

use super::nwc_bark::{cold_bark_provider, opened_bark_provider};

const INFO_PUBLISH_TIMEOUT: Duration = Duration::from_secs(10);
const SETTLEMENT_MONITOR_RESERVE: Duration = Duration::from_secs(5);
const NWC_CLIENT_SECRET_KEY_PREFIX: &str = "nwc_client_secret:";
const MAX_NWC_WAKE_HISTORY: usize = 30;
const NWC_INFO_EVENT_PUBLISH_ATTEMPTS: u8 = 3;
const NWC_FOREGROUND_OPERATION_TIMEOUT: Duration = Duration::from_secs(30);

pub(crate) struct NwcWalletCleanup {
    secrets: Vec<(String, String)>,
}

fn nwc_client_secret_key(client_pubkey: &str) -> String {
    format!("{NWC_CLIENT_SECRET_KEY_PREFIX}{client_pubkey}")
}

fn nwc_info_event_key(client_pubkey: &str, relay: &str) -> String {
    format!("{client_pubkey}|{relay}")
}

async fn publish_nwc_info_event_with_retry(
    relay: String,
    keys: Keys,
    client_pubkey: Option<NostrPublicKey>,
    permissions: Vec<NwcPermission>,
) -> anyhow::Result<()> {
    nwc_mobile_tokio::retry_with_exponential_backoff(
        NWC_INFO_EVENT_PUBLISH_ATTEMPTS,
        Duration::from_secs(1),
        || {
            publish_nwc_info_event(
                relay.clone(),
                keys.clone(),
                client_pubkey,
                permissions.clone(),
            )
        },
    )
    .await
}

fn parse_nwc_relay_urls(value: &str, fallback: &str) -> anyhow::Result<Vec<String>> {
    parse_connection_relays(value, fallback, DEFAULT_MAXIMUM_CONNECTION_RELAYS).map_err(Into::into)
}

pub(crate) fn nwc_relay_input_is_valid(value: &str) -> bool {
    parse_nwc_relay_urls(value, "").is_ok()
}

pub(crate) fn nwc_budget_interval_display(interval: NwcBudgetInterval) -> &'static str {
    match interval {
        NwcBudgetInterval::Never => "Never",
        NwcBudgetInterval::Hourly => "Hourly",
        NwcBudgetInterval::Daily => "Daily",
        NwcBudgetInterval::Weekly => "Weekly",
        NwcBudgetInterval::Monthly => "Monthly",
        NwcBudgetInterval::Yearly => "Yearly",
    }
}

/// The narrow slice of Rebel application state made available to NWC flows.
pub(crate) struct NwcAppContext<'a> {
    pub(crate) state: &'a mut AppState,
    pub(crate) wallet: Option<Wallet>,
    pub(crate) wallet_generation: u64,
    pub(crate) rev: u64,
}

pub(crate) struct NwcPushRegistrationUpdate {
    pub(crate) apns_device_token: Option<String>,
    pub(crate) registration_status: String,
    pub(crate) wake_server_url: Option<String>,
    pub(crate) app_id: String,
    pub(crate) environment: String,
    pub(crate) install_id: String,
}

pub(crate) struct NwcControllerOutput {
    pub(crate) save_app_data: bool,
    pub(crate) haptics: Vec<HapticFeedback>,
    pub(crate) side_effects: Vec<AppUpdate>,
}

/// Rebel's application-level owner for the reusable `nwc-mobile` runtime.
pub(crate) struct NwcController {
    data_dir: PathBuf,
    secrets: Arc<dyn SecretStore>,
    tx: Sender<CoreMsg>,
    runtime: tokio::runtime::Handle,
    icon_download_semaphore: Arc<tokio::sync::Semaphore>,
    icon_downloads: HashSet<String>,
    icon_cache: ApplicationIconCache,
    wake_coordinator: ForegroundWakeCoordinator<String>,
    in_flight_info_events: HashSet<String>,
    pub(super) manager: Option<NwcApplicationManager>,
    push_config: NwcPushConfig,
    settlement_monitor_config: Option<InvoiceSettlementMonitorConfig>,
    save_app_data: bool,
    haptics: Vec<HapticFeedback>,
    side_effects: Vec<AppUpdate>,
}

impl NwcController {
    pub(crate) fn new(
        data_dir: PathBuf,
        cache_dir: &Path,
        secrets: Arc<dyn SecretStore>,
        tx: Sender<CoreMsg>,
        runtime: tokio::runtime::Handle,
        icon_download_semaphore: Arc<tokio::sync::Semaphore>,
    ) -> Self {
        let icon_cache = ApplicationIconCache::new(cache_dir);
        let _ = icon_cache.prepare();
        let manager = NwcApplicationManager::open(&data_dir).ok();
        if let Some(manager) = manager.as_ref() {
            let _ = manager.service().refresh_nwc_info_events();
        }
        Self {
            data_dir,
            secrets,
            tx,
            runtime,
            icon_download_semaphore,
            icon_downloads: HashSet::new(),
            icon_cache,
            wake_coordinator: ForegroundWakeCoordinator::default(),
            in_flight_info_events: HashSet::new(),
            manager,
            push_config: NwcPushConfig::default(),
            settlement_monitor_config: None,
            save_app_data: false,
            haptics: Vec::new(),
            side_effects: Vec::new(),
        }
    }

    pub(crate) fn take_output(&mut self) -> NwcControllerOutput {
        NwcControllerOutput {
            save_app_data: std::mem::take(&mut self.save_app_data),
            haptics: std::mem::take(&mut self.haptics),
            side_effects: std::mem::take(&mut self.side_effects),
        }
    }

    #[cfg(test)]
    pub(crate) fn clear_manager_for_test(&mut self) {
        self.manager = None;
    }

    pub(super) fn request_save(&mut self) {
        self.save_app_data = true;
    }

    pub(super) fn request_haptic(&mut self, feedback: HapticFeedback) {
        self.haptics.push(feedback);
    }

    pub(super) fn push_side_effect(&mut self, update: AppUpdate) {
        self.side_effects.push(update);
    }

    pub(super) fn service_keys(&self) -> anyhow::Result<Keys> {
        let secret = self
            .secrets
            .get_secret(NOSTR_SECRET_KEY.to_string())
            .map(Zeroizing::new)
            .context("Nostr secret key not found")?;
        Keys::parse(secret.as_str()).context("invalid Nostr secret key")
    }

    pub(crate) fn hydrate_icon_urls(&self, context: &mut NwcAppContext<'_>) {
        for connection in &mut context.state.nwc.connections {
            connection.icon_display_url = connection
                .icon_url
                .as_deref()
                .and_then(|url| cached_nwc_icon_url(&self.icon_cache, url));
        }
        if let Some(request) = context.state.nwa.request.as_ref() {
            context.state.nwa.icon_display_url = request
                .icon_url
                .as_deref()
                .and_then(|url| cached_nwc_icon_url(&self.icon_cache, url));
        }
    }

    pub(crate) fn prefetch_icons(&mut self, context: &mut NwcAppContext<'_>) {
        let mut urls = context
            .state
            .nwc
            .connections
            .iter()
            .filter_map(|connection| connection.icon_url.clone())
            .collect::<Vec<_>>();
        if let Some(url) = context
            .state
            .nwa
            .request
            .as_ref()
            .and_then(|request| request.icon_url.clone())
        {
            urls.push(url);
        }
        urls.sort();
        urls.dedup();
        for url in urls {
            self.prefetch_icon(context, url);
        }
    }

    pub(super) fn prefetch_icon(&mut self, context: &mut NwcAppContext<'_>, remote_url: String) {
        let Ok(icon_url) = ApplicationIconUrl::parse(&remote_url) else {
            return;
        };
        if self
            .icon_cache
            .cached_file_url(&icon_url)
            .ok()
            .flatten()
            .is_some()
            || !self.icon_downloads.insert(remote_url.clone())
        {
            self.refresh_icon_url(context, &remote_url);
            return;
        }

        let tx = self.tx.clone();
        let icon_cache = self.icon_cache.clone();
        let semaphore = self.icon_download_semaphore.clone();
        self.runtime.spawn(async move {
            let failed_url = remote_url.clone();
            let result = async {
                let _permit = semaphore.acquire().await?;
                let bytes = nwc_mobile_http::download_application_icon(&icon_url).await?;
                let normalized = normalize_profile_picture_to_jpeg(&bytes)?;
                icon_cache.store(&icon_url, &normalized)?;
                Ok::<_, anyhow::Error>(remote_url)
            }
            .await;
            let message = match result {
                Ok(remote_url) => AsyncMsg::NwcIconCached { remote_url },
                Err(_) => AsyncMsg::NwcIconCacheFailed {
                    remote_url: failed_url,
                },
            };
            let _ = tx.send(CoreMsg::Async(message));
        });
    }

    pub(crate) fn finish_icon_cache(
        &mut self,
        context: &mut NwcAppContext<'_>,
        remote_url: String,
        succeeded: bool,
    ) {
        self.icon_downloads.remove(&remote_url);
        if succeeded {
            self.refresh_icon_url(context, &remote_url);
        }
    }

    pub(super) fn icon_display_url(&self, remote_url: Option<&str>) -> Option<String> {
        remote_url.and_then(|url| cached_nwc_icon_url(&self.icon_cache, url))
    }

    fn refresh_icon_url(&self, context: &mut NwcAppContext<'_>, remote_url: &str) {
        let Some(file_url) = cached_nwc_icon_url(&self.icon_cache, remote_url) else {
            return;
        };
        for connection in &mut context.state.nwc.connections {
            if connection.icon_url.as_deref() == Some(remote_url) {
                connection.icon_display_url = Some(file_url.clone());
            }
        }
        if let Some(request) = context.state.nwa.request.as_ref() {
            if request.icon_url.as_deref() == Some(remote_url) {
                context.state.nwa.icon_display_url = Some(file_url);
            }
        }
    }

    pub(crate) fn reset_wallet_session(&mut self) {
        self.wake_coordinator.reset();
    }

    pub(crate) fn load_connections(&self, context: &mut NwcAppContext<'_>) {
        let Some(manager) = self.manager.as_ref() else {
            context.state.nwc.last_wake_status =
                "NWC authorization storage is unavailable".to_string();
            return;
        };
        let presentations = match manager.connections() {
            Ok(presentations) => presentations,
            Err(error) => {
                context.state.nwc.last_wake_status =
                    format!("NWC authorization storage is unavailable: {error}");
                return;
            }
        };
        let connections = presentations
            .into_iter()
            .enumerate()
            .map(|(index, presentation)| {
                let wallet_managed_secret = self
                    .secrets
                    .get_secret(nwc_client_secret_key(presentation.client_pubkey_hex()))
                    .is_some();
                MobileConnectionView::from_presentation(
                    presentation,
                    format!("NWC {}", index + 1),
                    None,
                    wallet_managed_secret,
                )
                .map_err(anyhow::Error::from)
            })
            .collect::<anyhow::Result<Vec<_>>>();
        match connections {
            Ok(connections) => context.state.nwc.connections = connections,
            Err(error) => {
                context.state.nwc.last_wake_status =
                    format!("NWC authorization data is invalid: {error:#}");
            }
        }
    }

    pub(crate) fn refresh_connection_usage(&self, context: &mut NwcAppContext<'_>) {
        let Some(manager) = self.manager.as_ref() else {
            return;
        };
        let Ok(presentations) = manager.connections() else {
            context.state.nwc.last_wake_status =
                "NWC connection usage is temporarily unavailable".to_string();
            return;
        };
        for connection in &mut context.state.nwc.connections {
            if let Some(presentation) = presentations
                .iter()
                .find(|presentation| presentation.id() == connection.id)
            {
                connection.last_used_at = presentation.last_used_at().map(UnixTimestamp::as_secs);
                connection.spent_sat = presentation.spent_sat();
                connection.budget_period_started_at =
                    presentation.budget_period_started_at().as_secs();
            }
        }
    }

    pub(crate) fn create_connection(
        &mut self,
        context: &mut NwcAppContext<'_>,
        name: String,
        relay: String,
        budget_sat: u64,
        budget_interval: NwcBudgetInterval,
        permissions: Vec<NwcPermission>,
    ) {
        if self.manager.is_none() {
            context.state.toast = Some("NWC authorization storage is unavailable.".to_string());
            self.request_haptic(HapticFeedback::NotificationError);
            return;
        }
        let service_keys = match self.service_keys() {
            Ok(keys) => keys,
            Err(_) => {
                context.state.toast =
                    Some("Create or open the wallet before adding NWC.".to_string());
                self.request_haptic(HapticFeedback::NotificationWarning);
                return;
            }
        };

        let provider = rebel_secret_provider(self.secrets.clone());
        let created = self
            .manager
            .as_ref()
            .expect("checked above")
            .create_connection(
                WalletConnectionRequest::new(
                    service_keys.public_key().to_hex(),
                    relay,
                    context.state.nwc.default_relay.clone(),
                    permissions.into_iter().map(Into::into).collect(),
                    budget_sat,
                    budget_interval.into(),
                    NWC_ENCRYPTION,
                    None,
                    context.state.lightning_address.address.clone(),
                ),
                &provider,
            );
        let created = match created {
            Ok(created) => created,
            Err(error) => {
                context.state.toast = Some(format!("Could not create NWC connection: {error}"));
                self.request_haptic(HapticFeedback::NotificationError);
                return;
            }
        };
        let draft = created.draft();
        let relay_storage = draft.relay_storage().to_string();
        let trimmed_name = name.trim();
        let display_name = if trimmed_name.is_empty() {
            format!("NWC {}", context.state.nwc.connections.len() + 1)
        } else {
            trimmed_name.to_string()
        };
        let metadata = ApplicationConnectionMetadata::new(
            display_name.clone(),
            None,
            draft.authorization().relay_urls().to_vec(),
        );
        let metadata_result = metadata.and_then(|metadata| {
            self.manager
                .as_ref()
                .expect("checked above")
                .set_connection_metadata(draft.id(), metadata)
                .map_err(|_| nwc_mobile::RegistryError::InvalidConnection)
        });
        if metadata_result.is_err() {
            let _ = self
                .manager
                .as_ref()
                .expect("checked above")
                .revoke_connection(draft.id(), &provider);
            context.state.toast = Some("Could not persist NWC connection metadata.".to_string());
            self.request_haptic(HapticFeedback::NotificationError);
            return;
        }
        let connection = crate::NwcConnection::from_created(
            &created,
            MobileConnectionMetadata {
                name: display_name,
                icon_url: None,
                icon_display_url: None,
                wallet_managed_secret: true,
            },
        )
        .expect("shared workflow result must produce a native connection view");
        context.state.nwc.connections.push(connection);
        context.state.nwc.default_relay = relay_storage;
        context.state.toast = Some("NWC string created.".to_string());
        self.request_haptic(HapticFeedback::NotificationSuccess);
        self.request_save();
        self.publish_pending_info_events(context);
        self.sync_push_registrations(context);
        let connection = context.state.nwc.connections.last().expect("just inserted");
        self.push_side_effect(AppUpdate::NwcConnectionExportReady {
            rev: context.rev + 1,
            connection_id: connection.id.clone(),
            name: connection.name.clone(),
            uri: created.uri().to_owned(),
            copy_to_clipboard: false,
            present_qr: true,
        });
    }

    pub(crate) fn export_connection(
        &mut self,
        context: &mut NwcAppContext<'_>,
        id: String,
        copy_to_clipboard: bool,
    ) {
        let Some(connection) = context
            .state
            .nwc
            .connections
            .iter()
            .find(|connection| connection.id == id)
        else {
            context.state.toast = Some("NWC connection was not found.".to_string());
            return;
        };
        let provider = rebel_secret_provider(self.secrets.clone());
        let uri = match self
            .manager
            .as_ref()
            .expect("connection state requires the shared manager")
            .export_connection_uri(
                &connection.id,
                context.state.lightning_address.address.clone(),
                &provider,
            ) {
            Ok(uri) => uri,
            Err(_) => {
                context.state.toast = Some(
                    "This client-created NWC connection cannot be exported by the wallet."
                        .to_string(),
                );
                self.request_haptic(HapticFeedback::NotificationWarning);
                return;
            }
        };
        self.push_side_effect(AppUpdate::NwcConnectionExportReady {
            rev: context.rev + 1,
            connection_id: connection.id.clone(),
            name: connection.name.clone(),
            uri,
            copy_to_clipboard,
            present_qr: !copy_to_clipboard,
        });
    }

    pub(crate) fn delete_connection(&mut self, context: &mut NwcAppContext<'_>, id: String) {
        let deleted_connections = context
            .state
            .nwc
            .connections
            .iter()
            .filter(|connection| connection.id == id)
            .cloned()
            .collect::<Vec<_>>();
        if deleted_connections.is_empty() {
            return;
        }
        let provider = rebel_secret_provider(self.secrets.clone());
        let revocation_result = self
            .manager
            .as_ref()
            .context("NWC authorization storage is unavailable")
            .and_then(|manager| {
                deleted_connections.iter().try_for_each(|connection| {
                    let revoked = manager
                        .revoke_connection(&connection.id, &provider)
                        .context("could not revoke the NWC authorization")?;
                    anyhow::ensure!(
                        revoked.client_secret_deleted(),
                        "could not delete the NWC client secret"
                    );
                    Ok(())
                })
            });
        if let Err(error) = revocation_result {
            context.state.toast = Some(format!("Could not revoke NWC connection: {error:#}"));
            self.request_haptic(HapticFeedback::NotificationError);
            return;
        }
        let deleted_client_pubkeys = context
            .state
            .nwc
            .connections
            .iter()
            .filter(|connection| connection.id == id)
            .map(|connection| connection.client_pubkey.clone())
            .collect::<Vec<_>>();
        let before = context.state.nwc.connections.len();
        context
            .state
            .nwc
            .connections
            .retain(|connection| connection.id != id);
        if context.state.nwc.connections.len() < before {
            self.in_flight_info_events.retain(|key| {
                !deleted_client_pubkeys
                    .iter()
                    .any(|client_pubkey| key.starts_with(&format!("{client_pubkey}|")))
            });
            context.state.toast = Some("NWC string deleted.".to_string());
            self.request_haptic(HapticFeedback::NotificationWarning);
            self.request_save();
            self.sync_push_registrations(context);
        }
    }

    pub(crate) fn publish_pending_info_events(&mut self, context: &mut NwcAppContext<'_>) {
        let Ok(keys) = self.service_keys() else {
            return;
        };
        let pending = context
            .state
            .nwc
            .connections
            .iter()
            .flat_map(|connection| {
                let client_pubkey = connection.client_pubkey.clone();
                let targeted = self
                    .secrets
                    .get_secret(nwc_client_secret_key(&client_pubkey))
                    .is_none();
                let permissions = connection.enabled_permissions();
                connection
                    .pending_info_event_relays
                    .iter()
                    .cloned()
                    .map(move |relay| (client_pubkey.clone(), relay, targeted, permissions.clone()))
            })
            .collect::<Vec<_>>();

        for (client_pubkey_hex, relay, targeted, permissions) in pending {
            let in_flight_key = nwc_info_event_key(&client_pubkey_hex, &relay);
            if !self.in_flight_info_events.insert(in_flight_key) {
                continue;
            }
            let client_pubkey = if targeted {
                match public_key_from_npub_or_hex(&client_pubkey_hex) {
                    Ok(client_pubkey) => Some(client_pubkey),
                    Err(_) => {
                        self.in_flight_info_events
                            .remove(&nwc_info_event_key(&client_pubkey_hex, &relay));
                        continue;
                    }
                }
            } else {
                None
            };
            let tx = self.tx.clone();
            let keys = keys.clone();
            self.runtime.spawn(async move {
                let result = publish_nwc_info_event_with_retry(
                    relay.clone(),
                    keys,
                    client_pubkey,
                    permissions,
                )
                .await;
                let message = match result {
                    Ok(()) => AsyncMsg::NwcInfoEventPublished {
                        client_pubkey: client_pubkey_hex,
                        relay,
                    },
                    Err(error) => AsyncMsg::NwcInfoEventFailed {
                        client_pubkey: client_pubkey_hex,
                        relay,
                        error: format!("{error:#}"),
                    },
                };
                let _ = tx.send(CoreMsg::Async(message));
            });
        }
    }

    pub(crate) fn refresh_connection_uris(&self, context: &mut NwcAppContext<'_>) {
        context.state.refresh_derived();
    }

    pub(crate) fn update_push_registration(
        &mut self,
        context: &mut NwcAppContext<'_>,
        update: NwcPushRegistrationUpdate,
    ) {
        let NwcPushRegistrationUpdate {
            apns_device_token,
            registration_status,
            wake_server_url,
            app_id,
            environment,
            install_id,
        } = update;
        let wake_enabled = registration_status != "Permission denied";
        let settlement_monitor_config =
            InvoiceSettlementMonitorConfig::new(wake_server_url.clone(), install_id.clone()).ok();
        context.state.push_notifications.apns_device_token = apns_device_token.clone();
        context.state.push_notifications.registration_status = registration_status;
        let config = NwcPushConfig::new(
            wake_server_url,
            apns_device_token,
            app_id,
            environment,
            install_id,
            wake_enabled,
        );
        if self.push_config != config {
            self.push_config = config;
        }
        if let Some(manager) = self.manager.as_mut() {
            manager.mark_registration_refresh_pending();
        }
        self.settlement_monitor_config = settlement_monitor_config;
        self.sync_push_registrations(context);
    }

    pub(crate) fn sync_push_registrations(&mut self, context: &mut NwcAppContext<'_>) {
        let Ok(config) = self.push_config.ready() else {
            return;
        };
        let Ok(keys) = self.service_keys() else {
            return;
        };
        let Some(manager) = self.manager.as_mut() else {
            return;
        };
        match manager.begin_registration(config.enabled()) {
            Ok(RegistrationStart::Busy) => return,
            Ok(RegistrationStart::Ready) => {}
            Ok(_) => return,
            Err(_) => {
                context.state.nwc.last_wake_status =
                    "NWC wake registration storage is unavailable".to_string();
                return;
            }
        }
        let data_dir = self.data_dir.clone();
        let tx = self.tx.clone();
        self.runtime.spawn(async move {
            let result = match NwcApplicationManager::open(&data_dir) {
                Ok(manager) => {
                    run_registration_worker(manager.service().ledger(), config, keys).await
                }
                Err(error) => Err(error.into()),
            };
            let message = match result {
                Ok(pass) => AsyncMsg::NwcPushRegistrationFinished {
                    applied: pass.applied(),
                    deferred: pass.deferred(),
                    next_attempt_at: pass.next_attempt_at(),
                    error: None,
                },
                Err(_) => AsyncMsg::NwcPushRegistrationFinished {
                    applied: 0,
                    deferred: 0,
                    next_attempt_at: None,
                    error: Some("durable registration pass failed".to_string()),
                },
            };
            let _ = tx.send(CoreMsg::Async(message));
        });
    }

    pub(crate) fn finish_push_registration(
        &mut self,
        context: &mut NwcAppContext<'_>,
        applied: usize,
        deferred: usize,
        next_attempt_at: Option<u64>,
        error: Option<String>,
    ) {
        let pass = if error.is_some() {
            ApplicationRegistrationPass::failed()
        } else {
            ApplicationRegistrationPass::completed(applied, deferred, next_attempt_at)
        };
        let Some(manager) = self.manager.as_mut() else {
            return;
        };
        match manager.finish_registration(pass, UnixTimestamp::from_secs(crate::time::now_unix())) {
            ApplicationRegistrationCompletion::Ignored => {}
            ApplicationRegistrationCompletion::RunAgain => self.sync_push_registrations(context),
            ApplicationRegistrationCompletion::Failed { retry_at } => {
                let error = error.unwrap_or_else(|| "durable registration pass failed".to_string());
                context.state.nwc.last_wake_status =
                    format!("NWC wake registration failed: {error}");
                self.schedule_push_retry(retry_at);
            }
            ApplicationRegistrationCompletion::Deferred { retry_at } => {
                context.state.nwc.last_wake_status =
                    "NWC wake registration queued for retry".to_string();
                if let Some(retry_at) = retry_at {
                    self.schedule_push_retry(retry_at);
                }
            }
            ApplicationRegistrationCompletion::Applied { applied, retry_at } => {
                context.state.nwc.last_wake_status = format!(
                    "Applied {applied} NWC wake registration{}",
                    if applied == 1 { "" } else { "s" }
                );
                if let Some(retry_at) = retry_at {
                    self.schedule_push_retry(retry_at);
                }
            }
            ApplicationRegistrationCompletion::Idle {
                retry_at: Some(retry_at),
            } => self.schedule_push_retry(retry_at),
            ApplicationRegistrationCompletion::Idle { retry_at: None } => {}
            _ => {}
        }
    }

    fn schedule_push_retry(&mut self, next_attempt_at: u64) {
        let Some(manager) = self.manager.as_mut() else {
            return;
        };
        let nonce = manager.schedule_registration_retry();
        let delay = registration_retry_delay(
            next_attempt_at,
            UnixTimestamp::from_secs(crate::time::now_unix()),
        );
        let tx = self.tx.clone();
        self.runtime.spawn(async move {
            tokio::time::sleep(delay).await;
            let _ = tx.send(CoreMsg::Async(AsyncMsg::NwcPushRetryDue { nonce }));
        });
    }

    pub(crate) fn handle_push_retry_due(&mut self, context: &mut NwcAppContext<'_>, nonce: u64) {
        if self
            .manager
            .as_ref()
            .is_some_and(|manager| manager.registration_retry_is_current(nonce))
        {
            self.sync_push_registrations(context);
        }
    }

    pub(crate) fn foregrounded(&mut self, context: &mut NwcAppContext<'_>) {
        self.refresh_connection_usage(context);
        self.publish_pending_info_events(context);
        self.sync_push_registrations(context);
        self.prefetch_icons(context);
    }

    pub(crate) fn enqueue_wake_requests(
        &mut self,
        context: &mut NwcAppContext<'_>,
        requests: Vec<NwcWakeRequest>,
    ) {
        let mut added = 0usize;
        for request in requests {
            if !self.wake_request_is_known(context, &request.event_id_hex) {
                context.state.nwc.pending_wake_requests.push(request);
                added += 1;
            }
        }
        Self::cap_pending_wake_requests(context);
        context.state.nwc.last_wake_status = match added {
            0 => "No new NWC wake requests".to_string(),
            1 => "Queued 1 NWC wake request".to_string(),
            count => format!("Queued {count} NWC wake requests"),
        };
        self.process_pending_wake_requests(context);
    }

    fn cap_pending_wake_requests(context: &mut NwcAppContext<'_>) {
        let len = context.state.nwc.pending_wake_requests.len();
        if len > MAX_NWC_WAKE_HISTORY {
            context
                .state
                .nwc
                .pending_wake_requests
                .drain(0..len - MAX_NWC_WAKE_HISTORY);
        }
    }

    fn cap_processed_wake_requests(context: &mut NwcAppContext<'_>) {
        let len = context.state.nwc.processed_wake_requests.len();
        if len > MAX_NWC_WAKE_HISTORY {
            context
                .state
                .nwc
                .processed_wake_requests
                .drain(0..len - MAX_NWC_WAKE_HISTORY);
        }
    }

    fn wake_request_is_known(&self, context: &NwcAppContext<'_>, event_id: &str) -> bool {
        context
            .state
            .nwc
            .pending_wake_requests
            .iter()
            .any(|request| request.event_id_hex == event_id)
            || context
                .state
                .nwc
                .processed_wake_requests
                .iter()
                .any(|request| request.event_id_hex == event_id)
            || self.wake_coordinator.is_in_flight(&event_id.to_string())
    }

    pub(crate) fn process_pending_wake_requests(&mut self, context: &mut NwcAppContext<'_>) {
        if self.manager.is_none() {
            context.state.nwc.last_wake_status =
                "NWC wake queued: authorization storage is unavailable".to_string();
            return;
        }
        let Some(request) = context
            .state
            .nwc
            .pending_wake_requests
            .iter()
            .find(|request| !self.wake_coordinator.is_in_flight(&request.event_id_hex))
            .cloned()
        else {
            return;
        };

        let Some(wallet) = context.wallet.clone() else {
            context.state.nwc.last_wake_status =
                "NWC wake queued: wallet is not open yet".to_string();
            return;
        };
        if self.service_keys().is_err() {
            context.state.nwc.last_wake_status =
                "NWC wake queued: Nostr key is not available".to_string();
            return;
        }

        self.wake_coordinator.begin(request.event_id_hex.clone());
        self.process_wake_request(request, wallet, context.wallet_generation);
    }

    fn process_wake_request(&self, request: NwcWakeRequest, wallet: Wallet, generation: u64) {
        let tx = self.tx.clone();
        let data_dir = self.data_dir.clone();
        let secrets = self.secrets.clone();
        let monitor_config = self.settlement_monitor_config.clone();
        self.runtime.spawn(async move {
            let event_id = request.event_id_hex.clone();
            let result = async {
                let wake = WakeEnvelope::new(
                    request.relay_url.clone(),
                    request.event_id_hex.clone(),
                    request.wallet_service_public_key_hex.clone(),
                    request.embedded_event_json.clone(),
                    request.received_at_seconds,
                )
                .validate()
                .context("invalid NWC wake envelope")?;
                let provider = opened_bark_provider(wallet);
                let config = nwc_mobile_config(&data_dir, provider, secrets, monitor_config);
                let mobile = NwcMobile::open(config).context("NWC ledger is unavailable")?;
                let outcome = mobile
                    .execute_wake(
                        wake,
                        NwcMobileWakeKind::from_settlement_check(request.settlement_check),
                        NWC_FOREGROUND_OPERATION_TIMEOUT,
                        &NeverCancelled,
                    )
                    .await;
                Ok::<_, anyhow::Error>(outcome.disposition())
            }
            .await;

            let msg = match result {
                Ok(disposition) => AsyncMsg::NwcWakeEngineFinished {
                    generation,
                    request,
                    disposition,
                },
                Err(error) => AsyncMsg::NwcWakeRequestFailed {
                    generation,
                    event_id,
                    error: format!("{error:#}"),
                },
            };
            let _ = tx.send(CoreMsg::Async(msg));
        });
    }

    pub(crate) fn reconcile_payment_notifications(&self, context: &NwcAppContext<'_>) {
        let Some(wallet) = context.wallet.clone() else {
            return;
        };
        let Ok(keys) = self.service_keys() else {
            return;
        };
        let Ok(wallet_service_pubkey) =
            nwc_mobile::PublicKey::from_hex(&keys.public_key().to_hex())
        else {
            return;
        };
        let data_dir = self.data_dir.clone();
        let secrets = self.secrets.clone();
        self.runtime.spawn(async move {
            let config = nwc_mobile_config(&data_dir, opened_bark_provider(wallet), secrets, None);
            let Ok(mobile) = NwcMobile::open(config) else {
                return;
            };
            let _ = mobile
                .reconcile_notifications(
                    wallet_service_pubkey,
                    NWC_FOREGROUND_OPERATION_TIMEOUT,
                    &NeverCancelled,
                )
                .await;
        });
    }

    pub(crate) fn finish_wake_engine(
        &mut self,
        context: &mut NwcAppContext<'_>,
        generation: u64,
        request: NwcWakeRequest,
        disposition: WakeDisposition,
    ) {
        let decision = self
            .wake_coordinator
            .handle_disposition(&request.event_id_hex, disposition);
        match decision {
            ForegroundWakeDecision::Finished(outcome) => match outcome {
                ForegroundWakeOutcome::Completed => {
                    self.finish_wake(context, request, "completed", true)
                }
                ForegroundWakeOutcome::AlreadyProcessed => {
                    self.finish_wake(context, request, "already_processed", false)
                }
                ForegroundWakeOutcome::Rejected(code) => {
                    self.finish_wake(context, request, &format!("rejected:{code:?}"), false)
                }
                ForegroundWakeOutcome::RetryExhausted => {
                    self.finish_wake(context, request, "retry_exhausted", false)
                }
                _ => self.finish_wake(context, request, "unsupported_disposition", false),
            },
            ForegroundWakeDecision::Retry { delay, cause } => {
                context.state.nwc.last_wake_status = match cause {
                    ForegroundWakeRetryCause::Engine(reason) => {
                        format!("NWC wake retry scheduled: {reason:?}")
                    }
                    ForegroundWakeRetryCause::QueuedForApplication(reason) => {
                        format!("NWC wake queued: {reason:?}")
                    }
                    _ => "NWC wake retry scheduled".to_string(),
                };
                self.schedule_wake_retry(generation, request.event_id_hex, delay);
                self.process_pending_wake_requests(context);
            }
            _ => self.finish_wake(context, request, "unsupported_disposition", false),
        }
    }

    fn finish_wake(
        &mut self,
        context: &mut NwcAppContext<'_>,
        request: NwcWakeRequest,
        status: &str,
        success: bool,
    ) {
        self.wake_coordinator.forget(&request.event_id_hex);
        context
            .state
            .nwc
            .pending_wake_requests
            .retain(|pending| pending.event_id_hex != request.event_id_hex);
        context.state.nwc.last_wake_status = format!("NWC wake {status}.");
        context
            .state
            .nwc
            .processed_wake_requests
            .push(NwcProcessedWakeRequest {
                relay_url: request.relay_url,
                event_id_hex: request.event_id_hex,
                client_public_key_hex: String::new(),
                method: "request".to_string(),
                status: status.to_string(),
                amount_sat: 0,
                received_at_seconds: request.received_at_seconds,
                processed_at_seconds: crate::time::now_unix(),
            });
        Self::cap_processed_wake_requests(context);
        self.refresh_connection_usage(context);
        if success {
            self.request_haptic(HapticFeedback::NotificationSuccess);
        } else if status.starts_with("rejected")
            || matches!(status, "unsupported_disposition" | "retry_exhausted")
        {
            self.request_haptic(HapticFeedback::NotificationWarning);
        }
        self.process_pending_wake_requests(context);
    }

    pub(crate) fn fail_wake_request(
        &mut self,
        context: &mut NwcAppContext<'_>,
        event_id: String,
        error: String,
    ) {
        self.wake_coordinator.forget(&event_id);
        context.state.nwc.last_wake_status = format!("NWC wake failed: {error}");
        context
            .state
            .nwc
            .pending_wake_requests
            .retain(|request| request.event_id_hex != event_id);
        self.request_haptic(HapticFeedback::NotificationWarning);
        self.process_pending_wake_requests(context);
    }

    pub(crate) fn retry_wake_request(&mut self, context: &mut NwcAppContext<'_>, event_id: String) {
        self.wake_coordinator.retry_due(&event_id);
        self.process_pending_wake_requests(context);
    }

    fn schedule_wake_retry(&self, generation: u64, event_id: String, delay: Duration) {
        let tx = self.tx.clone();
        self.runtime.spawn(async move {
            tokio::time::sleep(delay).await;
            let _ = tx.send(CoreMsg::Async(AsyncMsg::NwcWakeRetryDue {
                generation,
                event_id,
            }));
        });
    }

    pub(crate) fn finish_info_event(
        &mut self,
        context: &mut NwcAppContext<'_>,
        client_pubkey: String,
        relay: String,
        error: Option<String>,
    ) {
        self.in_flight_info_events
            .remove(&nwc_info_event_key(&client_pubkey, &relay));
        if let Some(error) = error {
            context.state.nwc.last_wake_status =
                format!("NWC info event failed on {relay}: {error}");
            return;
        }
        if let Some(connection) = context
            .state
            .nwc
            .connections
            .iter_mut()
            .find(|connection| connection.client_pubkey == client_pubkey)
        {
            if let Some(manager) = self.manager.as_ref() {
                let _ = manager.acknowledge_nwc_info_event(&connection.id, &relay);
            }
            connection
                .pending_info_event_relays
                .retain(|pending_relay| pending_relay != &relay);
        }
        context.state.nwc.last_wake_status = format!("NWC info event published to {relay}");
    }

    pub(crate) fn remove_wallet_data(
        &mut self,
        context: &NwcAppContext<'_>,
    ) -> (NwcWalletCleanup, Vec<String>) {
        let cleanup = NwcWalletCleanup {
            secrets: context
                .state
                .nwc
                .connections
                .iter()
                .map(|connection| (connection.client_pubkey.clone(), connection.name.clone()))
                .collect(),
        };
        self.manager = None;
        let database_path = NwcApplicationManager::database_path(&self.data_dir);
        let errors = remove_wallet_database_files(&database_path)
            .err()
            .map(|error| vec![format!("{error:#}")])
            .unwrap_or_default();
        (cleanup, errors)
    }

    pub(crate) fn reset_after_wallet_deletion(&mut self) -> Vec<String> {
        self.wake_coordinator.reset();
        self.manager = NwcApplicationManager::open(&self.data_dir).ok();
        if self.manager.is_none() {
            vec!["could not reopen NWC authorization storage".to_string()]
        } else {
            Vec::new()
        }
    }

    pub(crate) fn delete_wallet_secrets(&self, cleanup: NwcWalletCleanup) -> Vec<String> {
        cleanup
            .secrets
            .into_iter()
            .filter_map(|(client_pubkey, name)| {
                (!self
                    .secrets
                    .delete_secret(nwc_client_secret_key(&client_pubkey)))
                .then(|| format!("NWC secret for {name}"))
            })
            .collect()
    }
}

fn cached_nwc_icon_url(cache: &ApplicationIconCache, remote_url: &str) -> Option<String> {
    let remote_url = ApplicationIconUrl::parse(remote_url).ok()?;
    cache.cached_file_url(&remote_url).ok().flatten()
}

// Existing Rebel clients advertise NIP-04. This must change atomically with
// connection migration and info-event advertisement.
pub(crate) const NWC_ENCRYPTION: NwcEncryption = NwcEncryption::LegacyNip04;

pub(crate) async fn publish_nwc_info_event(
    relay: String,
    keys: Keys,
    client_pubkey: Option<NostrPublicKey>,
    permissions: Vec<NwcPermission>,
) -> anyhow::Result<()> {
    let client_pubkey = client_pubkey
        .map(|key| nwc_mobile::PublicKey::from_hex(&key.to_hex()))
        .transpose()
        .context("invalid NWC client public key")?;
    let secret = NwcSecretKey::from_bytes(keys.secret_key().to_secret_bytes())
        .context("invalid NWC wallet service key")?;
    let approved_methods = permissions
        .into_iter()
        .map(NwcMethod::from)
        .collect::<BTreeSet<_>>();
    let methods = standard_nwc_methods()
        .into_iter()
        .filter(|method| client_pubkey.is_none() || approved_methods.contains(method))
        .collect();
    nwc_mobile_nostr::publish_nwc_info_event_with_notifications(
        &relay,
        &secret,
        client_pubkey.as_ref(),
        methods,
        vec![
            NwcNotificationType::PaymentReceived,
            NwcNotificationType::PaymentSent,
        ],
        NWC_ENCRYPTION,
        INFO_PUBLISH_TIMEOUT,
    )
    .await
    .context("failed to publish NWC info event")
}

pub(crate) type RebelSecretProvider = StoredNwcSecrets<dyn SecretStore>;

pub(crate) fn rebel_secret_provider(secrets: Arc<dyn SecretStore>) -> RebelSecretProvider {
    StoredNwcSecrets::new(secrets, NOSTR_SECRET_KEY)
}

impl ProtectedSecretStore for dyn SecretStore {
    fn load_secret(&self, key: &str) -> Result<Option<String>, HostError> {
        Ok(SecretStore::get_secret(self, key.to_owned()))
    }

    fn store_secret(&self, key: &str, value: &str) -> Result<(), HostError> {
        SecretStore::set_secret(self, key.to_owned(), value.to_owned())
            .then_some(())
            .ok_or_else(unavailable)
    }

    fn delete_secret(&self, key: &str) -> Result<(), HostError> {
        SecretStore::delete_secret(self, key.to_owned())
            .then_some(())
            .ok_or_else(unavailable)
    }
}

const fn unavailable() -> HostError {
    HostError::new(HostErrorKind::Unavailable)
}

pub(crate) fn nwc_mobile_config<P>(
    data_dir: impl AsRef<Path>,
    provider: P,
    secrets: Arc<dyn SecretStore>,
    settlement_monitor: Option<InvoiceSettlementMonitorConfig>,
) -> NwcMobileConfig
where
    P: LightningNodeProvider + 'static,
{
    let secret_provider = rebel_secret_provider(secrets);
    let mut config = NwcMobileConfig::new(
        data_dir,
        provider,
        NostrRelayTransport,
        secret_provider.clone(),
    );
    if let Some(monitor) = settlement_monitor {
        config = config.with_completion_handler(
            InvoiceSettlementCompletion::new(monitor, secret_provider),
            SETTLEMENT_MONITOR_RESERVE,
        );
    }
    config
}

pub(crate) async fn run_registration_worker(
    ledger: &nwc_mobile::WakeLedger,
    config: nwc_mobile_http::ReadyApnsWakeRegistrationConfig,
    keys: Keys,
) -> anyhow::Result<nwc_mobile_http::RegistrationPass> {
    let signing_key = Nip98SigningKey::from_bytes(keys.secret_key().to_secret_bytes())
        .context("invalid wake registration signing key")?;
    nwc_mobile_http::run_registration_worker(ledger, config, signing_key)
        .await
        .context("wake registration outbox pass failed")
}

/// Rebel-specific wallet bootstrap around the shared native wake contract.
#[derive(uniffi::Object)]
pub struct NwcExtensionEngine {
    data_dir: PathBuf,
    secrets: Arc<dyn SecretStore>,
    settlement_monitor_config: Option<InvoiceSettlementMonitorConfig>,
}

#[uniffi::export]
impl NwcExtensionEngine {
    #[uniffi::constructor]
    pub fn new(
        data_dir: String,
        secret_store: Box<dyn SecretStore>,
        wake_server_url: Option<String>,
        install_id: String,
    ) -> Arc<Self> {
        Arc::new(Self {
            data_dir: PathBuf::from(data_dir),
            secrets: Arc::from(secret_store),
            settlement_monitor_config: InvoiceSettlementMonitorConfig::new(
                wake_server_url,
                install_id,
            )
            .ok(),
        })
    }

    pub async fn execute_wake(
        &self,
        request: MobileWakeEnvelope,
        execution_milliseconds: u64,
        cancellation: Arc<MobileCancellation>,
    ) -> NwcExtensionWakeExecution {
        let data_dir = self.data_dir.clone();
        let config = nwc_mobile_config(
            &data_dir,
            cold_bark_provider(data_dir.clone(), self.secrets.clone()),
            self.secrets.clone(),
            self.settlement_monitor_config.clone(),
        );
        execute_native_extension_wake(config, request, execution_milliseconds, cancellation).await
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use nostr_sdk::prelude::Keys;

    use super::*;
    use crate::{NwcConnection, NwcPermission};

    struct EmptySecretStore;

    impl SecretStore for EmptySecretStore {
        fn get_secret(&self, _key: String) -> Option<String> {
            None
        }

        fn set_secret(&self, _key: String, _value: String) -> bool {
            true
        }

        fn delete_secret(&self, _key: String) -> bool {
            true
        }
    }

    struct TestHarness {
        _data_dir: tempfile::TempDir,
        _cache_dir: tempfile::TempDir,
        _runtime: tokio::runtime::Runtime,
        controller: NwcController,
        state: AppState,
    }

    impl TestHarness {
        fn new() -> Self {
            let data_dir = tempfile::tempdir().expect("data dir");
            let cache_dir = tempfile::tempdir().expect("cache dir");
            let runtime = tokio::runtime::Runtime::new().expect("runtime");
            let (tx, _rx) = flume::unbounded();
            let controller = NwcController::new(
                data_dir.path().to_path_buf(),
                cache_dir.path(),
                Arc::new(EmptySecretStore),
                tx,
                runtime.handle().clone(),
                Arc::new(tokio::sync::Semaphore::new(1)),
            );
            Self {
                _data_dir: data_dir,
                _cache_dir: cache_dir,
                _runtime: runtime,
                controller,
                state: AppState::initial(),
            }
        }

        fn with_nwc<R>(
            &mut self,
            operation: impl FnOnce(&mut NwcController, &mut NwcAppContext<'_>) -> R,
        ) -> R {
            let mut context = NwcAppContext {
                state: &mut self.state,
                wallet: None,
                wallet_generation: 0,
                rev: 0,
            };
            operation(&mut self.controller, &mut context)
        }
    }

    fn test_nwc_connection(client_pubkey: &str) -> NwcConnection {
        NwcConnection {
            id: format!("nwc-{client_pubkey}"),
            name: "Test".to_string(),
            icon_url: None,
            icon_display_url: None,
            relay: "wss://relay.example.com".to_string(),
            wallet_managed_secret: true,
            service_pubkey: "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
                .to_string(),
            client_pubkey: client_pubkey.to_string(),
            budget_sat: 0,
            spent_sat: 0,
            budget_display: String::new(),
            spent_display: String::new(),
            budget_interval: NwcBudgetInterval::Never,
            budget_interval_display: String::new(),
            permissions: vec![NwcPermission::GetInfo],
            created_at: 0,
            last_used_at: None,
            expires_at: None,
            budget_period_started_at: 0,
            pending_info_event_relays: Vec::new(),
        }
    }

    fn test_nwc_wake_request() -> NwcWakeRequest {
        NwcWakeRequest {
            relay_url: "wss://relay.example.com".to_string(),
            event_id_hex: "event".to_string(),
            wallet_service_public_key_hex: "wallet".to_string(),
            embedded_event_json: None,
            received_at_seconds: 100,
            settlement_check: false,
        }
    }

    #[test]
    fn relay_input_validation_matches_creation_policy() {
        assert!(nwc_relay_input_is_valid("wss://relay.example/path/"));
        assert!(nwc_relay_input_is_valid(
            "wss://relay.example/one\nwss://relay.example/two"
        ));
        assert!(!nwc_relay_input_is_valid(""));
        assert!(!nwc_relay_input_is_valid("ws://relay.example"));
        assert!(!nwc_relay_input_is_valid(
            "wss://relay.example/1,wss://relay.example/2,wss://relay.example/3"
        ));
    }

    #[test]
    fn push_registration_retry_delay_has_a_floor() {
        assert_eq!(
            registration_retry_delay(100, UnixTimestamp::from_secs(100)),
            Duration::from_secs(5)
        );
        assert_eq!(
            registration_retry_delay(99, UnixTimestamp::from_secs(100)),
            Duration::from_secs(5)
        );
        assert_eq!(
            registration_retry_delay(110, UnixTimestamp::from_secs(100)),
            Duration::from_secs(10)
        );
    }

    #[test]
    fn pending_wake_processing_fails_closed_until_service_is_ready() {
        let mut harness = TestHarness::new();
        harness.controller.manager = None;
        harness
            .state
            .nwc
            .pending_wake_requests
            .push(test_nwc_wake_request());

        harness.with_nwc(|controller, context| controller.process_pending_wake_requests(context));

        assert!(!harness
            .controller
            .wake_coordinator
            .is_in_flight(&"event".to_string()));
        assert!(harness
            .state
            .nwc
            .last_wake_status
            .contains("authorization storage is unavailable"));
    }

    #[test]
    fn loads_connections_from_the_authoritative_nwc_mobile_ledger() {
        let mut harness = TestHarness::new();
        let service_keys = Keys::generate();
        let client_keys = Keys::generate();
        let mut connection = test_nwc_connection(&client_keys.public_key().to_hex());
        connection.service_pubkey = service_keys.public_key().to_hex();
        connection.name = "Stored display name".to_string();
        let authorization = nwc_mobile::HostConnectionAuthorization::new(
            connection.id.clone(),
            connection.client_pubkey.clone(),
            connection.service_pubkey.clone(),
            vec![connection.relay.clone()],
            vec![nwc_mobile::NwcMethod::GetInfo],
            connection.budget_sat,
            connection.budget_interval.into(),
            nwc_mobile::FeePolicy::CountTowardBudget {
                maximum_fee_sat: nwc_mobile::maximum_mobile_fee_sat(connection.budget_sat),
            },
            NWC_ENCRYPTION,
            None,
        );
        harness
            .controller
            .manager
            .as_ref()
            .expect("manager")
            .service()
            .create_host_connection(authorization)
            .expect("persist connection");
        harness
            .controller
            .manager
            .as_ref()
            .expect("manager")
            .set_connection_metadata(
                &connection.id,
                ApplicationConnectionMetadata::new(
                    connection.name.clone(),
                    None,
                    vec![connection.relay.clone()],
                )
                .expect("metadata"),
            )
            .expect("persist metadata");

        harness.with_nwc(|controller, context| controller.load_connections(context));

        assert_eq!(harness.state.nwc.connections.len(), 1);
        let loaded = &harness.state.nwc.connections[0];
        assert_eq!(loaded.name, "Stored display name");
        assert_eq!(loaded.client_pubkey, client_keys.public_key().to_hex());
        assert_eq!(loaded.service_pubkey, service_keys.public_key().to_hex());
        assert!(!loaded.wallet_managed_secret);
    }

    #[test]
    fn completed_engine_wake_leaves_the_queue_and_enters_history() {
        let mut harness = TestHarness::new();
        let request = test_nwc_wake_request();
        harness
            .state
            .nwc
            .pending_wake_requests
            .push(request.clone());
        harness
            .controller
            .wake_coordinator
            .begin(request.event_id_hex.clone());

        harness.with_nwc(|controller, context| {
            controller.finish_wake_engine(
                context,
                0,
                request,
                WakeDisposition::Completed {
                    notification: nwc_mobile::NotificationHint::Completed,
                },
            )
        });

        assert!(harness.state.nwc.pending_wake_requests.is_empty());
        assert!(!harness
            .controller
            .wake_coordinator
            .is_in_flight(&"event".to_string()));
        assert_eq!(harness.state.nwc.processed_wake_requests.len(), 1);
        assert_eq!(
            harness.state.nwc.processed_wake_requests[0].status,
            "completed"
        );
    }

    #[test]
    fn retryable_engine_wake_remains_owned_and_queued() {
        let mut harness = TestHarness::new();
        let request = test_nwc_wake_request();
        harness
            .state
            .nwc
            .pending_wake_requests
            .push(request.clone());
        harness
            .controller
            .wake_coordinator
            .begin(request.event_id_hex.clone());

        harness.with_nwc(|controller, context| {
            controller.finish_wake_engine(
                context,
                0,
                request,
                WakeDisposition::RetryAfter {
                    delay: Duration::from_secs(60),
                    reason: nwc_mobile::RetryReason::RelayUnavailable,
                    notification: nwc_mobile::NotificationHint::Processing,
                },
            )
        });

        assert_eq!(harness.state.nwc.pending_wake_requests.len(), 1);
        assert!(harness
            .controller
            .wake_coordinator
            .is_in_flight(&"event".to_string()));
        assert_eq!(
            harness
                .controller
                .wake_coordinator
                .retry_attempts(&"event".to_string()),
            1
        );
        assert!(harness.state.nwc.processed_wake_requests.is_empty());
    }

    #[test]
    fn exhausted_wake_retries_leave_the_queue_and_enter_history() {
        let mut harness = TestHarness::new();
        let request = test_nwc_wake_request();
        harness
            .state
            .nwc
            .pending_wake_requests
            .push(request.clone());
        harness
            .controller
            .wake_coordinator
            .begin(request.event_id_hex.clone());
        for _ in 0..nwc_mobile::DEFAULT_FOREGROUND_WAKE_RETRY_ATTEMPTS {
            let _ = harness.controller.wake_coordinator.handle_disposition(
                &request.event_id_hex,
                WakeDisposition::RetryAfter {
                    delay: Duration::from_secs(1),
                    reason: nwc_mobile::RetryReason::WalletUnavailable,
                    notification: nwc_mobile::NotificationHint::Processing,
                },
            );
        }

        harness.with_nwc(|controller, context| {
            controller.finish_wake_engine(
                context,
                0,
                request,
                WakeDisposition::QueuedForApplication {
                    reason: nwc_mobile::QueueReason::WalletUnavailable,
                    notification: nwc_mobile::NotificationHint::OpenApplication,
                },
            )
        });

        assert!(harness.state.nwc.pending_wake_requests.is_empty());
        assert!(!harness
            .controller
            .wake_coordinator
            .is_in_flight(&"event".to_string()));
        assert_eq!(
            harness
                .controller
                .wake_coordinator
                .retry_attempts(&"event".to_string()),
            0
        );
        assert_eq!(harness.state.nwc.processed_wake_requests.len(), 1);
        assert_eq!(
            harness.state.nwc.processed_wake_requests[0].status,
            "retry_exhausted"
        );
        assert!(harness
            .controller
            .take_output()
            .haptics
            .contains(&HapticFeedback::NotificationWarning));
    }
}
