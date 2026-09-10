use std::time::Duration;

pub(super) const WALLET_WORK_TIMEOUT: Duration = Duration::from_secs(90);
pub(super) const FOREGROUND_MAINTENANCE_INTERVAL: Duration = Duration::from_secs(5 * 60);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum WalletWorkKind {
    Load,
    Sync,
    Maintain,
    ForceRefresh,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct WalletWorkRequest {
    pub(super) kind: WalletWorkKind,
    pub(super) report_errors: bool,
    /// The request was caused by state that changed after the in-flight work
    /// started, so even equivalent work must run again afterward.
    pub(super) ensure_after_current: bool,
}

impl WalletWorkRequest {
    pub(super) const fn lifecycle(kind: WalletWorkKind) -> Self {
        Self {
            kind,
            report_errors: false,
            ensure_after_current: false,
        }
    }

    pub(super) const fn data_changed(kind: WalletWorkKind) -> Self {
        Self {
            kind,
            report_errors: false,
            ensure_after_current: true,
        }
    }

    pub(super) const fn user_sync() -> Self {
        Self {
            kind: WalletWorkKind::Sync,
            report_errors: true,
            ensure_after_current: false,
        }
    }

    fn merge(self, other: Self) -> Self {
        Self {
            kind: self.kind.max(other.kind),
            report_errors: self.report_errors || other.report_errors,
            ensure_after_current: self.ensure_after_current || other.ensure_after_current,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct WalletWorkToken {
    pub(super) id: u64,
    pub(super) generation: u64,
    pub(super) kind: WalletWorkKind,
    pub(super) report_errors: bool,
}

#[derive(Default)]
pub(super) struct WalletWorkCoordinator {
    next_id: u64,
    in_flight: Option<WalletWorkToken>,
    queued: Option<WalletWorkRequest>,
}

impl WalletWorkCoordinator {
    pub(super) fn request(
        &mut self,
        generation: u64,
        request: WalletWorkRequest,
    ) -> Option<WalletWorkToken> {
        if let Some(in_flight) = self.in_flight.as_mut() {
            if !request.ensure_after_current
                && in_flight.kind >= request.kind
                && (in_flight.kind != WalletWorkKind::ForceRefresh
                    || request.kind == WalletWorkKind::ForceRefresh)
            {
                in_flight.report_errors |= request.report_errors;
            } else {
                self.defer(request);
            }
            return None;
        }

        let request = match self.queued.take() {
            Some(queued) => queued.merge(request),
            None => request,
        };
        Some(self.start(generation, request))
    }

    pub(super) fn defer(&mut self, request: WalletWorkRequest) {
        self.queued = Some(match self.queued.take() {
            Some(queued) => queued.merge(request),
            None => request,
        });
    }

    pub(super) fn start_queued(&mut self, generation: u64) -> Option<WalletWorkToken> {
        if self.in_flight.is_some() {
            return None;
        }
        let request = self.queued.take()?;
        Some(self.start(generation, request))
    }

    pub(super) fn finish(&mut self, generation: u64, operation_id: u64) -> Option<WalletWorkToken> {
        let token = self.in_flight?;
        if token.generation != generation || token.id != operation_id {
            return None;
        }
        self.in_flight = None;
        Some(token)
    }

    pub(super) fn reset(&mut self) {
        self.in_flight = None;
        self.queued = None;
    }

    pub(super) fn in_flight(&self) -> Option<WalletWorkToken> {
        self.in_flight
    }

    pub(super) fn queued(&self) -> Option<WalletWorkRequest> {
        self.queued
    }

    pub(super) fn has_work(&self) -> bool {
        self.in_flight.is_some() || self.queued.is_some()
    }

    fn start(&mut self, generation: u64, request: WalletWorkRequest) -> WalletWorkToken {
        self.next_id = self.next_id.wrapping_add(1).max(1);
        let token = WalletWorkToken {
            id: self.next_id,
            generation,
            kind: request.kind,
            report_errors: request.report_errors,
        };
        self.in_flight = Some(token);
        token
    }
}

pub(super) fn refresh_poll_delay(attempt: u8) -> Duration {
    const DELAYS: [u64; 6] = [2, 5, 10, 20, 30, 60];
    let index = usize::from(attempt).min(DELAYS.len() - 1);
    Duration::from_secs(DELAYS[index])
}

#[cfg(test)]
mod tests {
    use super::{refresh_poll_delay, WalletWorkCoordinator, WalletWorkKind, WalletWorkRequest};

    #[test]
    fn explicit_refresh_does_not_swallow_maintenance_or_duplicate_itself() {
        let mut coordinator = WalletWorkCoordinator::default();
        let request = WalletWorkRequest {
            kind: WalletWorkKind::ForceRefresh,
            report_errors: true,
            ensure_after_current: false,
        };
        let refresh = coordinator.request(1, request).unwrap();
        assert!(coordinator.request(1, request).is_none());
        assert!(coordinator.queued().is_none());
        coordinator.request(1, WalletWorkRequest::lifecycle(WalletWorkKind::Maintain));
        assert_eq!(coordinator.in_flight(), Some(refresh));
        coordinator.finish(1, refresh.id);
        assert_eq!(
            coordinator.start_queued(1).unwrap().kind,
            WalletWorkKind::Maintain
        );
    }

    #[test]
    fn maintenance_supersedes_queued_sync() {
        let mut coordinator = WalletWorkCoordinator::default();
        let sync = coordinator
            .request(7, WalletWorkRequest::lifecycle(WalletWorkKind::Sync))
            .expect("sync starts");

        assert!(coordinator
            .request(7, WalletWorkRequest::lifecycle(WalletWorkKind::Maintain))
            .is_none());
        assert_eq!(coordinator.finish(7, sync.id), Some(sync));

        let maintenance = coordinator.start_queued(7).expect("maintenance queued");
        assert_eq!(maintenance.kind, WalletWorkKind::Maintain);
    }

    #[test]
    fn startup_load_finishes_before_maintenance() {
        let mut coordinator = WalletWorkCoordinator::default();
        let load = coordinator
            .request(1, WalletWorkRequest::lifecycle(WalletWorkKind::Load))
            .expect("load starts");

        coordinator.request(1, WalletWorkRequest::lifecycle(WalletWorkKind::Maintain));
        assert_eq!(coordinator.in_flight(), Some(load));
        coordinator.finish(1, load.id);

        assert_eq!(
            coordinator
                .start_queued(1)
                .expect("maintenance starts")
                .kind,
            WalletWorkKind::Maintain
        );
    }

    #[test]
    fn lifecycle_request_is_absorbed_by_equivalent_in_flight_work() {
        let mut coordinator = WalletWorkCoordinator::default();
        let maintenance = coordinator
            .request(1, WalletWorkRequest::lifecycle(WalletWorkKind::Maintain))
            .expect("maintenance starts");

        assert!(coordinator
            .request(1, WalletWorkRequest::lifecycle(WalletWorkKind::Maintain))
            .is_none());
        assert_eq!(coordinator.finish(1, maintenance.id), Some(maintenance));
        assert!(coordinator.start_queued(1).is_none());
    }

    #[test]
    fn data_change_queues_equivalent_work_after_current() {
        let mut coordinator = WalletWorkCoordinator::default();
        let maintenance = coordinator
            .request(1, WalletWorkRequest::lifecycle(WalletWorkKind::Maintain))
            .expect("maintenance starts");

        coordinator.request(1, WalletWorkRequest::data_changed(WalletWorkKind::Maintain));
        coordinator.finish(1, maintenance.id);

        assert_eq!(
            coordinator.start_queued(1).expect("follow-up starts").kind,
            WalletWorkKind::Maintain
        );
    }

    #[test]
    fn stale_completion_does_not_finish_current_work() {
        let mut coordinator = WalletWorkCoordinator::default();
        let token = coordinator
            .request(4, WalletWorkRequest::user_sync())
            .expect("sync starts");

        assert!(coordinator.finish(3, token.id).is_none());
        assert!(coordinator.finish(4, token.id + 1).is_none());
        assert_eq!(coordinator.in_flight(), Some(token));
        assert_eq!(coordinator.finish(4, token.id), Some(token));
    }

    #[test]
    fn reset_discards_in_flight_and_queued_work() {
        let mut coordinator = WalletWorkCoordinator::default();
        coordinator.request(1, WalletWorkRequest::user_sync());
        coordinator.request(1, WalletWorkRequest::data_changed(WalletWorkKind::Maintain));

        coordinator.reset();

        assert!(!coordinator.has_work());
    }

    #[test]
    fn refresh_poll_backoff_caps_at_sixty_seconds() {
        assert_eq!(refresh_poll_delay(0).as_secs(), 2);
        assert_eq!(refresh_poll_delay(3).as_secs(), 20);
        assert_eq!(refresh_poll_delay(20).as_secs(), 60);
    }
}
