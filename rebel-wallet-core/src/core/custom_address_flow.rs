use std::str::FromStr;
use std::time::Duration;

use anyhow::Context;
use bark::ark::Address as ArkAddress;
use bark::Wallet;
use bitcoin::Amount;

use super::AppCore;
use crate::custom_address::{
    amount_msats_to_sat, quote_registration, register_address, validate_custom_address_name,
    verify_registration, RegisterResult, RegisterStatus,
};
use crate::persistence::{PaymentAnnotation, PendingCustomLightningAddress};
use crate::state::arkzap_domain_for_ark_address;
use crate::time::now_unix;
use crate::updates::{AsyncMsg, CoreMsg, HapticFeedback};
use crate::LightningAddressRegistrationPhase;

async fn register_custom_lightning_address(
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
            response, true, true, false, false, None,
        ));
    }

    if response.invoice.trim().is_empty() {
        anyhow::bail!("registration response did not include an invoice");
    }
    Ok(registration_update_from_result(
        response, false, false, false, true, None,
    ))
}

#[allow(clippy::too_many_arguments)]
async fn pay_and_verify_custom_lightning_address_registration(
    wallet: Wallet,
    domain: String,
    name: String,
    lightning_address: String,
    ark_address: String,
    invoice: String,
    purchase_id: String,
    amount_sat: u64,
) -> AsyncMsg {
    let annotation = custom_address_registration_payment_annotation(&ark_address, amount_sat);
    match pay_registration_ark_address(&wallet, &ark_address, amount_sat).await {
        Ok(()) => {
            let client = match reqwest::Client::builder()
                .timeout(Duration::from_secs(20))
                .build()
                .context("failed to build custom address client")
            {
                Ok(client) => client,
                Err(e) => {
                    return registration_pending_update(
                        name,
                        lightning_address,
                        ark_address,
                        invoice,
                        purchase_id,
                        amount_sat,
                        true,
                        true,
                        Some(annotation),
                        Some(format!("Payment sent. Status check failed: {e:#}")),
                    );
                }
            };
            for _ in 0..12 {
                let status = match verify_registration(&client, &domain, &purchase_id).await {
                    Ok(status) => status,
                    Err(e) => {
                        return registration_pending_update(
                            name,
                            lightning_address,
                            ark_address,
                            invoice,
                            purchase_id,
                            amount_sat,
                            true,
                            true,
                            Some(annotation),
                            Some(format!("Payment sent. Status check failed: {e:#}")),
                        );
                    }
                };
                if status.active {
                    return registration_update_from_status_with_payment(
                        status,
                        true,
                        Some(annotation),
                    );
                }
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
            registration_pending_update(
                name,
                lightning_address,
                ark_address,
                invoice,
                purchase_id,
                amount_sat,
                true,
                true,
                Some(annotation),
                Some("Payment sent. Check status in a moment.".to_string()),
            )
        }
        Err(e) => registration_pending_update(
            name,
            lightning_address,
            ark_address,
            invoice,
            purchase_id,
            amount_sat,
            false,
            false,
            None,
            Some(format!(
                "Could not pay from wallet balance over Ark: {e:#}. Scan the invoice to pay externally."
            )),
        ),
    }
}

fn custom_address_registration_payment_annotation(
    ark_address: &str,
    amount_sat: u64,
) -> PaymentAnnotation {
    PaymentAnnotation {
        contact_id: None,
        label: Some("Custom Lightning address registration".to_string()),
        destination: ark_address.to_string(),
        invoice: None,
        payment_hash: None,
        amount_sat: -i64::try_from(amount_sat).unwrap_or(i64::MAX),
        outbound: true,
        zap: false,
        created_at: now_unix(),
    }
}

async fn pay_registration_ark_address(
    wallet: &Wallet,
    ark_address_text: &str,
    amount_sat: u64,
) -> anyhow::Result<()> {
    let ark_address =
        ArkAddress::from_str(ark_address_text).context("registration Ark address was invalid")?;
    wallet
        .send_arkoor_payment(&ark_address, Amount::from_sat(amount_sat))
        .await
        .context("Ark payment failed")?;
    Ok(())
}

fn registration_update_from_result(
    response: RegisterResult,
    active: bool,
    paid: bool,
    paid_from_wallet: bool,
    requires_confirmation: bool,
    warning: Option<String>,
) -> AsyncMsg {
    AsyncMsg::LightningAddressRegistrationUpdated {
        name: response.name,
        lightning_address: response.lightning_address,
        payment_ark_address: response.ark_address,
        invoice: Some(response.invoice),
        purchase_id: Some(response.id.to_string()),
        amount_msats: Some(response.fee_sats.saturating_mul(1_000)),
        active,
        paid: paid || response.state == "settled",
        paid_from_wallet,
        requires_confirmation,
        annotation: None,
        warning,
    }
}

#[allow(clippy::too_many_arguments)]
fn registration_pending_update(
    name: String,
    lightning_address: String,
    payment_ark_address: String,
    invoice: String,
    purchase_id: String,
    amount_sat: u64,
    paid: bool,
    paid_from_wallet: bool,
    annotation: Option<PaymentAnnotation>,
    warning: Option<String>,
) -> AsyncMsg {
    AsyncMsg::LightningAddressRegistrationUpdated {
        name,
        lightning_address,
        payment_ark_address,
        invoice: Some(invoice),
        purchase_id: Some(purchase_id),
        amount_msats: Some(amount_sat.saturating_mul(1_000)),
        active: false,
        paid,
        paid_from_wallet,
        requires_confirmation: false,
        annotation,
        warning,
    }
}

fn registration_update_from_status(status: RegisterStatus) -> AsyncMsg {
    registration_update_from_status_with_payment(status, false, None)
}

fn registration_update_from_status_with_payment(
    status: RegisterStatus,
    paid_from_wallet: bool,
    annotation: Option<PaymentAnnotation>,
) -> AsyncMsg {
    AsyncMsg::LightningAddressRegistrationUpdated {
        name: status.name,
        lightning_address: status.lightning_address,
        payment_ark_address: status.ark_address,
        invoice: Some(status.invoice),
        purchase_id: Some(status.id.to_string()),
        amount_msats: Some(status.fee_sats.saturating_mul(1_000)),
        active: status.active,
        paid: status.state == "settled" || status.active,
        paid_from_wallet,
        requires_confirmation: false,
        annotation,
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
        self.state
            .lightning_address
            .registration_payment_ark_address = None;
        self.state.lightning_address.registration_invoice = None;
        self.state.lightning_address.registration_purchase_id = None;
        self.state.lightning_address.registration_amount_sat = 0;
        self.state
            .lightning_address
            .registration_requires_confirmation = false;
        self.request_haptic(HapticFeedback::ImpactMedium);

        let tx = self.tx.clone();
        self.rt.spawn(async move {
            let msg = register_custom_lightning_address(wallet, domain, name, ark_address)
                .await
                .unwrap_or_else(|e| {
                    AsyncMsg::Error(format!(
                        "Custom Lightning address registration failed: {e:#}"
                    ))
                });
            let _ = tx.send(CoreMsg::Async(msg));
        });
    }

    pub(super) fn confirm_lightning_address_registration_payment(&mut self) {
        let Some(wallet) = self.wallet.clone() else {
            self.state.toast = Some("Wallet is not ready yet.".to_string());
            self.request_haptic(HapticFeedback::NotificationWarning);
            return;
        };
        let name = self.state.lightning_address.custom_name.clone();
        let Some(lightning_address) = self
            .state
            .lightning_address
            .registration_address
            .clone()
            .filter(|address| !address.trim().is_empty())
        else {
            self.state.toast = Some("No custom address registration to pay.".to_string());
            self.request_haptic(HapticFeedback::NotificationWarning);
            return;
        };
        let Some(backing_ark_address) = self
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
        let Some(invoice) = self
            .state
            .lightning_address
            .registration_invoice
            .clone()
            .filter(|invoice| !invoice.trim().is_empty())
        else {
            self.state.toast = Some("No registration invoice to pay.".to_string());
            self.request_haptic(HapticFeedback::NotificationWarning);
            return;
        };
        let Some(purchase_id) = self
            .state
            .lightning_address
            .registration_purchase_id
            .clone()
            .filter(|id| !id.trim().is_empty())
        else {
            self.state.toast = Some("No registration invoice to pay.".to_string());
            self.request_haptic(HapticFeedback::NotificationWarning);
            return;
        };
        let amount_sat = self.state.lightning_address.registration_amount_sat;
        if amount_sat == 0 {
            self.state.toast = Some("Registration amount is not ready yet.".to_string());
            self.request_haptic(HapticFeedback::NotificationWarning);
            return;
        }
        if amount_sat > self.state.wallet.balance_sat {
            self.state
                .lightning_address
                .registration_requires_confirmation = false;
            self.state.lightning_address.registration_error =
                Some("Insufficient balance. Scan the invoice to pay externally.".to_string());
            self.state.toast =
                Some("Insufficient balance. Scan the invoice to pay externally.".to_string());
            self.request_haptic(HapticFeedback::NotificationWarning);
            self.save_app_data();
            return;
        }

        let Some(payment_ark_address) = self
            .state
            .lightning_address
            .registration_payment_ark_address
            .clone()
            .filter(|address| !address.trim().is_empty())
        else {
            self.state.toast = Some("No registration payment address to pay.".to_string());
            self.request_haptic(HapticFeedback::NotificationWarning);
            return;
        };

        let domain = arkzap_domain_for_ark_address(&backing_ark_address).to_string();
        self.state.lightning_address.registration_phase =
            LightningAddressRegistrationPhase::Verifying;
        self.state.lightning_address.registration_status_text = "Paying".to_string();
        self.state.lightning_address.registration_error = None;
        self.state
            .lightning_address
            .registration_requires_confirmation = false;
        self.request_haptic(HapticFeedback::ImpactMedium);

        let tx = self.tx.clone();
        self.rt.spawn(async move {
            let msg = pay_and_verify_custom_lightning_address_registration(
                wallet,
                domain,
                name,
                lightning_address,
                payment_ark_address,
                invoice,
                purchase_id,
                amount_sat,
            )
            .await;
            let _ = tx.send(CoreMsg::Async(msg));
        });
    }

    pub(super) fn cancel_lightning_address_registration_payment(&mut self) {
        self.clear_lightning_address_registration();
        self.restore_active_custom_lightning_address_name();
        self.save_app_data();
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
        self.state
            .lightning_address
            .registration_requires_confirmation = false;
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
        self.state
            .lightning_address
            .registration_payment_ark_address = None;
        self.state.lightning_address.registration_invoice = None;
        self.state.lightning_address.registration_purchase_id = None;
        self.state.lightning_address.registration_amount_sat = 0;
        self.state
            .lightning_address
            .registration_requires_confirmation = false;
    }

    pub(super) fn restore_active_custom_lightning_address_name(&mut self) {
        if let Some(name) = self
            .state
            .lightning_address
            .custom_address
            .as_deref()
            .and_then(lightning_address_local_part)
        {
            self.state.lightning_address.custom_name = name.to_string();
        }
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
        payment_ark_address: String,
        invoice: Option<String>,
        purchase_id: Option<String>,
        amount_msats: Option<u64>,
        active: bool,
        paid: bool,
        paid_from_wallet: bool,
        requires_confirmation: bool,
        warning: Option<String>,
    ) {
        self.state.lightning_address.custom_name = name;
        self.state.lightning_address.registration_address = Some(lightning_address.clone());
        self.state
            .lightning_address
            .registration_payment_ark_address = Some(payment_ark_address);
        self.state.lightning_address.registration_invoice = invoice;
        self.state.lightning_address.registration_purchase_id = purchase_id;
        self.state.lightning_address.registration_amount_sat = amount_msats
            .and_then(|amount| amount_msats_to_sat(amount).ok())
            .unwrap_or(0);
        self.state
            .lightning_address
            .registration_requires_confirmation = requires_confirmation;

        if active {
            self.state.lightning_address.custom_address = Some(lightning_address);
            self.state.lightning_address.registration_phase =
                LightningAddressRegistrationPhase::Active;
            self.state.lightning_address.registration_status_text = "Active".to_string();
            self.state.lightning_address.registration_error = None;
            self.state.lightning_address.registration_invoice = None;
            self.state.lightning_address.registration_purchase_id = None;
            self.state.lightning_address.registration_amount_sat = 0;
            self.state
                .lightning_address
                .registration_requires_confirmation = false;
            self.state.toast = Some(if paid_from_wallet {
                "Custom Lightning address registered and paid.".to_string()
            } else {
                "Custom Lightning address registered.".to_string()
            });
            self.request_haptic(HapticFeedback::NotificationSuccess);
        } else {
            self.state.lightning_address.registration_phase =
                LightningAddressRegistrationPhase::AwaitingPayment;
            self.state.lightning_address.registration_status_text = if requires_confirmation {
                "Confirm payment".to_string()
            } else if paid {
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
            payment_ark_address: self
                .state
                .lightning_address
                .registration_payment_ark_address
                .clone()
                .filter(|address| !address.trim().is_empty()),
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
    lightning_address_local_part(address).is_some_and(|local_part| local_part == name)
}

pub(super) fn lightning_address_local_part(address: &str) -> Option<&str> {
    address
        .split_once('@')
        .map(|(local_part, _)| local_part)
        .filter(|local_part| !local_part.trim().is_empty())
}
