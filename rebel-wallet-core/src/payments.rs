use std::str::FromStr;
use std::time::Duration;

use anyhow::{anyhow, bail, Context};
use bark::ark::lightning::PaymentHash;
use bark::ark::Address as ArkAddress;
use bark::lightning_invoice::Bolt11Invoice;
use bark::movement::{Movement, MovementStatus, PaymentMethod as BarkPaymentMethod};
use bark::Wallet;
use flume::Sender;
use futures_util::StreamExt;
use serde::Deserialize;

use crate::{AsyncMsg, CoreMsg};

pub(crate) struct ParsedSendDestination {
    pub(crate) destination: String,
    pub(crate) amount_sat: Option<u64>,
    pub(crate) memo: Option<String>,
    pub(crate) toast: Option<String>,
}

pub(crate) async fn parse_send_destination(
    wallet: Wallet,
    raw: &str,
) -> Option<ParsedSendDestination> {
    let request = wallet.parse_payment_request(raw).await.ok()?;
    let amount_sat = request
        .amount
        .map(|amount| amount.to_sat())
        .or_else(|| embedded_send_amount_sat(raw));
    let memo = request.message.or(request.label);

    for option in request
        .options
        .iter()
        .filter(|option| option.errors.is_empty())
    {
        if let BarkPaymentMethod::Ark(address) = &option.method {
            return Some(ParsedSendDestination {
                destination: address.to_string(),
                amount_sat,
                memo,
                toast: None,
            });
        }
    }

    for option in request
        .options
        .iter()
        .filter(|option| option.errors.is_empty())
    {
        if let BarkPaymentMethod::Invoice(invoice) = &option.method {
            return Some(ParsedSendDestination {
                destination: invoice.to_string(),
                amount_sat,
                memo,
                toast: None,
            });
        }
    }

    for option in request
        .options
        .iter()
        .filter(|option| option.errors.is_empty())
    {
        if let BarkPaymentMethod::LightningAddress(address) = &option.method {
            return Some(ParsedSendDestination {
                destination: address.to_string(),
                amount_sat,
                memo,
                toast: None,
            });
        }
    }

    for option in request
        .options
        .iter()
        .filter(|option| option.errors.is_empty())
    {
        if let BarkPaymentMethod::Bitcoin(address) = &option.method {
            return Some(ParsedSendDestination {
                destination: address.assume_checked_ref().to_string(),
                amount_sat,
                memo,
                toast: None,
            });
        }
    }

    let toast = request
        .options
        .iter()
        .find_map(|option| match &option.method {
            BarkPaymentMethod::Ark(_) | BarkPaymentMethod::Invoice(_) => option
                .errors
                .first()
                .map(|e| format!("Invalid payment request: {e}")),
            BarkPaymentMethod::Bitcoin(_) => option
                .errors
                .first()
                .map(|e| format!("Invalid on-chain payment request: {e}")),
            BarkPaymentMethod::OutputScript(_) => {
                Some("Output script payment QR codes are not supported yet.".to_string())
            }
            BarkPaymentMethod::Offer(_) => {
                Some("BOLT12 payment QR codes are not supported yet.".to_string())
            }
            BarkPaymentMethod::LightningAddress(_) => {
                Some("Lightning address QR codes are not supported yet.".to_string())
            }
            BarkPaymentMethod::Custom(_) => {
                Some("This payment instruction type is not supported yet.".to_string())
            }
        });

    Some(ParsedSendDestination {
        destination: raw.to_string(),
        amount_sat,
        memo,
        toast,
    })
}

pub(crate) fn embedded_send_amount_sat(destination: &str) -> Option<u64> {
    bolt11_amount_sat(destination).or_else(|| bitcoin_uri_amount_sat(destination))
}

fn bolt11_amount_sat(destination: &str) -> Option<u64> {
    let invoice = strip_lightning_prefix(destination.trim());
    let invoice = Bolt11Invoice::from_str(invoice).ok()?;
    let msat = invoice.amount_milli_satoshis()?;
    let sat = msat.checked_add(999)? / 1_000;
    (sat > 0).then_some(sat)
}

fn bitcoin_uri_amount_sat(destination: &str) -> Option<u64> {
    let uri = destination.trim();
    if !uri.to_ascii_lowercase().starts_with("bitcoin:") {
        return None;
    }
    let url = reqwest::Url::parse(uri).ok()?;
    let amount = url
        .query_pairs()
        .find_map(|(key, value)| (key == "amount").then(|| value.into_owned()))?;
    decimal_btc_to_sat(&amount)
}

fn decimal_btc_to_sat(amount: &str) -> Option<u64> {
    let amount = amount.trim();
    if amount.is_empty() || amount.starts_with('-') || amount.starts_with('+') {
        return None;
    }
    let (whole, fractional) = amount.split_once('.').unwrap_or((amount, ""));
    if whole.is_empty() || !whole.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    if fractional.len() > 8 || !fractional.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let whole_sat = whole.parse::<u64>().ok()?.checked_mul(100_000_000)?;
    let mut fractional_text = fractional.to_string();
    while fractional_text.len() < 8 {
        fractional_text.push('0');
    }
    let fractional_sat = if fractional_text.is_empty() {
        0
    } else {
        fractional_text.parse::<u64>().ok()?
    };
    let sat = whole_sat.checked_add(fractional_sat)?;
    (sat > 0).then_some(sat)
}

#[derive(Debug, Deserialize)]
struct LnurlPayParams {
    tag: Option<String>,
    callback: String,
    #[serde(rename = "minSendable")]
    min_sendable: u64,
    #[serde(rename = "maxSendable")]
    max_sendable: u64,
}

#[derive(Debug, Deserialize)]
struct LnurlPayInvoice {
    pr: Option<String>,
    status: Option<String>,
    reason: Option<String>,
}

pub(crate) fn is_lnurl_pay_destination(destination: &str) -> bool {
    let destination = strip_lightning_prefix(destination.trim());
    let lower = destination.to_ascii_lowercase();
    lower.starts_with("lnurl") || is_valid_lightning_address(destination)
}

pub(crate) async fn resolve_lnurl_pay_invoice(
    destination: &str,
    amount_sat: u64,
) -> anyhow::Result<String> {
    if amount_sat == 0 {
        bail!("Enter an amount before sending to this Lightning address.");
    }

    let lnurl = lnurl_pay_url(destination)?;
    let amount_msat = amount_sat
        .checked_mul(1_000)
        .ok_or_else(|| anyhow!("send amount is too large"))?;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(20))
        .build()
        .context("failed to build LNURL client")?;

    let params = client
        .get(lnurl)
        .send()
        .await
        .context("failed to fetch LNURL pay request")?
        .error_for_status()
        .context("LNURL pay request returned an error")?
        .json::<LnurlPayParams>()
        .await
        .context("failed to parse LNURL pay request")?;

    if params.tag.as_deref() != Some("payRequest") {
        bail!("LNURL endpoint is not a pay request");
    }
    if params.min_sendable > params.max_sendable {
        bail!("LNURL endpoint returned an invalid amount range");
    }
    if amount_msat < params.min_sendable || amount_msat > params.max_sendable {
        bail!(
            "Amount must be between {} and {} sats.",
            msats_to_display_sats(params.min_sendable),
            msats_to_display_sats(params.max_sendable)
        );
    }

    let mut callback =
        reqwest::Url::parse(&params.callback).context("LNURL callback is not a valid URL")?;
    if callback.scheme() != "https" && callback.scheme() != "http" {
        bail!("LNURL callback must use http or https");
    }
    callback
        .query_pairs_mut()
        .append_pair("amount", &amount_msat.to_string());

    let invoice = client
        .get(callback)
        .send()
        .await
        .context("failed to fetch LNURL invoice")?
        .error_for_status()
        .context("LNURL invoice request returned an error")?
        .json::<LnurlPayInvoice>()
        .await
        .context("failed to parse LNURL invoice response")?;

    if invoice.status.as_deref() == Some("ERROR") {
        bail!(
            "{}",
            invoice
                .reason
                .filter(|reason| !reason.trim().is_empty())
                .unwrap_or_else(|| "LNURL endpoint returned an error".to_string())
        );
    }

    invoice
        .pr
        .filter(|pr| !pr.trim().is_empty())
        .ok_or_else(|| anyhow!("LNURL endpoint did not return an invoice"))
}

pub(crate) fn lnurl_pay_url(destination: &str) -> anyhow::Result<reqwest::Url> {
    let destination = strip_lightning_prefix(destination.trim());
    if is_valid_lightning_address(destination) {
        let (local, domain) = destination
            .split_once('@')
            .ok_or_else(|| anyhow!("invalid Lightning address"))?;
        let base = format!("https://{domain}");
        let mut url = reqwest::Url::parse(&base).context("invalid Lightning address domain")?;
        url.path_segments_mut()
            .map_err(|_| anyhow!("invalid Lightning address domain"))?
            .extend([".well-known", "lnurlp", local]);
        return Ok(url);
    }

    let lower = destination.to_ascii_lowercase();
    if lower.starts_with("lnurl") {
        let (hrp, bytes) = bech32::decode(destination).context("invalid LNURL")?;
        if hrp.to_string().to_ascii_lowercase() != "lnurl" {
            bail!("invalid LNURL prefix");
        }
        let url = String::from_utf8(bytes).context("LNURL does not contain a valid URL")?;
        let url = reqwest::Url::parse(&url).context("LNURL does not contain a valid URL")?;
        if url.scheme() != "https" && url.scheme() != "http" {
            bail!("LNURL must use http or https");
        }
        return Ok(url);
    }

    bail!("not a Lightning address or LNURL")
}

fn strip_lightning_prefix(destination: &str) -> &str {
    destination
        .strip_prefix("lightning:")
        .or_else(|| destination.strip_prefix("LIGHTNING:"))
        .unwrap_or(destination)
}

fn is_valid_lightning_address(address: &str) -> bool {
    let address = address.trim();
    let Some((local, domain)) = address.split_once('@') else {
        return false;
    };
    if local.is_empty() || domain.is_empty() || domain.contains('@') {
        return false;
    }
    if !local
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '+'))
    {
        return false;
    }
    let domain = domain.to_ascii_lowercase();
    if !domain.contains('.') || domain.starts_with('.') || domain.ends_with('.') {
        return false;
    }
    domain.split('.').all(|label| {
        !label.is_empty()
            && !label.starts_with('-')
            && !label.ends_with('-')
            && label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
    })
}

pub(crate) fn msats_to_display_sats(msats: u64) -> String {
    if msats % 1_000 == 0 {
        (msats / 1_000).to_string()
    } else {
        format!("{:.3}", msats as f64 / 1_000.0)
    }
}

pub(crate) async fn monitor_lightning_receive(
    wallet: Wallet,
    tx: Sender<CoreMsg>,
    payment_hash: PaymentHash,
) {
    let payment_hash_text = payment_hash.to_string();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10 * 60);
    let mut last_status = String::new();
    let mut last_paid = false;

    loop {
        if tokio::time::Instant::now() >= deadline {
            break;
        }

        let mut should_stop = false;
        match wallet.lightning_receive_status(payment_hash).await {
            Ok(Some(receive)) => {
                let (status, paid) = receive_status(&receive);
                send_receive_status_if_changed(
                    &tx,
                    &payment_hash_text,
                    &mut last_status,
                    &mut last_paid,
                    status,
                    paid,
                );

                if paid {
                    should_stop = true;
                } else {
                    send_receive_status_if_changed(
                        &tx,
                        &payment_hash_text,
                        &mut last_status,
                        &mut last_paid,
                        if receive.htlc_vtxos.is_empty() {
                            "waiting"
                        } else {
                            "claiming"
                        },
                        false,
                    );

                    if let Ok(Ok(receive)) = tokio::time::timeout(
                        Duration::from_secs(10),
                        wallet.try_claim_lightning_receive(payment_hash, false, None),
                    )
                    .await
                    {
                        let (status, paid) = receive_status(&receive);
                        send_receive_status_if_changed(
                            &tx,
                            &payment_hash_text,
                            &mut last_status,
                            &mut last_paid,
                            status,
                            paid,
                        );
                        should_stop = paid;
                    }
                }
            }
            Ok(None) => {
                send_receive_status_if_changed(
                    &tx,
                    &payment_hash_text,
                    &mut last_status,
                    &mut last_paid,
                    "waiting",
                    false,
                );
            }
            Err(e) => {
                let _ = tx.send(CoreMsg::Async(AsyncMsg::Error(format!(
                    "Lightning receive status failed: {e:#}"
                ))));
                break;
            }
        }

        if should_stop {
            let _ = tx.send(CoreMsg::Async(AsyncMsg::LightningReceiveClaimed {
                payment_hash: payment_hash_text,
            }));
            break;
        }

        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

fn receive_status(receive: &bark::persist::models::LightningReceive) -> (&'static str, bool) {
    if receive.preimage_revealed_at.is_some() || receive.finished_at.is_some() {
        ("paid", true)
    } else if receive.htlc_vtxos.is_empty() {
        ("waiting", false)
    } else {
        ("claimable", false)
    }
}

fn send_receive_status_if_changed(
    tx: &Sender<CoreMsg>,
    payment_hash: &str,
    last_status: &mut String,
    last_paid: &mut bool,
    status: &str,
    paid: bool,
) {
    if last_status == status && *last_paid == paid {
        return;
    }
    *last_status = status.to_string();
    *last_paid = paid;
    let _ = tx.send(CoreMsg::Async(AsyncMsg::LightningReceiveStatus {
        payment_hash: payment_hash.to_string(),
        status: status.to_string(),
        paid,
    }));
}

pub(crate) async fn monitor_ark_receive(wallet: Wallet, tx: Sender<CoreMsg>, address: ArkAddress) {
    let address_text = address.to_string();
    let payment_method = BarkPaymentMethod::Ark(address.clone());
    let mut movements = wallet
        .subscribe_notifications()
        .filter_arkoor_address_movements(address);
    let _ = tx.send(CoreMsg::Async(AsyncMsg::ArkAddress(address_text.clone())));
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10 * 60);

    loop {
        if tokio::time::Instant::now() >= deadline {
            break;
        }

        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        let Ok(Some(movement)) = tokio::time::timeout(remaining, movements.next()).await else {
            break;
        };
        if let Some(amount_sat) = ark_receive_amount(&movement, &payment_method) {
            send_ark_receive_confirmed(&tx, &address_text, amount_sat);
            break;
        }
    }
}

fn ark_receive_amount(movement: &Movement, payment_method: &BarkPaymentMethod) -> Option<u64> {
    if movement.status != MovementStatus::Successful {
        return None;
    }
    movement
        .received_on
        .iter()
        .find(|destination| destination.destination == *payment_method)
        .map(|destination| destination.amount.to_sat())
}

fn send_ark_receive_confirmed(tx: &Sender<CoreMsg>, address: &str, amount_sat: u64) {
    let _ = tx.send(CoreMsg::Async(AsyncMsg::ArkReceiveConfirmed {
        address: address.to_string(),
        amount_sat,
    }));
}

#[cfg(test)]
mod tests {
    use super::{decimal_btc_to_sat, embedded_send_amount_sat};

    #[test]
    fn extracts_amount_from_bitcoin_uri() {
        assert_eq!(
            embedded_send_amount_sat(
                "bitcoin:?amount=0.0005&lightning=lnbc1example&ark=tark1example"
            ),
            Some(50_000)
        );
        assert_eq!(
            embedded_send_amount_sat("bitcoin:bc1qexample?label=Rebel&amount=1.23456789"),
            Some(123_456_789)
        );
    }

    #[test]
    fn rejects_invalid_or_zero_bitcoin_amounts() {
        assert_eq!(decimal_btc_to_sat("0"), None);
        assert_eq!(decimal_btc_to_sat("0.00000000"), None);
        assert_eq!(decimal_btc_to_sat("0.000000001"), None);
        assert_eq!(decimal_btc_to_sat("-1"), None);
        assert_eq!(decimal_btc_to_sat("1.2.3"), None);
    }
}
