//! Reconcile historical mailbox inputs without deleting their transaction history.
use std::{path::Path, time::Duration};

use anyhow::{ensure, Context};
use ark::{attestations::VtxoStatusAttestation, ProtocolEncoding};
use bark::{
    persist::{sqlite::SqliteClient, BarkPersister},
    vtxo::{VtxoState, VtxoStateKind},
    Wallet,
};
use server_rpc::{protos, ServerConnection};

fn confirmed_spent(state: i32) -> anyhow::Result<bool> {
    match protos::VtxoSpendState::try_from(state) {
        Ok(protos::VtxoSpendState::Spent) => Ok(true),
        Ok(
            protos::VtxoSpendState::Spendable
            | protos::VtxoSpendState::Unclaimed
            | protos::VtxoSpendState::Unregistered
            | protos::VtxoSpendState::HtlcRecvUnclaimed,
        ) => Ok(false),
        _ => anyhow::bail!("Server returned an unknown VTXO spend state; records unchanged"),
    }
}

pub(crate) async fn reconcile(wallet: &Wallet, db_path: &Path) -> anyhow::Result<usize> {
    tokio::time::timeout(Duration::from_secs(20), reconcile_inner(wallet, db_path))
        .await
        .context("VTXO reconciliation timed out")?
}

async fn reconcile_inner(wallet: &Wallet, db_path: &Path) -> anyhow::Result<usize> {
    let pending = wallet.pending_round_input_vtxos().await?;
    let candidates: Vec<_> = wallet
        .spendable_vtxos()
        .await?
        .into_iter()
        .filter(|v| !pending.iter().any(|p| p.vtxo.id() == v.vtxo.id()))
        .collect();
    if candidates.is_empty() {
        return Ok(0);
    }
    let properties = wallet.properties().await?;
    ensure!(db_path.is_file(), "Wallet database is missing");
    let db = SqliteClient::open(db_path)?;
    let stored = db
        .read_properties()
        .await?
        .context("Wallet database has no identity")?;
    ensure!(
        stored.fingerprint == properties.fingerprint && stored.network == properties.network,
        "Wallet database identity changed; reconciliation cancelled"
    );
    let builder = ServerConnection::builder()
        .address(wallet.config().server_address.clone())
        .network(properties.network)
        .user_agent(concat!("rebel-wallet/", env!("CARGO_PKG_VERSION")));
    let mut server = builder.connect().await?;
    let info = server.ark_info().await;
    ensure!(
        properties.server_pubkey == Some(info.server_pubkey)
            && properties.server_mailbox_pubkey == Some(info.mailbox_pubkey),
        "Server identity changed; reconciliation cancelled"
    );
    let mut spent = Vec::new();
    for candidate in candidates {
        let id = candidate.vtxo.id();
        let key = wallet.get_vtxo_key(id).await?;
        let response = server
            .client
            .get_vtxo_status(protos::GetVtxoStatusRequest {
                vtxo_id: id.to_bytes().to_vec(),
                attestation: VtxoStatusAttestation::new(id, &key).serialize(),
            })
            .await
            .with_context(|| format!("Could not verify VTXO {id}"))?
            .into_inner();
        if confirmed_spent(response.spend_state)? {
            spent.push(id);
        }
    }
    // Atomic checked transition: never overwrite a lock acquired during the RPCs,
    // and never modify records based on a payment error string or an RPC failure.
    if !spent.is_empty() {
        db.update_vtxo_states_checked(&spent, VtxoState::Spent, &[VtxoStateKind::Spendable])
            .await?;
        eprintln!("VTXO reconciliation: {} server-confirmed spent records removed from spendable balance; history retained", spent.len());
    }
    Ok(spent.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_explicit_server_spent_status_consumes_a_record() {
        use protos::VtxoSpendState::*;
        assert!(confirmed_spent(Spent as i32).unwrap());
        for state in [Spendable, Unclaimed, Unregistered, HtlcRecvUnclaimed] {
            assert!(!confirmed_spent(state as i32).unwrap());
        }
        assert!(confirmed_spent(Unspecified as i32).is_err());
        assert!(confirmed_spent(i32::MAX).is_err());
    }
}
