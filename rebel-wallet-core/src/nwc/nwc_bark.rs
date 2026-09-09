//! Rebel Wallet's Bark implementation of the `nwc-mobile` Lightning node contract.

#![forbid(unsafe_code)]

use std::collections::BTreeSet;
use std::future::Future;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use bark::actions::lightning::pay::{LightningSend, LightningSendState};
use bark::actions::lightning::receive::{LightningReceive, LightningReceiveState};
use bark::ark::lightning::PaymentHash as BarkPaymentHash;
use bark::movement::{Movement, MovementStatus};
use bark::persist::models::SettledLightningReceive;
use bark::{FeeEstimate, Wallet};
use bip39::Mnemonic;
use bitcoin::Amount;
use nostr_sdk::prelude::ToBech32;
use nwc_mobile::{
    prepare_transaction_page, standard_wallet_info, AmountMsat, CreatedInvoice, HostError,
    HostErrorKind, HostFuture, InvoiceLookup, ListTransactionsRequest, MakeInvoiceRequest,
    NwcLightningNode, NwcNotificationType, PayInvoiceRequest, PaymentFailure, PaymentHash,
    PaymentPreimage, PaymentQuote, PaymentStatus, PublicKey, TransactionDirection, UnixTimestamp,
    WakeDiagnosticCode, WakeDiagnosticSink, WalletInfo, WalletTransaction,
};
use nwc_mobile_bolt11::{
    created_invoice, exact_sats, parse_invoice, payment_amount_sats, quote_invoice_sats,
    sats_to_msats,
};
use nwc_mobile_tokio::{
    poll_until_terminal, sleep, spawn_abort_on_drop, LightningNodeProvider,
    LightningNodeProviderFn, LightningNodeRequest, OpenedLightningNode, ReadyLightningNodeProvider,
};
use serde_json::Value;
use zeroize::Zeroizing;

use crate::core::{derive_nostr_keys_from_mnemonic, NOSTR_SECRET_KEY, WALLET_SEED_KEY};
use crate::payments::lightning_receive_progress_is_paid;
use crate::persistence::{PersistedAppData, ServerConfig};
use crate::wallet::{open_bark_wallet, WalletOpenMode};
use crate::{SecretStore, WalletNetwork};

const PAYMENT_SETTLE_TIMEOUT: Duration = Duration::from_secs(22);
const PAYMENT_POLL_INTERVAL: Duration = Duration::from_millis(750);
const INVOICE_SETTLEMENT_MAX_WAIT: Duration = Duration::from_secs(25);
const INVOICE_SETTLEMENT_POLL_INTERVAL: Duration = Duration::from_millis(750);
const APP_DATA_FILE: &str = "rebel-app-data.json";

/// Opens Rebel's existing Bark wallet when a native worker starts cold.
pub(super) fn cold_bark_provider(
    data_dir: PathBuf,
    secrets: Arc<dyn SecretStore>,
) -> impl LightningNodeProvider {
    LightningNodeProviderFn::new(move |request, context| {
        let data_dir = data_dir.clone();
        let secrets = secrets.clone();
        Box::pin(async move {
            if context.cancellation().is_cancelled() {
                return Err(HostError::new(HostErrorKind::Cancelled));
            }
            let server_config = extension_server_config(&data_dir)
                .ok_or_else(|| HostError::new(HostErrorKind::Unavailable))?;
            let mnemonic = secrets
                .get_secret(WALLET_SEED_KEY.to_string())
                .map(Zeroizing::new)
                .and_then(|seed| Mnemonic::from_str(seed.as_str()).ok())
                .ok_or_else(unavailable)?;
            ensure_nostr_secret(secrets.as_ref(), &mnemonic)?;
            if context.cancellation().is_cancelled() {
                return Err(HostError::new(HostErrorKind::Cancelled));
            }
            let wallet = open_bark_wallet(
                data_dir,
                &mnemonic,
                WalletOpenMode::OpenExisting,
                server_config,
            )
            .await
            .map_err(|_| unavailable())?
            .wallet;
            if context.cancellation().is_cancelled() {
                return Err(HostError::new(HostErrorKind::Cancelled));
            }
            Ok(opened_bark_node(wallet, request))
        })
    })
}

pub(super) fn opened_bark_provider(wallet: Wallet) -> impl LightningNodeProvider {
    ReadyLightningNodeProvider::new(move |request| Ok(opened_bark_node(wallet.clone(), request)))
}

fn opened_bark_node(wallet: Wallet, request: LightningNodeRequest) -> OpenedLightningNode {
    let wallet_info = bark_wallet_info(request.wallet_service_pubkey().clone());
    let node = match request.diagnostics() {
        Some(diagnostics) => NwcBarkLightning::with_diagnostics(wallet, diagnostics),
        None => NwcBarkLightning::new(wallet),
    };
    OpenedLightningNode::new(node, wallet_info)
}

fn extension_server_config(data_dir: &std::path::Path) -> Option<ServerConfig> {
    let raw = std::fs::read_to_string(data_dir.join(APP_DATA_FILE)).ok()?;
    let data: PersistedAppData = serde_json::from_str(&raw).ok()?;
    Some(
        if data.network == WalletNetwork::Regtest && data.servers.network == WalletNetwork::Regtest
        {
            data.servers
        } else {
            ServerConfig::for_network(data.network)
        },
    )
}

fn ensure_nostr_secret(secrets: &dyn SecretStore, mnemonic: &Mnemonic) -> Result<(), HostError> {
    if secrets.get_secret(NOSTR_SECRET_KEY.to_string()).is_some() {
        return Ok(());
    }
    let mnemonic = Zeroizing::new(mnemonic.to_string());
    let keys = derive_nostr_keys_from_mnemonic(mnemonic.as_str())
        .map_err(|_| HostError::new(HostErrorKind::Internal))?;
    let encoded = Zeroizing::new(
        keys.secret_key()
            .to_bech32()
            .expect("secret-key bech32 encoding is infallible"),
    );
    secrets
        .set_secret(NOSTR_SECRET_KEY.to_string(), encoded.to_string())
        .then_some(())
        .ok_or_else(unavailable)
}

const fn unavailable() -> HostError {
    HostError::new(HostErrorKind::Unavailable)
}

/// An already-open Bark wallet exposed as a compact Lightning node.
#[derive(Clone)]
pub(crate) struct NwcBarkLightning {
    wallet: Wallet,
    diagnostics: Option<Arc<dyn WakeDiagnosticSink>>,
}

impl NwcBarkLightning {
    /// Creates a node over an already-open Bark wallet.
    #[must_use]
    pub(crate) fn new(wallet: Wallet) -> Self {
        Self {
            wallet,
            diagnostics: None,
        }
    }

    /// Creates a node that emits bounded, non-secret execution codes.
    #[must_use]
    pub(crate) fn with_diagnostics(
        wallet: Wallet,
        diagnostics: Arc<dyn WakeDiagnosticSink>,
    ) -> Self {
        Self {
            wallet,
            diagnostics: Some(diagnostics),
        }
    }

    fn record_diagnostic(&self, code: WakeDiagnosticCode) {
        if let Some(diagnostics) = self.diagnostics.as_deref() {
            diagnostics.record(code);
        }
    }
}

impl NwcLightningNode for NwcBarkLightning {
    fn get_balance(&self) -> HostFuture<'_, Result<AmountMsat, HostError>> {
        Box::pin(async move {
            let balance = self
                .wallet
                .balance()
                .await
                .map_err(|_| host_error(HostErrorKind::Internal))?;
            Ok(sats_to_msats(balance.spendable.to_sat()))
        })
    }

    fn create_invoice(
        &self,
        request: MakeInvoiceRequest,
    ) -> HostFuture<'_, Result<CreatedInvoice, HostError>> {
        Box::pin(async move {
            let amount_sat = exact_sats(request.amount())?;
            if amount_sat == 0 {
                return Err(host_error(HostErrorKind::Rejected));
            }
            let invoice = self
                .wallet
                .bolt11_invoice(
                    Amount::from_sat(amount_sat),
                    request.description().map(ToString::to_string),
                    None,
                )
                .await
                .map_err(|_| host_error(HostErrorKind::Internal))?;
            created_invoice(invoice)
        })
    }

    fn quote_invoice<'a>(
        &'a self,
        invoice: &'a str,
        amount: Option<AmountMsat>,
    ) -> HostFuture<'a, Result<PaymentQuote, HostError>> {
        Box::pin(async move { quote_invoice_sats(invoice, amount) })
    }

    fn pay_invoice(
        &self,
        request: PayInvoiceRequest,
    ) -> HostFuture<'_, Result<PaymentStatus, HostError>> {
        Box::pin(async move {
            let invoice = parse_invoice(request.invoice())?;
            let amount_sat = payment_amount_sats(&invoice, request.amount())?;
            let payment_hash: BarkPaymentHash = (*invoice.payment_hash()).into();
            let payment = async {
                let existing = payment_status_without_mailbox(&self.wallet, payment_hash).await?;
                if !matches!(existing, PaymentStatus::Unknown) {
                    return Ok(existing);
                }

                let fee = self
                    .wallet
                    .estimate_lightning_send_fee(Amount::from_sat(amount_sat))
                    .await
                    .map_err(|_| {
                        self.record_diagnostic(WakeDiagnosticCode::PaymentBackendFailed);
                        host_error(HostErrorKind::Internal)
                    })?;
                let spendable = if fee.vtxos_spent.is_empty() {
                    Some(
                        self.wallet
                            .balance()
                            .await
                            .map_err(|_| {
                                self.record_diagnostic(WakeDiagnosticCode::PaymentBackendFailed);
                                host_error(HostErrorKind::Internal)
                            })?
                            .spendable,
                    )
                } else {
                    None
                };
                if let Some((reason, diagnostic)) = payment_preflight_failure(
                    &fee,
                    spendable,
                    Amount::from_sat(request.maximum_fee().as_sat()),
                ) {
                    self.record_diagnostic(diagnostic);
                    return Ok(PaymentStatus::Failed { reason });
                }

                // Bark persists sends by payment hash before network execution, so
                // the engine's event idempotency key composes with Bark's stronger
                // global invoice idempotency without a second mutable mapping.
                let user_amount = request
                    .amount()
                    .map(exact_sats)
                    .transpose()?
                    .map(Amount::from_sat);
                self.wallet
                    .pay_lightning_invoice(invoice, user_amount, false)
                    .await
                    .map_err(|_| {
                        self.record_diagnostic(WakeDiagnosticCode::PaymentBackendFailed);
                        host_error(HostErrorKind::Internal)
                    })?;
                wait_for_payment_terminal(&self.wallet, payment_hash).await
            };
            run_with_bark_mailbox(self.wallet.clone(), payment).await
        })
    }

    fn lookup_invoice(
        &self,
        request: InvoiceLookup,
        settlement_timeout: Option<Duration>,
    ) -> HostFuture<'_, Result<Option<WalletTransaction>, HostError>> {
        Box::pin(async move {
            let payment_hash = lookup_payment_hash(&request)?;
            let settlement_wait = settlement_timeout
                .filter(|timeout| !timeout.is_zero())
                .map(|timeout| timeout.min(INVOICE_SETTLEMENT_MAX_WAIT));
            let result =
                reconcile_then_lookup_transaction(&self.wallet, payment_hash, settlement_wait)
                    .await;
            match &result {
                Ok(Some(transaction))
                    if matches!(transaction.status, PaymentStatus::Succeeded { .. }) =>
                {
                    self.record_diagnostic(WakeDiagnosticCode::InvoiceLookupSettled);
                }
                Ok(Some(_)) => {
                    self.record_diagnostic(WakeDiagnosticCode::InvoiceLookupPending);
                }
                Ok(None) => {
                    self.record_diagnostic(WakeDiagnosticCode::InvoiceLookupNotFound);
                }
                Err(_) => {
                    self.record_diagnostic(WakeDiagnosticCode::InvoiceLookupFailed);
                }
            }
            result
        })
    }

    fn list_transactions(
        &self,
        request: ListTransactionsRequest,
    ) -> HostFuture<'_, Result<Vec<WalletTransaction>, HostError>> {
        Box::pin(async move {
            // Bark's history and pending receive tables are local views. Refresh
            // the durable mailbox first so an NWC client never receives a stale
            // transaction list after the application has been suspended.
            self.wallet.sync().await;
            let mut transactions = self
                .wallet
                .history()
                .await
                .map_err(|_| host_error(HostErrorKind::Internal))?
                .iter()
                .filter_map(transaction_from_movement)
                .collect::<Vec<_>>();
            let mut known_hashes = transactions
                .iter()
                .filter_map(|transaction| transaction.payment_hash.as_ref())
                .map(PaymentHash::to_hex)
                .collect::<BTreeSet<_>>();

            let receives = self
                .wallet
                .pending_lightning_receives()
                .await
                .map_err(|_| host_error(HostErrorKind::Internal))?;
            transactions.extend(receives.iter().filter_map(|receive| {
                let hash = receive.payment_hash.to_string();
                known_hashes
                    .insert(hash)
                    .then(|| transaction_from_in_progress_receive(receive))
                    .flatten()
            }));

            let sends = self
                .wallet
                .pending_lightning_sends()
                .await
                .map_err(|_| host_error(HostErrorKind::Internal))?;
            transactions.extend(sends.iter().filter_map(|send| {
                let hash = send.invoice.payment_hash().to_string();
                known_hashes
                    .insert(hash)
                    .then(|| transaction_from_pending_send(send))
                    .flatten()
            }));

            Ok(prepare_transaction_page(transactions, request))
        })
    }
}

async fn payment_status_without_mailbox(
    wallet: &Wallet,
    payment_hash: BarkPaymentHash,
) -> Result<PaymentStatus, HostError> {
    // Drive one nonblocking step and release Bark's action lock. The mailbox
    // processor may need that same lock to apply the server-delivered preimage.
    let state = wallet
        .check_lightning_payment(payment_hash, false)
        .await
        .map_err(|_| host_error(HostErrorKind::Internal))?;
    match state {
        LightningSendState::InProgress(_) => Ok(PaymentStatus::Pending),
        LightningSendState::Paid(paid) => {
            let history = wallet
                .history()
                .await
                .map_err(|_| host_error(HostErrorKind::Internal))?;
            let movement = history
                .iter()
                .find(|movement| movement.lightning_payment_hash() == Some(payment_hash))
                .ok_or_else(|| host_error(HostErrorKind::Internal))?;
            let direction = movement_direction(movement);
            let amount = movement_amount_sat(movement, direction);
            let preimage = PaymentPreimage::from_hex(&paid.preimage.to_string())
                .map_err(|_| host_error(HostErrorKind::Internal))?;
            Ok(PaymentStatus::Succeeded {
                preimage,
                amount: sats_to_msats(amount),
                fee: sats_to_msats(movement.offchain_fee.to_sat()),
            })
        }
        LightningSendState::Unknown => {
            let history = wallet
                .history()
                .await
                .map_err(|_| host_error(HostErrorKind::Internal))?;
            Ok(history
                .iter()
                .find(|movement| movement.lightning_payment_hash() == Some(payment_hash))
                .map(payment_status_from_movement)
                .transpose()?
                .unwrap_or(PaymentStatus::Unknown))
        }
    }
}

async fn wait_for_payment_terminal(
    wallet: &Wallet,
    payment_hash: BarkPaymentHash,
) -> Result<PaymentStatus, HostError> {
    poll_until_terminal(
        PAYMENT_SETTLE_TIMEOUT,
        PAYMENT_POLL_INTERVAL,
        || payment_status_without_mailbox(wallet, payment_hash),
        |status| !matches!(status, PaymentStatus::Pending),
    )
    .await?
    .ok_or_else(|| host_error(HostErrorKind::TimedOut))
}

/// Keeps Bark's durable mailbox stream alive while a bounded payment operation runs.
///
/// The full wallet app normally owns this stream. Short-lived native workers such as
/// an iOS notification service extension must run it explicitly so the server's
/// payment-finished message can settle the checkpoint and persist the preimage.
async fn run_with_bark_mailbox<T>(
    wallet: Wallet,
    operation: impl Future<Output = Result<T, HostError>>,
) -> Result<T, HostError> {
    let _mailbox = spawn_abort_on_drop(async move {
        let _ = wallet
            .subscribe_process_mailbox_messages(None, Default::default())
            .await;
    })
    .map_err(|_| host_error(HostErrorKind::Internal))?;
    operation.await
}

fn payment_status_from_movement(movement: &Movement) -> Result<PaymentStatus, HostError> {
    match movement.status {
        MovementStatus::Pending => Ok(PaymentStatus::Pending),
        MovementStatus::Failed | MovementStatus::Canceled => Ok(PaymentStatus::Failed {
            reason: PaymentFailure::Other,
        }),
        MovementStatus::Successful => {
            let preimage =
                movement_preimage(movement).ok_or_else(|| host_error(HostErrorKind::Internal))?;
            let direction = movement_direction(movement);
            Ok(PaymentStatus::Succeeded {
                preimage,
                amount: sats_to_msats(movement_amount_sat(movement, direction)),
                fee: sats_to_msats(movement.offchain_fee.to_sat()),
            })
        }
    }
}

async fn lookup_transaction(
    wallet: &Wallet,
    payment_hash: BarkPaymentHash,
) -> Result<Option<WalletTransaction>, HostError> {
    if let Ok(receive) = wallet.lightning_receive_state(payment_hash).await {
        return Ok(transaction_from_receive_state(&receive));
    }
    let history = wallet
        .history()
        .await
        .map_err(|_| host_error(HostErrorKind::Internal))?;
    if let Some(transaction) = history
        .iter()
        .find(|movement| movement.lightning_payment_hash() == Some(payment_hash))
        .and_then(transaction_from_movement)
    {
        return Ok(Some(transaction));
    }
    match wallet
        .lightning_send_state(payment_hash)
        .await
        .map_err(|_| host_error(HostErrorKind::Internal))?
    {
        LightningSendState::InProgress(send) => Ok(transaction_from_pending_send(&send)),
        LightningSendState::Paid(_) | LightningSendState::Unknown => Ok(None),
    }
}

async fn reconcile_then_lookup_transaction(
    wallet: &Wallet,
    payment_hash: BarkPaymentHash,
    settlement_wait: Option<Duration>,
) -> Result<Option<WalletTransaction>, HostError> {
    // `nwc-mobile` derives payment reconciliation from the same node lookup
    // used by NIP-47. Drive Bark's mailbox-backed outgoing state first so an
    // ambiguous or late payment cannot remain pending merely because the
    // containing process was started by an NSE wake.
    let payment_status = run_with_bark_mailbox(
        wallet.clone(),
        wait_for_payment_terminal(wallet, payment_hash),
    )
    .await?;
    if !matches!(payment_status, PaymentStatus::Unknown) {
        let mut transaction = lookup_transaction(wallet, payment_hash)
            .await?
            .ok_or_else(|| host_error(HostErrorKind::Internal))?;
        transaction.status = payment_status;
        return Ok(Some(transaction));
    }

    // A settled receive is already durable and needs no network refresh. This
    // fast path also lets the notification worker publish payment_received in
    // the same NSE wake without repeating the mailbox round trip.
    if let Ok(receive @ LightningReceiveState::Settled(_)) =
        wallet.lightning_receive_state(payment_hash).await
    {
        return Ok(transaction_from_receive_state(&receive));
    }

    let settlement_deadline = settlement_wait.map(|wait| Instant::now() + wait);

    // Preserve the full wallet refresh used by Rebel's original NWC receive
    // flow. Mailbox-only sync is insufficient when Bark also needs refreshed
    // Ark state to drive the receive action. A server-scheduled settlement
    // wake keeps driving this exact invoice until it settles or the bounded
    // wait expires; ordinary lookup_invoice requests still perform one pass.
    wallet.sync().await;

    loop {
        let claim_result = wallet
            .try_claim_lightning_receive(payment_hash, false)
            .await
            .map_err(|_| host_error(HostErrorKind::Internal));

        match claim_result {
            Ok(receive @ LightningReceiveState::Settled(_)) => {
                return Ok(transaction_from_receive_state(&receive));
            }
            Ok(LightningReceiveState::InProgress(receive))
                if lightning_receive_progress_is_paid(&receive.progress) =>
            {
                return Ok(transaction_from_in_progress_receive(&receive));
            }
            Ok(LightningReceiveState::InProgress(_)) => {}
            Err(_) => {
                // Bark persists action progress before parking or returning a
                // transient claim error. Re-read the checkpoint so a receive
                // whose preimage was already revealed is still reported paid.
                if let Ok(LightningReceiveState::InProgress(receive)) =
                    wallet.lightning_receive_state(payment_hash).await
                {
                    if lightning_receive_progress_is_paid(&receive.progress) {
                        return Ok(transaction_from_in_progress_receive(&receive));
                    }
                }
            }
        }

        let Some(deadline) = settlement_deadline else {
            break;
        };
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break;
        }
        sleep(remaining.min(INVOICE_SETTLEMENT_POLL_INTERVAL)).await;
    }

    // Preserve outgoing and history-only lookup behavior, and return the last
    // durable incoming state when a transient claim attempt could not advance.
    lookup_transaction(wallet, payment_hash).await
}

fn transaction_from_movement(movement: &Movement) -> Option<WalletTransaction> {
    let payment_hash = movement
        .lightning_payment_hash()
        .and_then(|hash| PaymentHash::from_hex(&hash.to_string()).ok())?;
    let direction = movement_direction(movement);
    let status = payment_status_from_movement(movement).ok()?;
    Some(WalletTransaction {
        payment_hash: Some(payment_hash),
        direction,
        amount: sats_to_msats(movement_amount_sat(movement, direction)),
        fee: sats_to_msats(movement.offchain_fee.to_sat()),
        created_at: timestamp(movement.time.created_at.timestamp()),
        settled_at: movement
            .time
            .completed_at
            .map(|time| timestamp(time.timestamp())),
        status,
    })
}

fn transaction_from_in_progress_receive(receive: &LightningReceive) -> Option<WalletTransaction> {
    let payment_hash = PaymentHash::from_hex(&receive.payment_hash.to_string()).ok()?;
    let amount = AmountMsat::from_msat(receive.invoice.amount_milli_satoshis()?);
    let created_at = UnixTimestamp::from_secs(receive.invoice.duration_since_epoch().as_secs());

    if lightning_receive_progress_is_paid(&receive.progress) {
        let preimage = PaymentPreimage::from_hex(&receive.payment_preimage.to_string()).ok()?;
        let observed_settlement_at =
            UnixTimestamp::from_secs(crate::time::now_unix().max(created_at.as_secs()));
        Some(WalletTransaction::settled_incoming(
            payment_hash,
            preimage,
            amount,
            created_at,
            observed_settlement_at,
        ))
    } else {
        Some(WalletTransaction::pending_incoming(
            payment_hash,
            amount,
            created_at,
        ))
    }
}

fn transaction_from_settled_receive(
    receive: &SettledLightningReceive,
) -> Option<WalletTransaction> {
    Some(WalletTransaction::settled_incoming(
        PaymentHash::from_hex(&receive.payment_hash.to_string()).ok()?,
        PaymentPreimage::from_hex(&receive.preimage.to_string()).ok()?,
        sats_to_msats(receive.amount.to_sat()),
        UnixTimestamp::from_secs(receive.invoice.duration_since_epoch().as_secs()),
        timestamp(receive.settled_at.timestamp()),
    ))
}

fn transaction_from_receive_state(receive: &LightningReceiveState) -> Option<WalletTransaction> {
    match receive {
        LightningReceiveState::InProgress(receive) => transaction_from_in_progress_receive(receive),
        LightningReceiveState::Settled(receive) => transaction_from_settled_receive(receive),
    }
}

fn transaction_from_pending_send(send: &LightningSend) -> Option<WalletTransaction> {
    Some(WalletTransaction::pending_outgoing(
        PaymentHash::from_hex(&send.invoice.payment_hash().to_string()).ok()?,
        sats_to_msats(send.payment_amount.to_sat()),
        sats_to_msats(send.fee.to_sat()),
        UnixTimestamp::from_secs(0),
    ))
}

fn movement_direction(movement: &Movement) -> TransactionDirection {
    if (!movement.received_on.is_empty() && movement.sent_to.is_empty())
        || movement.effective_balance.to_sat() >= 0
    {
        TransactionDirection::Incoming
    } else {
        TransactionDirection::Outgoing
    }
}

fn movement_amount_sat(movement: &Movement, direction: TransactionDirection) -> u64 {
    let destinations = match direction {
        TransactionDirection::Incoming => &movement.received_on,
        TransactionDirection::Outgoing => &movement.sent_to,
        _ => return movement.effective_balance.to_sat().unsigned_abs(),
    };
    let total = destinations
        .iter()
        .map(|destination| destination.amount.to_sat())
        .sum();
    if total > 0 {
        total
    } else {
        movement.effective_balance.to_sat().unsigned_abs()
    }
}

fn movement_preimage(movement: &Movement) -> Option<PaymentPreimage> {
    movement
        .metadata
        .get("payment_preimage")
        .and_then(Value::as_str)
        .and_then(|preimage| PaymentPreimage::from_hex(preimage).ok())
}

fn lookup_payment_hash(request: &InvoiceLookup) -> Result<BarkPaymentHash, HostError> {
    match request {
        InvoiceLookup::PaymentHash(hash) => bark_payment_hash(hash),
        InvoiceLookup::Invoice(invoice) => {
            parse_invoice(invoice).map(|invoice| (*invoice.payment_hash()).into())
        }
        _ => Err(host_error(HostErrorKind::Rejected)),
    }
}

fn bark_payment_hash(payment_hash: &PaymentHash) -> Result<BarkPaymentHash, HostError> {
    BarkPaymentHash::from_str(&payment_hash.to_hex())
        .map_err(|_| host_error(HostErrorKind::Rejected))
}

fn timestamp(seconds: i64) -> UnixTimestamp {
    UnixTimestamp::from_secs(seconds.max(0) as u64)
}

pub(crate) fn bark_wallet_info(service_pubkey: PublicKey) -> WalletInfo {
    // Do not advertise payment_received until the mobile host has a reliable server-side wake
    // source. NWC clients such as Alby Go then poll lookup_invoice, which wakes the app and
    // refreshes Bark's mailbox.
    standard_wallet_info(service_pubkey, [NwcNotificationType::PaymentSent])
}

fn payment_preflight_failure(
    fee: &FeeEstimate,
    spendable: Option<Amount>,
    maximum_fee: Amount,
) -> Option<(PaymentFailure, WakeDiagnosticCode)> {
    if spendable.is_some_and(|balance| balance < fee.gross_amount) {
        return Some((
            PaymentFailure::InsufficientFunds,
            WakeDiagnosticCode::PaymentInsufficientFunds,
        ));
    }
    if fee.fee > maximum_fee {
        return Some((
            PaymentFailure::Other,
            WakeDiagnosticCode::PaymentFeeLimitExceeded,
        ));
    }
    None
}

const fn host_error(kind: HostErrorKind) -> HostError {
    HostError::new(kind)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wallet_info_requires_lookup_polling_for_receive_settlement() {
        let service_pubkey =
            PublicKey::from_hex("79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798")
                .expect("service public key");
        assert_eq!(
            bark_wallet_info(service_pubkey)
                .notifications()
                .collect::<Vec<_>>(),
            vec![NwcNotificationType::PaymentSent]
        );
    }

    #[test]
    fn payment_preflight_distinguishes_balance_and_fee_failures() {
        let fee = FeeEstimate::new(
            Amount::from_sat(610),
            Amount::from_sat(600),
            Amount::from_sat(10),
            Vec::new(),
        );

        assert_eq!(
            payment_preflight_failure(&fee, Some(Amount::from_sat(500)), Amount::from_sat(500),),
            Some((
                PaymentFailure::InsufficientFunds,
                WakeDiagnosticCode::PaymentInsufficientFunds,
            ))
        );
        assert_eq!(
            payment_preflight_failure(&fee, Some(Amount::from_sat(1_000)), Amount::from_sat(500),),
            Some((
                PaymentFailure::Other,
                WakeDiagnosticCode::PaymentFeeLimitExceeded,
            ))
        );
        assert_eq!(
            payment_preflight_failure(&fee, Some(Amount::from_sat(1_000)), Amount::from_sat(600),),
            None
        );
    }

    #[test]
    fn negative_wallet_timestamps_fail_closed_to_epoch() {
        assert_eq!(timestamp(-1), UnixTimestamp::from_secs(0));
        assert_eq!(timestamp(42), UnixTimestamp::from_secs(42));
    }
}
