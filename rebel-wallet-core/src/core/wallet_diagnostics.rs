use std::fmt::Write;
use std::time::Duration;

use bark::movement::{Movement, MovementStatus};
use bark::subsystem::{RoundMovement, Subsystem};
use bark::Wallet;

use super::AppCore;
use crate::updates::{AsyncMsg, CoreMsg};

impl AppCore {
    pub(super) fn force_refresh_wallet_vtxos(&mut self) {
        if self.state.wallet_refresh_running {
            return;
        }
        if self.wallet.is_none() {
            self.state.wallet_refresh_status = "Wallet is not open.".to_string();
            return;
        }
        if self.wallet_work.has_work()
            || self.state.busy.sending_payment
            || self.send_screen_blocks_maintenance()
        {
            self.state.wallet_refresh_status =
                "Wallet work is in progress. Wait for it to finish and try again.".to_string();
            return;
        }
        self.state.wallet_refresh_running = true;
        self.state.wallet_refresh_status =
            "Reconciling pending rounds and requesting a VTXO refresh…".to_string();
        self.request_wallet_work(super::WalletWorkRequest {
            kind: super::WalletWorkKind::ForceRefresh,
            report_errors: true,
            ensure_after_current: false,
        });
    }

    pub(super) fn load_wallet_diagnostics(&mut self) {
        if self.state.wallet_diagnostics_loading {
            return;
        }
        let Some(wallet) = self.wallet.clone() else {
            self.state.wallet_diagnostics = "Wallet is not open.".to_string();
            return;
        };
        self.state.wallet_diagnostics_loading = true;
        let generation = self.wallet_generation;
        let tx = self.tx.clone();
        self.rt.spawn(async move {
            let report = match tokio::time::timeout(Duration::from_secs(20), report(&wallet)).await
            {
                Ok(Ok(report)) => report,
                Ok(Err(error)) => format!("Could not read wallet diagnostics: {error:#}"),
                Err(_) => "Reading wallet diagnostics timed out. Try Reload.".to_string(),
            };
            let _ = tx.send(CoreMsg::Async(AsyncMsg::WalletDiagnosticsLoaded {
                generation,
                report,
            }));
        });
    }
}

fn is_refresh(movement: &Movement) -> bool {
    movement.subsystem.name == Subsystem::ROUND.as_name()
        && movement.subsystem.kind == RoundMovement::Refresh.to_string()
}

fn completed_refresh_time(movement: &Movement) -> Option<String> {
    if !is_refresh(movement) || movement.status != MovementStatus::Successful {
        return None;
    }
    movement.time.completed_at.map(|time| time.to_rfc3339())
}

async fn report(wallet: &Wallet) -> anyhow::Result<String> {
    let mut vtxos = wallet.all_vtxos().await?;
    let mut history = wallet.history().await?;
    history.sort_by_key(|movement| std::cmp::Reverse(movement.time.created_at));
    vtxos.sort_by_key(|item| (item.state.kind().to_string(), item.vtxo.id()));
    let mut text = format!(
        "Read at: {}\n\nLocal database snapshot; server spendability is not verified. Reload only rereads local records and does not refresh funds. Records may change while this snapshot is being read.\n\nVTXOs: {} (showing up to 500, including spent/locked)\n",
        chrono::Local::now().to_rfc3339(), vtxos.len(),
    );
    for item in vtxos.iter().take(500) {
        let id = item.vtxo.id();
        let created_by = history
            .iter()
            .find(|movement| movement.output_vtxos.contains(&id));
        let refreshed = created_by
            .and_then(completed_refresh_time)
            .unwrap_or_else(|| "No completed refresh recorded for this output".to_string());
        writeln!(text, "\n{id}\nAmount: {} sats\nLocal state: {}\nExpiry: block {}\nRecovery registration recorded: {}\nCreated by completed refresh: {refreshed}",
            item.vtxo.amount().to_sat(), item.state.kind(), item.vtxo.expiry_height(), item.registered)?;
        if let Some(movement) = created_by {
            writeln!(
                text,
                "Origin movement: {} / {} ({})",
                movement.id, movement.subsystem.kind, movement.status
            )?;
        }
        for movement in history
            .iter()
            .filter(|movement| movement.input_vtxos.contains(&id))
            .take(5)
        {
            writeln!(
                text,
                "Used by: {} / {} ({}) at {}",
                movement.id,
                movement.subsystem.kind,
                movement.status,
                movement.time.updated_at.to_rfc3339()
            )?;
        }
    }
    text.push_str("\nREFRESH HISTORY (up to 50 newest attempts)\nA refresh consumes old VTXOs and creates new IDs. Only successful movements with a completion time count as completed refreshes. Missing history after restore is not proof that a VTXO was never refreshed.\n");
    let refreshes: Vec<_> = history
        .iter()
        .filter(|movement| is_refresh(movement))
        .take(50)
        .collect();
    if refreshes.is_empty() {
        text.push_str("\nNo refresh movements recorded.\n");
    }
    for movement in refreshes {
        writeln!(text, "\nMovement {} — {}\nStarted: {}\nLast updated: {}\nCompleted refresh: {}\nInputs: {}\nOutputs: {}", movement.id, movement.status,
            movement.time.created_at.to_rfc3339(), movement.time.updated_at.to_rfc3339(),
            completed_refresh_time(movement).unwrap_or_else(|| "Not recorded".to_string()),
            movement.input_vtxos.iter().map(ToString::to_string).collect::<Vec<_>>().join(", "),
            movement.output_vtxos.iter().map(ToString::to_string).collect::<Vec<_>>().join(", "))?;
    }
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bark::movement::{MovementId, MovementSubsystem};

    #[test]
    fn refresh_failure_preserves_error_without_automatic_retry() {
        let (_data, _cache, mut core) = super::super::tests::test_core();
        let token = core
            .wallet_work
            .request(
                core.wallet_generation,
                super::super::WalletWorkRequest {
                    kind: super::super::WalletWorkKind::ForceRefresh,
                    report_errors: true,
                    ensure_after_current: false,
                },
            )
            .unwrap();
        core.state.wallet_refresh_running = true;
        core.finish_wallet_work(
            token.generation,
            token.id,
            Err("server says VTXO is spent".to_string()),
        );
        assert!(!core.state.wallet_refresh_running);
        assert!(core
            .state
            .wallet_refresh_status
            .contains("server says VTXO is spent"));
        assert!(core.wallet_retry_kind.is_none());
        assert!(!core.wallet_work.has_work());
    }

    #[test]
    fn only_completed_successful_refreshes_have_a_refresh_time() {
        let now = chrono::Local::now();
        let mut movement = Movement::new(
            MovementId(1),
            MovementStatus::Pending,
            &MovementSubsystem {
                name: Subsystem::ROUND.as_name().to_string(),
                kind: RoundMovement::Refresh.to_string(),
            },
            now,
        );
        movement.time.completed_at = Some(now);
        assert_eq!(completed_refresh_time(&movement), None);
        movement.status = MovementStatus::Successful;
        assert_eq!(completed_refresh_time(&movement), Some(now.to_rfc3339()));
        movement.time.completed_at = None;
        assert_eq!(completed_refresh_time(&movement), None);
        movement.time.completed_at = Some(now);
        movement.subsystem.kind = "send".to_string();
        assert_eq!(completed_refresh_time(&movement), None);
    }
}
