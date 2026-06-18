use std::str::FromStr;
use std::time::Duration;

use anyhow::Context;
use bark::ark::Address as ArkAddress;
use bark::lightning_invoice::Bolt11Invoice;
use bark::Wallet;
use bitcoin::Amount;

use super::{msats_to_display_sats, AppCore};
use crate::custom_address::{
    amount_msats_to_sat, quote_registration, register_address, validate_custom_address_name,
    verify_registration, RegisterResult, RegisterStatus,
};
use crate::persistence::PendingCustomLightningAddress;
use crate::state::arkzap_domain_for_ark_address;
use crate::updates::{AsyncMsg, CoreMsg, HapticFeedback};
use crate::LightningAddressRegistrationPhase;

async fn register_and_pay_custom_lightning_address(
    wallet: Wallet,
    domain: String,
    name: String,
    ark_address_text: String,
) -> anyhow::Result<AsyncMsg> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(20))
        .build()
        .context("failed to build custom address client")?;
    let quote = quote_registration(&client, &domain, &name, &ark_address_text).await?;
    let ark_address = ArkAddress::from_str(&quote.ark_address).context("invalid Ark address")?;
    let signature = wallet
        .sign_address_message(&ark_address, quote.message.as_bytes())
        .await
        .context("failed to sign custom address registration")?
        .to_string();

    let response = register_address(
        &client,
        &domain,
        &quote.name,
        &quote.ark_address,
        &signature,
    )
    .await?;

    if response.active {
        return Ok(registration_update_from_result(
            response, true, true, false, None,
        ));
    }

    let amount_sat = response.fee_sats;
    let invoice = response.invoice.clone();
    if invoice.trim().is_empty() {
        anyhow::bail!("registration response did not include an invoice");
    }
    let purchase_id = response.id.to_string();
    let pending_response = RegisterResult { ..response };

    let balance = wallet.balance().await.context("balance failed")?;
    if balance.spendable.to_sat() < amount_sat {
        return Ok(registration_update_from_result(
            pending_response,
            false,
            false,
            false,
            None,
        ));
    }

    match pay_registration_invoice(&wallet, &invoice, amount_sat).await {
        Ok(()) => {
            for _ in 0..12 {
                let status = verify_registration(&client, &domain, &purchase_id).await?;
                if status.active {
                    return Ok(registration_update_from_status_with_payment(status, true));
                }
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
            Ok(registration_update_from_result(
                pending_response,
                false,
                true,
                true,
                Some("Payment sent. Check status in a moment.".to_string()),
            ))
        }
        Err(e) => Ok(registration_update_from_result(
            pending_response,
            false,
            false,
            false,
            Some(format!(
                "Could not pay from wallet balance: {e:#}. Scan the invoice to pay externally."
            )),
        )),
    }
}

async fn pay_registration_invoice(
    wallet: &Wallet,
    invoice_text: &str,
    amount_sat: u64,
) -> anyhow::Result<()> {
    let invoice =
        Bolt11Invoice::from_str(invoice_text).context("registration invoice was invalid")?;
    let expected_msat = amount_sat
        .checked_mul(1_000)
        .ok_or_else(|| anyhow::anyhow!("registration amount is too large"))?;
    match invoice.amount_milli_satoshis() {
        Some(invoice_msat) if invoice_msat == expected_msat => {}
        Some(invoice_msat) => anyhow::bail!(
            "registration invoice amount mismatch: requested {} sats, got {} sats",
            amount_sat,
            msats_to_display_sats(invoice_msat)
        ),
        None => anyhow::bail!("registration invoice did not include an amount"),
    }
    wallet
        .pay_lightning_invoice(invoice, Some(Amount::from_sat(amount_sat)), true)
        .await
        .context("Lightning payment failed")?;
    Ok(())
}

fn registration_update_from_result(
    response: RegisterResult,
    active: bool,
    paid: bool,
    paid_from_wallet: bool,
    warning: Option<String>,
) -> AsyncMsg {
    AsyncMsg::LightningAddressRegistrationUpdated {
        name: response.name,
        lightning_address: response.lightning_address,
        ark_address: response.ark_address,
        invoice: Some(response.invoice),
        purchase_id: Some(response.id.to_string()),
        amount_msats: Some(response.fee_sats.saturating_mul(1_000)),
        active,
        paid: paid || response.state == "settled",
        paid_from_wallet,
        warning,
    }
}

fn registration_update_from_status(status: RegisterStatus) -> AsyncMsg {
    registration_update_from_status_with_payment(status, false)
}

fn registration_update_from_status_with_payment(
    status: RegisterStatus,
    paid_from_wallet: bool,
) -> AsyncMsg {
    AsyncMsg::LightningAddressRegistrationUpdated {
        name: status.name,
        lightning_address: status.lightning_address,
        ark_address: status.ark_address,
        invoice: Some(status.invoice),
        purchase_id: Some(status.id.to_string()),
        amount_msats: Some(status.fee_sats.saturating_mul(1_000)),
        active: status.active,
        paid: status.state == "settled" || status.active,
        paid_from_wallet,
        warning: None,
    }
}

impl AppCore {
    pub(super) fn ensure_lightning_address(&mut self) {
        if let Some(address) = self.load_lightning_address_ark_address() {
            self.state.lightning_address.backing_ark_address = Some(address);
            return;
        }
        if self
            .state
            .lightning_address
            .backing_ark_address
            .as_ref()
            .is_some_and(|address| !address.trim().is_empty())
        {
            if let Some(address) = self.state.lightning_address.backing_ark_address.as_ref() {
                self.save_lightning_address_ark_address(address);
            }
            return;
        }
        let Some(wallet) = self.wallet.clone() else {
            return;
        };
        let tx = self.tx.clone();
        self.rt.spawn(async move {
            let msg = match wallet.new_address().await {
                Ok(address) => AsyncMsg::LightningAddressReady(address.to_string()),
                Err(e) => AsyncMsg::Error(format!("Could not create Arkzap address: {e:#}")),
            };
            let _ = tx.send(CoreMsg::Async(msg));
        });
    }

    pub(super) fn register_lightning_address(&mut self) {
        self.state.refresh_derived();
        let name = self.state.lightning_address.custom_name.trim().to_string();
        if let Some(error) = validate_custom_address_name(&name) {
            self.state.lightning_address.registration_error = Some(error);
            self.request_haptic(HapticFeedback::NotificationWarning);
            return;
        }

        let Some(wallet) = self.wallet.clone() else {
            self.state.toast = Some("Wallet is not ready yet.".to_string());
            self.request_haptic(HapticFeedback::NotificationWarning);
            return;
        };
        let Some(ark_address) = self
            .state
            .lightning_address
            .backing_ark_address
            .clone()
            .filter(|address| !address.trim().is_empty())
        else {
            self.ensure_lightning_address();
            self.state.toast = Some("Preparing Arkzap address.".to_string());
            self.request_haptic(HapticFeedback::NotificationWarning);
            return;
        };

        let domain = arkzap_domain_for_ark_address(&ark_address).to_string();
        self.state.lightning_address.registration_phase =
            LightningAddressRegistrationPhase::Registering;
        self.state.lightning_address.registration_status_text = "Registering".to_string();
        self.state.lightning_address.registration_error = None;
        self.state.lightning_address.registration_address = None;
        self.state.lightning_address.registration_invoice = None;
        self.state.lightning_address.registration_purchase_id = None;
        self.state.lightning_address.registration_amount_sat = 0;
        self.request_haptic(HapticFeedback::ImpactMedium);

        let tx = self.tx.clone();
        self.rt.spawn(async move {
            let msg = register_and_pay_custom_lightning_address(wallet, domain, name, ark_address)
                .await
                .unwrap_or_else(|e| {
                    AsyncMsg::Error(format!(
                        "Custom Lightning address registration failed: {e:#}"
                    ))
                });
            let _ = tx.send(CoreMsg::Async(msg));
        });
    }

    pub(super) fn verify_lightning_address_registration(&mut self) {
        let Some(purchase_id) = self
            .state
            .lightning_address
            .registration_purchase_id
            .clone()
            .filter(|id| !id.trim().is_empty())
        else {
            self.state.toast = Some("No registration invoice to check.".to_string());
            self.request_haptic(HapticFeedback::NotificationWarning);
            return;
        };
        let Some(ark_address) = self
            .state
            .lightning_address
            .backing_ark_address
            .clone()
            .filter(|address| !address.trim().is_empty())
        else {
            self.state.toast = Some("Arkzap address is not ready yet.".to_string());
            self.request_haptic(HapticFeedback::NotificationWarning);
            return;
        };
        let domain = arkzap_domain_for_ark_address(&ark_address).to_string();

        self.state.lightning_address.registration_phase =
            LightningAddressRegistrationPhase::Verifying;
        self.state.lightning_address.registration_status_text = "Checking".to_string();
        self.state.lightning_address.registration_error = None;
        self.request_haptic(HapticFeedback::ImpactLight);

        let tx = self.tx.clone();
        self.rt.spawn(async move {
            let client = reqwest::Client::builder()
                .timeout(Duration::from_secs(20))
                .build()
                .context("failed to build custom address client");
            let msg = match client {
                Ok(client) => verify_registration(&client, &domain, &purchase_id)
                    .await
                    .map(registration_update_from_status),
                Err(e) => Err(e),
            }
            .unwrap_or_else(|e| {
                AsyncMsg::Error(format!("Custom Lightning address status failed: {e:#}"))
            });
            let _ = tx.send(CoreMsg::Async(msg));
        });
    }

    pub(super) fn clear_lightning_address_registration(&mut self) {
        let has_custom_address = self
            .state
            .lightning_address
            .custom_address
            .as_ref()
            .is_some_and(|address| !address.trim().is_empty());
        self.state.lightning_address.registration_phase = if has_custom_address {
            LightningAddressRegistrationPhase::Active
        } else {
            LightningAddressRegistrationPhase::Idle
        };
        self.state.lightning_address.registration_status_text = if has_custom_address {
            "Active".to_string()
        } else {
            "Ready".to_string()
        };
        self.state.lightning_address.registration_error = None;
        self.state.lightning_address.registration_address = None;
        self.state.lightning_address.registration_invoice = None;
        self.state.lightning_address.registration_purchase_id = None;
        self.state.lightning_address.registration_amount_sat = 0;
    }

    pub(super) fn clear_stale_lightning_address_registration_for_name(&mut self, name: &str) {
        let pending_matches_name = self
            .state
            .lightning_address
            .registration_address
            .as_deref()
            .and_then(|address| address.split_once('@').map(|(local_part, _)| local_part))
            .is_some_and(|local_part| local_part == name);
        if pending_matches_name {
            return;
        }
        self.clear_lightning_address_registration();
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn apply_lightning_address_registration_update(
        &mut self,
        name: String,
        lightning_address: String,
        ark_address: String,
        invoice: Option<String>,
        purchase_id: Option<String>,
        amount_msats: Option<u64>,
        active: bool,
        paid: bool,
        paid_from_wallet: bool,
        warning: Option<String>,
    ) {
        self.state.lightning_address.custom_name = name;
        self.state.lightning_address.registration_address = Some(lightning_address.clone());
        self.state.lightning_address.backing_ark_address = Some(ark_address.clone());
        self.save_lightning_address_ark_address(&ark_address);
        self.state.lightning_address.registration_invoice = invoice;
        self.state.lightning_address.registration_purchase_id = purchase_id;
        self.state.lightning_address.registration_amount_sat = amount_msats
            .and_then(|amount| amount_msats_to_sat(amount).ok())
            .unwrap_or(0);

        if active {
            self.state.lightning_address.custom_address = Some(lightning_address);
            self.state.lightning_address.registration_phase =
                LightningAddressRegistrationPhase::Active;
            self.state.lightning_address.registration_status_text = "Active".to_string();
            self.state.lightning_address.registration_error = None;
            self.state.lightning_address.registration_invoice = None;
            self.state.lightning_address.registration_purchase_id = None;
            self.state.lightning_address.registration_amount_sat = 0;
            self.state.toast = Some(if paid_from_wallet {
                "Custom Lightning address registered and paid.".to_string()
            } else {
                "Custom Lightning address registered.".to_string()
            });
            self.request_haptic(HapticFeedback::NotificationSuccess);
        } else {
            self.state.lightning_address.registration_phase =
                LightningAddressRegistrationPhase::AwaitingPayment;
            self.state.lightning_address.registration_status_text = if paid {
                "Payment received".to_string()
            } else {
                "Awaiting payment".to_string()
            };
            self.state.lightning_address.registration_error = warning.clone();
            if let Some(warning) = warning {
                self.state.toast = Some(warning);
                self.request_haptic(HapticFeedback::NotificationWarning);
            }
        }

        self.save_app_data();
        self.sync_wallet();
    }

    pub(super) fn pending_custom_lightning_address(&self) -> Option<PendingCustomLightningAddress> {
        if !matches!(
            self.state.lightning_address.registration_phase,
            LightningAddressRegistrationPhase::AwaitingPayment
        ) {
            return None;
        }
        let registration_address = self
            .state
            .lightning_address
            .registration_address
            .clone()
            .filter(|address| !address.trim().is_empty())?;
        if !lightning_address_matches_name(
            &registration_address,
            &self.state.lightning_address.custom_name,
        ) {
            return None;
        }
        Some(PendingCustomLightningAddress {
            name: self.state.lightning_address.custom_name.clone(),
            lightning_address: registration_address,
            ark_address: self
                .state
                .lightning_address
                .backing_ark_address
                .clone()
                .filter(|address| !address.trim().is_empty())?,
            invoice: self
                .state
                .lightning_address
                .registration_invoice
                .clone()
                .filter(|invoice| !invoice.trim().is_empty())?,
            purchase_id: self
                .state
                .lightning_address
                .registration_purchase_id
                .clone()
                .filter(|purchase_id| !purchase_id.trim().is_empty())?,
            amount_msats: self
                .state
                .lightning_address
                .registration_amount_sat
                .checked_mul(1_000)?,
        })
    }
}

pub(super) fn pending_custom_lightning_address_matches_name(
    pending: &PendingCustomLightningAddress,
) -> bool {
    lightning_address_matches_name(&pending.lightning_address, &pending.name)
}

pub(super) fn lightning_address_matches_name(address: &str, name: &str) -> bool {
    address
        .split_once('@')
        .map(|(local_part, _)| local_part == name)
        .unwrap_or(false)
}
