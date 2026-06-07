use std::time::Duration;

use bark::ark::lightning::PaymentHash;
use bark::movement::PaymentMethod as BarkPaymentMethod;
use bark::Wallet;
use flume::Sender;

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
