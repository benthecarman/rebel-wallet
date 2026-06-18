use std::str::FromStr;
use std::time::Duration;

use anyhow::Context;
use bark::ark::lightning::{Offer, OfferAmount};
use bark::lightning_invoice::Bolt11Invoice;
use bitcoin::{Address as BitcoinAddress, Amount};
use nostr_sdk::prelude::PublicKey as NostrPublicKey;

use super::AppCore;
use crate::nostr_support::{primal_profile_contacts, public_key_from_npub_or_hex};
use crate::payments::{
    embedded_send_amount_sat, is_lnurl_pay_destination, lightning_offer_amount_sat,
    parse_send_destination, resolve_lnurl_pay_invoice, send_destination_kind,
    strip_lightning_prefix,
};
use crate::persistence::PaymentAnnotation;
use crate::time::now_unix;
use crate::updates::{AsyncMsg, CoreMsg, HapticFeedback};
use crate::zaps::{fetch_received_zap_receipts, request_zap_invoice};
use crate::{Contact, SendDestinationKind, SendPhase};

const SEND_FEE_ESTIMATE_DEBOUNCE: Duration = Duration::from_millis(350);

fn msats_to_display_sats(msats: u64) -> String {
    if msats.is_multiple_of(1_000) {
        (msats / 1_000).to_string()
    } else {
        format!("{:.3}", msats as f64 / 1_000.0)
    }
}

pub(super) fn send_fee_estimate_request(
    destination: &str,
    amount_sat: u64,
) -> Option<(u64, SendDestinationKind)> {
    let destination = destination.trim();
    if destination.is_empty() {
        return None;
    }

    if is_lnurl_pay_destination(destination) {
        return (amount_sat > 0).then_some((amount_sat, SendDestinationKind::Lightning));
    }

    let lower = destination.to_lowercase();
    if lower.starts_with("lightning:") || lower.starts_with("ln") {
        if amount_sat > 0 {
            return Some((amount_sat, SendDestinationKind::Lightning));
        }

        let invoice_or_offer = strip_lightning_prefix(destination);
        if let Some(invoice_sat) = embedded_send_amount_sat(invoice_or_offer) {
            return Some((invoice_sat, SendDestinationKind::Lightning));
        }
        if let Some(offer_sat) = lightning_offer_amount_sat(invoice_or_offer) {
            return (offer_sat > 0).then_some((offer_sat, SendDestinationKind::Lightning));
        }
        return None;
    }

    let kind = match send_destination_kind(destination) {
        kind @ (SendDestinationKind::Ark | SendDestinationKind::OnChain) => kind,
        SendDestinationKind::Unknown | SendDestinationKind::Lightning => return None,
    };
    (amount_sat > 0).then_some((amount_sat, kind))
}

fn parsed_offer(destination: &str) -> Option<Offer> {
    let destination = destination
        .strip_prefix("lightning:")
        .or_else(|| destination.strip_prefix("LIGHTNING:"))
        .unwrap_or(destination);
    Offer::from_str(destination.trim()).ok()
}

async fn checked_bitcoin_address(
    wallet: &bark::Wallet,
    address: &str,
) -> anyhow::Result<BitcoinAddress> {
    let address = BitcoinAddress::from_str(address).context("invalid on-chain address")?;
    let network = wallet.network().await?;
    address
        .require_network(network)
        .context("address is not valid for configured network")
}

impl AppCore {
    pub(super) fn pay_destination(&mut self) {
        let destination = self.state.send.destination.trim().to_string();
        if destination.is_empty() {
            self.state.toast = Some("Enter a destination first.".to_string());
            self.request_haptic(HapticFeedback::NotificationWarning);
            return;
        }
        let total_cost_sat = self
            .state
            .send
            .total_cost_sat
            .unwrap_or(self.state.send.amount_sat);
        if total_cost_sat > self.state.wallet.balance_sat {
            self.state.toast = Some("Insufficient balance for this send.".to_string());
            self.request_haptic(HapticFeedback::NotificationWarning);
            return;
        }
        if self.state.send.zap_enabled {
            self.request_haptic(HapticFeedback::ImpactMedium);
            self.pay_zap_destination(destination, self.state.send.amount_sat);
        } else if is_lnurl_pay_destination(&destination) {
            self.request_haptic(HapticFeedback::ImpactMedium);
            self.pay_lnurl_destination(destination, self.state.send.amount_sat);
        } else {
            match self.state.send.destination_kind {
                SendDestinationKind::Lightning => {
                    self.request_haptic(HapticFeedback::ImpactMedium);
                    if let Some(offer) = parsed_offer(&destination) {
                        self.pay_lightning_offer(offer, self.state.send.amount_sat);
                    } else {
                        let invoice = strip_lightning_prefix(&destination).to_string();
                        self.pay_lightning_invoice(invoice, Some(self.state.send.amount_sat));
                    }
                }
                SendDestinationKind::OnChain => {
                    self.request_haptic(HapticFeedback::ImpactMedium);
                    self.pay_onchain_address(destination, self.state.send.amount_sat);
                }
                SendDestinationKind::Ark => {
                    self.request_haptic(HapticFeedback::ImpactMedium);
                    self.pay_ark_address(destination, self.state.send.amount_sat);
                }
                SendDestinationKind::Unknown => {
                    self.state.toast = Some("Enter a valid payment destination.".to_string());
                    self.request_haptic(HapticFeedback::NotificationWarning);
                }
            }
        }
    }

    pub(super) fn set_send_destination(&mut self, destination: String) {
        let raw = destination.trim().to_string();
        if raw.is_empty() {
            self.reset_send_draft();
            return;
        }

        let was_amount_locked = self.state.send.amount_locked;
        let parsed = self
            .wallet
            .clone()
            .and_then(|wallet| self.rt.block_on(parse_send_destination(wallet, &raw)));
        if let Some(parsed) = parsed {
            self.state.send.destination = parsed.destination;
            if let Some(amount_sat) = parsed.amount_sat {
                self.state.send.amount_sat = amount_sat;
                self.state.send.amount_locked = true;
            } else {
                if was_amount_locked {
                    self.state.send.amount_sat = 0;
                }
                self.state.send.amount_locked = false;
            }
            if let Some(memo) = parsed.memo.filter(|m| !m.trim().is_empty()) {
                self.state.send.memo = memo;
            }
            if let Some(toast) = parsed.toast {
                self.state.toast = Some(toast);
            }
        } else {
            self.state.send.destination = raw.clone();
            if let Some(amount_sat) = embedded_send_amount_sat(&raw) {
                self.state.send.amount_sat = amount_sat;
                self.state.send.amount_locked = true;
            } else {
                if was_amount_locked {
                    self.state.send.amount_sat = 0;
                }
                self.state.send.amount_locked = false;
            }
        }
        self.state.send.search_query = raw;
        self.state.send.phase = SendPhase::Editing;
        self.request_send_fee_estimate();
    }

    pub(super) fn select_send_contact(&mut self, contact_id: String) {
        if !self
            .state
            .nostr
            .contacts
            .iter()
            .any(|contact| contact.id == contact_id)
        {
            if let Some(contact) = self
                .state
                .send
                .search_results
                .iter()
                .find(|contact| contact.id == contact_id)
                .cloned()
            {
                self.state.nostr.contacts.push(contact);
                self.sort_contacts();
                self.save_app_data();
            }
        }

        let Some(contact) = self
            .state
            .nostr
            .contacts
            .iter_mut()
            .find(|contact| contact.id == contact_id)
        else {
            self.state.toast = Some("Contact not found.".to_string());
            self.request_haptic(HapticFeedback::NotificationWarning);
            return;
        };

        let destination = if !contact.lightning_address.trim().is_empty() {
            contact.lightning_address.clone()
        } else {
            contact.lnurl.clone()
        };

        if destination.trim().is_empty() {
            self.state.toast = Some("This contact does not have a Lightning address.".to_string());
            self.request_haptic(HapticFeedback::NotificationWarning);
            return;
        }

        contact.last_used = now_unix();
        self.state.send.selected_contact_id = Some(contact.id.clone());
        self.state.send.zap_enabled = false;
        self.state.send.zap_available = public_key_from_npub_or_hex(&contact.npub).is_ok();
        self.save_app_data();
        self.request_haptic(HapticFeedback::ImpactLight);
        self.set_send_destination(destination);
    }

    pub(super) fn reset_send_draft(&mut self) {
        self.state.send.destination.clear();
        self.state.send.search_query.clear();
        self.state.send.global_search_results.clear();
        self.state.send.selected_contact_id = None;
        self.state.send.zap_enabled = false;
        self.state.send.zap_available = false;
        self.state.send.amount_sat = 0;
        self.state.send.amount_locked = false;
        self.state.send.memo.clear();
        self.state.send.last_result = None;
        self.state.send.phase = SendPhase::Drafting;
        self.clear_send_fee_estimate();
    }

    pub(super) fn request_send_fee_estimate(&mut self) {
        self.send_fee_estimate_request_id = self.send_fee_estimate_request_id.saturating_add(1);

        let destination = self.state.send.destination.trim().to_string();
        if destination.is_empty() {
            self.clear_send_fee_estimate();
            return;
        }

        let amount_sat = self.state.send.amount_sat;
        let Some((estimate_amount_sat, kind)) = send_fee_estimate_request(&destination, amount_sat)
        else {
            self.clear_send_fee_estimate();
            return;
        };

        self.state.send.estimating_fee = true;
        let tx = self.tx.clone();
        let request_id = self.send_fee_estimate_request_id;
        self.rt.spawn(async move {
            tokio::time::sleep(SEND_FEE_ESTIMATE_DEBOUNCE).await;
            let _ = tx.send(CoreMsg::Async(AsyncMsg::SendFeeEstimateDue {
                request_id,
                destination,
                amount_sat,
                estimate_amount_sat,
                kind,
            }));
        });
    }

    pub(super) fn perform_send_fee_estimate(
        &mut self,
        request_id: u64,
        destination: String,
        amount_sat: u64,
        estimate_amount_sat: u64,
        kind: SendDestinationKind,
    ) {
        let Some(wallet) = self.wallet.clone() else {
            self.clear_send_fee_estimate();
            return;
        };

        let tx = self.tx.clone();
        self.rt.spawn(async move {
            let estimate_amount = Amount::from_sat(estimate_amount_sat);
            let result = match kind {
                SendDestinationKind::Lightning => {
                    wallet.estimate_lightning_send_fee(estimate_amount).await
                }
                SendDestinationKind::Ark => {
                    wallet.estimate_arkoor_payment_fee(estimate_amount).await
                }
                SendDestinationKind::OnChain => {
                    match checked_bitcoin_address(&wallet, &destination).await {
                        Ok(address) => {
                            wallet
                                .estimate_send_onchain(&address, estimate_amount)
                                .await
                        }
                        Err(e) => Err(e),
                    }
                }
                SendDestinationKind::Unknown => Err(anyhow::anyhow!("invalid payment destination")),
            };
            let msg = match result {
                Ok(estimate) => AsyncMsg::SendFeeEstimated {
                    request_id,
                    destination,
                    amount_sat,
                    fee_sat: estimate.fee.to_sat(),
                    total_sat: estimate.gross_amount.to_sat(),
                },
                Err(e) => AsyncMsg::SendFeeEstimateFailed {
                    request_id,
                    destination,
                    amount_sat,
                    error: format!("{e:#}"),
                },
            };
            let _ = tx.send(CoreMsg::Async(msg));
        });
    }

    pub(super) fn clear_send_fee_estimate(&mut self) {
        self.send_fee_estimate_request_id = self.send_fee_estimate_request_id.saturating_add(1);
        self.state.send.estimating_fee = false;
        self.state.send.fee_estimate_sat = None;
        self.state.send.total_cost_sat = None;
        self.state.send.fee_estimate_error = None;
    }

    pub(super) fn send_fee_estimate_is_current(
        &self,
        request_id: u64,
        destination: &str,
        amount_sat: u64,
    ) -> bool {
        self.send_fee_estimate_request_id == request_id
            && self.state.send.destination.trim() == destination
            && self.state.send.amount_sat == amount_sat
    }

    pub(super) fn clear_send_contact_selection(&mut self) {
        self.state.send.selected_contact_id = None;
        self.state.send.zap_enabled = false;
        self.state.send.zap_available = false;
    }

    fn selected_send_contact(&self) -> Option<Contact> {
        let contact_id = self.state.send.selected_contact_id.as_ref()?;
        self.state
            .nostr
            .contacts
            .iter()
            .find(|contact| &contact.id == contact_id)
            .cloned()
    }

    fn payment_annotation(
        &self,
        destination: String,
        invoice: Option<String>,
        amount_sat: i64,
        zap: bool,
    ) -> Option<PaymentAnnotation> {
        let contact_id = self.state.send.selected_contact_id.clone()?;
        Some(PaymentAnnotation {
            contact_id: Some(contact_id),
            label: None,
            destination,
            invoice,
            payment_hash: None,
            amount_sat,
            outbound: amount_sat < 0,
            zap,
            created_at: now_unix(),
        })
    }

    pub(super) fn upsert_payment_annotation(&mut self, annotation: PaymentAnnotation) {
        let duplicate = self.payment_annotations.iter().any(|existing| {
            existing.payment_hash.is_some() && existing.payment_hash == annotation.payment_hash
                || existing.invoice.is_some() && existing.invoice == annotation.invoice
        });
        if !duplicate {
            self.payment_annotations.push(annotation);
        }
    }

    pub(super) fn scan_zap_receipts(&self) {
        let keys = match self.nostr_keys() {
            Ok(keys) => keys,
            Err(_) => return,
        };
        let tx = self.tx.clone();
        self.rt.spawn(async move {
            let Ok(receipts) = fetch_received_zap_receipts(keys.public_key()).await else {
                return;
            };
            let pubkeys = receipts
                .iter()
                .filter_map(|receipt| NostrPublicKey::from_hex(&receipt.sender_pubkey).ok())
                .collect::<Vec<_>>();
            let records = primal_profile_contacts(pubkeys, false)
                .await
                .unwrap_or_default();
            let _ = tx.send(CoreMsg::Async(AsyncMsg::ZapReceiptsLoaded {
                receipts,
                records,
            }));
        });
    }

    pub(super) fn pay_lightning_invoice(&mut self, invoice: String, amount_sat: Option<u64>) {
        if let Some(amount_sat) = amount_sat.filter(|amount| *amount > 0) {
            if amount_sat > self.state.wallet.balance_sat {
                self.state.toast =
                    Some("Insufficient balance for this Lightning payment.".to_string());
                return;
            }
        }
        let Some(wallet) = self.wallet.clone() else {
            self.state.toast = Some("Wallet is not ready yet.".to_string());
            return;
        };
        self.state.busy.sending_payment = true;
        self.state.send.phase = SendPhase::Sending;
        self.state.send.last_result = None;
        let tx = self.tx.clone();
        let annotation = self.payment_annotation(
            String::new(),
            Some(invoice.clone()),
            amount_sat.map(|amount| -(amount as i64)).unwrap_or(0),
            false,
        );
        self.rt.spawn(async move {
            let user_amount = amount_sat.filter(|a| *a > 0).map(Amount::from_sat);
            let parsed = Bolt11Invoice::from_str(&invoice);
            let msg = match parsed {
                Ok(invoice) => {
                    let annotation = annotation.map(|mut annotation| {
                        annotation.payment_hash = Some(invoice.payment_hash().to_string());
                        annotation
                    });
                    match wallet
                        .pay_lightning_invoice(invoice, user_amount, true)
                        .await
                    {
                        Ok(_) => AsyncMsg::Paid {
                            result: "Lightning invoice paid.".to_string(),
                            annotation,
                        },
                        Err(e) => AsyncMsg::Error(format!("Lightning payment failed: {e:#}")),
                    }
                }
                Err(e) => AsyncMsg::Error(format!("Invalid Lightning invoice: {e}")),
            };
            let _ = tx.send(CoreMsg::Async(msg));
        });
    }

    fn pay_lightning_offer(&mut self, offer: Offer, amount_sat: u64) {
        let offer_text = offer.to_string();
        let payment_amount_sat = if amount_sat > 0 {
            Some(amount_sat)
        } else {
            offer_payment_amount_sat(&offer)
        };
        if payment_amount_sat.is_none() {
            self.state.toast =
                Some("Enter an amount before sending to this Lightning offer.".to_string());
            return;
        }
        if let Some(amount_sat) = payment_amount_sat {
            if amount_sat > self.state.wallet.balance_sat {
                self.state.toast =
                    Some("Insufficient balance for this Lightning payment.".to_string());
                return;
            }
        }
        let Some(wallet) = self.wallet.clone() else {
            self.state.toast = Some("Wallet is not ready yet.".to_string());
            return;
        };
        self.state.busy.sending_payment = true;
        self.state.send.phase = SendPhase::Sending;
        self.state.send.last_result = None;
        let tx = self.tx.clone();
        let user_amount = (amount_sat > 0).then_some(Amount::from_sat(amount_sat));
        let annotation = self.payment_annotation(
            offer_text,
            None,
            payment_amount_sat
                .map(|amount| -(amount as i64))
                .unwrap_or(0),
            false,
        );
        self.rt.spawn(async move {
            let msg = match wallet.pay_lightning_offer(offer, user_amount, true).await {
                Ok(invoice) => AsyncMsg::Paid {
                    result: "Lightning offer paid.".to_string(),
                    annotation: annotation.map(|mut annotation| {
                        annotation.invoice = Some(invoice.to_string());
                        annotation.payment_hash = Some(invoice.payment_hash().to_string());
                        annotation
                    }),
                },
                Err(e) => AsyncMsg::Error(format!("Lightning offer payment failed: {e:#}")),
            };
            let _ = tx.send(CoreMsg::Async(msg));
        });
    }

    fn pay_lnurl_destination(&mut self, destination: String, amount_sat: u64) {
        if amount_sat == 0 {
            self.state.toast =
                Some("Enter an amount before sending to this Lightning address.".to_string());
            return;
        }
        if amount_sat > self.state.wallet.balance_sat {
            self.state.toast = Some("Insufficient balance for this Lightning payment.".to_string());
            return;
        }
        let Some(wallet) = self.wallet.clone() else {
            self.state.toast = Some("Wallet is not ready yet.".to_string());
            return;
        };
        self.state.busy.sending_payment = true;
        self.state.send.phase = SendPhase::Sending;
        self.state.send.last_result = None;
        let tx = self.tx.clone();
        let annotation =
            self.payment_annotation(destination.clone(), None, -(amount_sat as i64), false);
        self.rt.spawn(async move {
            let msg = match resolve_lnurl_pay_invoice(&destination, amount_sat).await {
                Ok(invoice) => match Bolt11Invoice::from_str(&invoice) {
                    Ok(invoice) => match amount_sat.checked_mul(1_000) {
                        Some(requested_msat) => match invoice.amount_milli_satoshis() {
                            Some(invoice_msat) if invoice_msat == requested_msat => {
                                let invoice_text = invoice.to_string();
                                let payment_hash = invoice.payment_hash().to_string();
                                match wallet
                                    .pay_lightning_invoice(
                                        invoice,
                                        Some(Amount::from_sat(amount_sat)),
                                        true,
                                    )
                                    .await
                                {
                                    Ok(_) => AsyncMsg::Paid {
                                        result: "Lightning address payment sent.".to_string(),
                                        annotation: annotation.map(|mut annotation| {
                                            annotation.invoice = Some(invoice_text);
                                            annotation.payment_hash = Some(payment_hash);
                                            annotation
                                        }),
                                    },
                                    Err(e) => {
                                        AsyncMsg::Error(format!("Lightning payment failed: {e:#}"))
                                    }
                                }
                            }
                            Some(invoice_msat) => AsyncMsg::Error(format!(
                                "LNURL invoice amount mismatch: requested {} sats, got {} sats.",
                                amount_sat,
                                msats_to_display_sats(invoice_msat)
                            )),
                            None => AsyncMsg::Error(
                                "LNURL invoice did not include an amount.".to_string(),
                            ),
                        },
                        None => AsyncMsg::Error(
                            "LNURL payment failed: send amount is too large.".to_string(),
                        ),
                    },
                    Err(e) => AsyncMsg::Error(format!("Invalid LNURL invoice: {e}")),
                },
                Err(e) => AsyncMsg::Error(format!("LNURL payment failed: {e:#}")),
            };
            let _ = tx.send(CoreMsg::Async(msg));
        });
    }

    fn pay_zap_destination(&mut self, destination: String, amount_sat: u64) {
        if amount_sat == 0 {
            self.state.toast = Some("Enter an amount before sending a zap.".to_string());
            return;
        }
        if amount_sat > self.state.wallet.balance_sat {
            self.state.toast = Some("Insufficient balance for this zap.".to_string());
            return;
        }
        let Some(wallet) = self.wallet.clone() else {
            self.state.toast = Some("Wallet is not ready yet.".to_string());
            return;
        };
        let keys = match self.nostr_keys() {
            Ok(keys) => keys,
            Err(e) => {
                self.state.toast = Some(format!("{e:#}"));
                return;
            }
        };
        let Some(contact) = self.selected_send_contact() else {
            self.state.toast = Some("Select a zap-capable contact before zapping.".to_string());
            return;
        };
        let recipient_pubkey = match public_key_from_npub_or_hex(&contact.npub) {
            Ok(pubkey) => pubkey,
            Err(e) => {
                self.state.toast = Some(format!("Invalid contact Nostr key: {e:#}"));
                return;
            }
        };

        self.state.busy.sending_payment = true;
        self.state.send.phase = SendPhase::Sending;
        self.state.send.last_result = None;
        let tx = self.tx.clone();
        let memo = self.state.send.memo.clone();
        let annotation =
            self.payment_annotation(destination.clone(), None, -(amount_sat as i64), true);
        self.rt.spawn(async move {
            let msg =
                match request_zap_invoice(&destination, recipient_pubkey, amount_sat, &memo, &keys)
                    .await
                {
                    Ok(invoice) => match Bolt11Invoice::from_str(&invoice) {
                        Ok(invoice) => match amount_sat.checked_mul(1_000) {
                            Some(requested_msat) => match invoice.amount_milli_satoshis() {
                                Some(invoice_msat) if invoice_msat == requested_msat => {
                                    let invoice_text = invoice.to_string();
                                    let payment_hash = invoice.payment_hash().to_string();
                                    match wallet
                                        .pay_lightning_invoice(
                                            invoice,
                                            Some(Amount::from_sat(amount_sat)),
                                            true,
                                        )
                                        .await
                                    {
                                        Ok(_) => AsyncMsg::Paid {
                                            result: "Zap sent.".to_string(),
                                            annotation: annotation.map(|mut annotation| {
                                                annotation.invoice = Some(invoice_text);
                                                annotation.payment_hash = Some(payment_hash);
                                                annotation
                                            }),
                                        },
                                        Err(e) => {
                                            AsyncMsg::Error(format!("Zap payment failed: {e:#}"))
                                        }
                                    }
                                }
                                Some(invoice_msat) => AsyncMsg::Error(format!(
                                    "Zap invoice amount mismatch: requested {} sats, got {} sats.",
                                    amount_sat,
                                    msats_to_display_sats(invoice_msat)
                                )),
                                None => AsyncMsg::Error(
                                    "Zap invoice did not include an amount.".to_string(),
                                ),
                            },
                            None => {
                                AsyncMsg::Error("Zap failed: send amount is too large.".to_string())
                            }
                        },
                        Err(e) => AsyncMsg::Error(format!("Invalid zap invoice: {e}")),
                    },
                    Err(e) => AsyncMsg::Error(format!("Zap failed: {e:#}")),
                };
            let _ = tx.send(CoreMsg::Async(msg));
        });
    }

    pub(super) fn pay_ark_address(&mut self, address: String, amount_sat: u64) {
        if amount_sat == 0 {
            self.state.toast = Some("Enter an amount before sending.".to_string());
            return;
        }
        if amount_sat > self.state.wallet.balance_sat {
            self.state.toast = Some("Insufficient balance for this Ark payment.".to_string());
            return;
        }
        let Some(wallet) = self.wallet.clone() else {
            self.state.toast = Some("Wallet is not ready yet.".to_string());
            return;
        };
        self.state.busy.sending_payment = true;
        self.state.send.phase = SendPhase::Sending;
        self.state.send.last_result = None;
        let tx = self.tx.clone();
        let annotation =
            self.payment_annotation(address.clone(), None, -(amount_sat as i64), false);
        self.rt.spawn(async move {
            let msg = match address.parse() {
                Ok(address) => match wallet
                    .send_arkoor_payment(&address, Amount::from_sat(amount_sat))
                    .await
                {
                    Ok(_) => AsyncMsg::Paid {
                        result: "Ark payment sent.".to_string(),
                        annotation,
                    },
                    Err(e) => AsyncMsg::Error(format!("Ark payment failed: {e:#}")),
                },
                Err(e) => AsyncMsg::Error(format!("Invalid Ark address: {e}")),
            };
            let _ = tx.send(CoreMsg::Async(msg));
        });
    }

    fn pay_onchain_address(&mut self, address: String, amount_sat: u64) {
        if amount_sat == 0 {
            self.state.toast = Some("Enter an amount before sending.".to_string());
            return;
        }
        if amount_sat < 330 {
            self.state.toast = Some("Amount too low to send.".to_string());
            return;
        }
        if amount_sat > self.state.wallet.balance_sat {
            self.state.toast = Some("Insufficient balance for this on-chain payment.".to_string());
            return;
        }
        let Some(wallet) = self.wallet.clone() else {
            self.state.toast = Some("Wallet is not ready yet.".to_string());
            return;
        };
        self.state.busy.sending_payment = true;
        self.state.send.phase = SendPhase::Sending;
        self.state.send.last_result = None;
        let tx = self.tx.clone();
        let annotation =
            self.payment_annotation(address.clone(), None, -(amount_sat as i64), false);
        self.rt.spawn(async move {
            let msg = match checked_bitcoin_address(&wallet, &address).await {
                Ok(address) => match wallet
                    .send_onchain(address, Amount::from_sat(amount_sat))
                    .await
                {
                    Ok(_) => AsyncMsg::Paid {
                        result: "On-chain payment sent.".to_string(),
                        annotation,
                    },
                    Err(e) => AsyncMsg::Error(format!("On-chain payment failed: {e:#}")),
                },
                Err(e) => AsyncMsg::Error(format!("{e:#}")),
            };
            let _ = tx.send(CoreMsg::Async(msg));
        });
    }
}

fn offer_payment_amount_sat(offer: &Offer) -> Option<u64> {
    match offer.amount()? {
        OfferAmount::Bitcoin { amount_msats } => {
            let sat = amount_msats.checked_add(999)? / 1_000;
            (sat > 0).then_some(sat)
        }
        OfferAmount::Currency { .. } => None,
    }
}
