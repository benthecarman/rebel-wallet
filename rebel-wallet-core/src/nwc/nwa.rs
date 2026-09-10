//! Rebel Wallet's Nostr Wallet Auth application flow.

use anyhow::Context;
use nwc_mobile::{
    ApplicationConnectionMetadata, MobileServiceError, NwaApprovalError, NwaCallbackBegin,
    NwaCallbackCompletion, UnixTimestamp,
};
use nwc_mobile_uniffi::MobileConnectionMetadata;

use crate::updates::{AppUpdate, HapticFeedback};
use crate::{NwaRequestState, NwcBudgetInterval, NwcConnection, NwcPermission, Screen, SetupState};

use super::nwc_mobile::{NwcAppContext, NwcController, NWC_ENCRYPTION};

impl NwcController {
    pub(crate) fn open_nwa_request(&mut self, context: &mut NwcAppContext<'_>, uri: String) {
        let Some(manager) = self.manager.as_ref() else {
            self.set_nwa_error(context, "NWC authorization storage is unavailable.");
            return;
        };
        match manager.open_nwa_request(&uri) {
            Ok(request) => {
                let state = match NwaRequestState::try_from(request)
                    .context("shared NWA presentation is not representable by the native contract")
                {
                    Ok(state) => state,
                    Err(error) => {
                        context.state.toast =
                            Some(format!("Nostr Wallet Auth request rejected: {error:#}"));
                        self.request_haptic(HapticFeedback::NotificationError);
                        return;
                    }
                };
                let icon_url = state.icon_url.clone();
                context.state.nwa.request = Some(state);
                context.state.nwa.approving = false;
                context.state.nwa.error_message = None;
                context.state.nwa.callback_pending = false;
                context.state.nwa.icon_display_url = None;
                if let Some(icon_url) = icon_url {
                    context.state.nwa.icon_display_url = self.icon_display_url(Some(&icon_url));
                    self.prefetch_icon(context, icon_url);
                }
                if context.state.setup == SetupState::Ready
                    && context.state.router.screen_stack.last() != Some(&Screen::Nwc)
                {
                    context.state.router.screen_stack.push(Screen::Nwc);
                }
            }
            Err(MobileServiceError::NwaAlreadyPending) => {
                context.state.toast = Some(
                    "Finish or cancel the current Nostr Wallet Auth request before opening another."
                        .to_string(),
                );
                self.request_haptic(HapticFeedback::NotificationWarning);
            }
            Err(error) => {
                context.state.toast =
                    Some(format!("Nostr Wallet Auth request rejected: {error:#}"));
                self.request_haptic(HapticFeedback::NotificationError);
            }
        }
    }

    pub(crate) fn approve_nwa_request(
        &mut self,
        context: &mut NwcAppContext<'_>,
        relay: String,
        budget_sat: u64,
        budget_interval: NwcBudgetInterval,
        permissions: Vec<NwcPermission>,
    ) {
        if context.state.nwa.approving {
            return;
        }
        let Some(request) = context.state.nwa.request.clone() else {
            self.set_nwa_error(
                context,
                "The Nostr Wallet Auth request is no longer available.",
            );
            return;
        };
        if self.manager.is_none() {
            self.set_nwa_error(context, "NWC authorization storage is unavailable.");
            return;
        }
        let service_keys = match self.service_keys() {
            Ok(keys) => keys,
            Err(_) => {
                self.set_nwa_error(context, "Create or open the wallet before adding NWC.");
                return;
            }
        };
        context.state.nwa.approving = true;
        context.state.nwa.error_message = None;
        let selection = nwc_mobile::NwaApprovalSelection::new(
            request.request_id_hex.clone(),
            service_keys.public_key().to_hex(),
            relay,
            context.state.nwc.default_relay.clone(),
            permissions.into_iter().map(Into::into).collect(),
            budget_sat,
            budget_interval.into(),
            NWC_ENCRYPTION,
            request.expires_at.map(UnixTimestamp::from_secs),
            context.state.lightning_address.address.clone(),
        );
        let approved = self
            .manager
            .as_mut()
            .expect("checked above")
            .approve_nwa(selection);
        let (approved, callback) = match approved {
            Ok(approved) => approved.into_parts(),
            Err(nwc_mobile::ApplicationWorkflowError::Service(
                MobileServiceError::NwaApproval(NwaApprovalError::AuthorityEscalation),
            )) => {
                self.set_nwa_error(context, "The approval exceeds the requested authority.");
                return;
            }
            Err(error) => {
                self.set_nwa_error(
                    context,
                    &format!("Could not approve NWC authorization: {error:#}"),
                );
                return;
            }
        };
        self.finish_nwa_approval(context, request, approved, callback);
    }

    fn finish_nwa_approval(
        &mut self,
        context: &mut NwcAppContext<'_>,
        request: NwaRequestState,
        approved: nwc_mobile::ApprovedApplicationConnection,
        callback: NwaCallbackBegin,
    ) {
        let icon_display_url = self.icon_display_url(request.icon_url.as_deref());
        let metadata = ApplicationConnectionMetadata::new(
            request.display_name.clone(),
            request.icon_url.clone(),
            approved.draft().authorization().relay_urls().to_vec(),
        );
        let stored = metadata.and_then(|metadata| {
            self.manager
                .as_ref()
                .expect("manager exists during approval")
                .set_connection_metadata(approved.draft().id(), metadata)
                .map_err(|_| nwc_mobile::RegistryError::InvalidConnection)
        });
        if stored.is_err() {
            let _ = self
                .manager
                .as_ref()
                .expect("manager exists during approval")
                .service()
                .revoke_host_connection(approved.draft().id());
            self.set_nwa_error(context, "Could not persist NWC connection metadata.");
            return;
        }
        let connection = NwcConnection::from_approved(
            &approved,
            MobileConnectionMetadata {
                name: request.display_name,
                icon_url: request.icon_url,
                icon_display_url,
                wallet_managed_secret: false,
            },
        )
        .expect("shared NWA approval must produce a native connection view");
        context.state.nwc.default_relay = connection.relay.clone();
        context.state.nwc.connections.push(connection);
        context.state.nwa.approving = false;
        context.state.nwa.error_message = None;
        self.publish_pending_info_events(context);
        self.sync_push_registrations(context);
        self.request_haptic(HapticFeedback::NotificationSuccess);

        match callback {
            NwaCallbackBegin::OpenUrl(url) => {
                context.state.nwa.callback_pending = true;
                self.push_side_effect(AppUpdate::OpenUrl {
                    rev: context.rev + 1,
                    url,
                });
            }
            NwaCallbackBegin::Complete => {
                context.state.toast = Some("NWC client authorized.".to_string());
                self.clear_nwa_request(context);
            }
            _ => self.set_nwa_error(context, "Unsupported NWA callback action."),
        }
    }

    pub(crate) fn retry_nwa_callback(&mut self, context: &mut NwcAppContext<'_>) {
        let Some(url) = self
            .manager
            .as_ref()
            .and_then(nwc_mobile::NwcApplicationManager::retry_nwa_callback)
        else {
            self.set_nwa_error(context, "There is no callback to retry.");
            return;
        };
        context.state.nwa.error_message = None;
        self.push_side_effect(AppUpdate::OpenUrl {
            rev: context.rev + 1,
            url,
        });
    }

    pub(crate) fn complete_nwa_callback_open(
        &mut self,
        context: &mut NwcAppContext<'_>,
        opened: bool,
    ) {
        let Some(manager) = self.manager.as_mut() else {
            return;
        };
        match manager.complete_nwa_callback(opened) {
            NwaCallbackCompletion::Ignored => {}
            NwaCallbackCompletion::Complete => self.clear_nwa_request(context),
            NwaCallbackCompletion::RetryAvailable => {
                context.state.nwa.callback_pending = true;
                self.set_nwa_error(
                    context,
                    "The connection was approved, but the requesting app could not be reopened. Return to it manually or retry.",
                );
            }
            _ => {}
        }
    }

    pub(crate) fn cancel_nwa_request(&mut self, context: &mut NwcAppContext<'_>) {
        if !context.state.nwa.approving {
            self.clear_nwa_request(context);
        }
    }

    fn set_nwa_error(&mut self, context: &mut NwcAppContext<'_>, message: &str) {
        context.state.nwa.approving = false;
        context.state.nwa.error_message = Some(message.to_string());
        self.request_haptic(HapticFeedback::NotificationError);
    }

    fn clear_nwa_request(&mut self, context: &mut NwcAppContext<'_>) {
        if let Some(manager) = self.manager.as_mut() {
            let _ = manager.cancel_nwa();
        }
        context.state.nwa = crate::NwaState::default();
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use tempfile::TempDir;

    use super::*;
    use crate::{AppState, SecretStore};

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

    fn test_controller() -> (
        TempDir,
        TempDir,
        tokio::runtime::Runtime,
        NwcController,
        AppState,
    ) {
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
        (
            data_dir,
            cache_dir,
            runtime,
            controller,
            AppState::initial(),
        )
    }

    #[test]
    fn inbound_nwa_cannot_replace_the_request_being_reviewed() {
        const CLIENT: &str = "687dd8ece211539364549b1f32c63eceec1e0661009ba65cf8ff2e73ba000746";
        let (_data_dir, _cache_dir, _runtime, mut controller, mut state) = test_controller();
        let mut context = NwcAppContext {
            state: &mut state,
            wallet: None,
            wallet_generation: 0,
            rev: 0,
        };
        controller.open_nwa_request(
            &mut context,
            format!("nostr+walletauth://{CLIENT}?relay=wss%3A%2F%2Frelay.example.com&name=First"),
        );
        let first = context.state.nwa.request.clone().expect("first request");

        controller.open_nwa_request(
            &mut context,
            format!("nostr+walletauth://{CLIENT}?relay=wss%3A%2F%2Frelay.example.com&name=Second"),
        );

        let current = context.state.nwa.request.as_ref().expect("current request");
        assert_eq!(current.request_id_hex, first.request_id_hex);
        assert_eq!(current.display_name, "First");
        assert!(context
            .state
            .toast
            .as_deref()
            .is_some_and(|message| message.contains("current Nostr Wallet Auth request")));
    }

    #[test]
    fn cancelling_nwa_never_opens_the_requester_callback() {
        const CLIENT: &str = "687dd8ece211539364549b1f32c63eceec1e0661009ba65cf8ff2e73ba000746";
        const STATE: &str = "0123456789abcdef0123456789abcdef";
        let (_data_dir, _cache_dir, _runtime, mut controller, mut state) = test_controller();
        let mut context = NwcAppContext {
            state: &mut state,
            wallet: None,
            wallet_generation: 0,
            rev: 0,
        };
        controller.open_nwa_request(
            &mut context,
            format!(
                "nostr+walletauth://{CLIENT}?relay=wss%3A%2F%2Frelay.example.com&return_to=https%3A%2F%2Fapp.example.com%2Fnwa&state={STATE}"
            ),
        );
        let _ = controller.take_output();

        controller.cancel_nwa_request(&mut context);

        assert!(controller
            .manager
            .as_ref()
            .expect("manager")
            .service()
            .pending_nwa_request()
            .expect("pending request")
            .is_none());
        assert!(controller.take_output().side_effects.is_empty());
    }
}
