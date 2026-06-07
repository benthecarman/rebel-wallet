use std::time::Duration;

use anyhow::{anyhow, bail, Context};
use bark::ark::lightning::PaymentHash;
use bark::movement::PaymentMethod as BarkPaymentMethod;
use bark::Wallet;
use flume::Sender;
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
    let amount_sat = request.amount.map(|amount| amount.to_sat());
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

    let toast = request
        .options
        .iter()
        .find_map(|option| match &option.method {
            BarkPaymentMethod::Ark(_) | BarkPaymentMethod::Invoice(_) => option
                .errors
                .first()
                .map(|e| format!("Invalid payment request: {e}")),
            BarkPaymentMethod::Bitcoin(_) | BarkPaymentMethod::OutputScript(_) => {
                Some("On-chain payment QR codes are not supported yet.".to_string())
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

fn lnurl_pay_url(destination: &str) -> anyhow::Result<reqwest::Url> {
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

fn msats_to_display_sats(msats: u64) -> String {
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
