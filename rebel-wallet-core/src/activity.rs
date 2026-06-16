use bark::movement::{Movement, PaymentMethod as BarkPaymentMethod};
use bark::subsystem::{RoundMovement, Subsystem};

use crate::{state, ActivityIconKind, ActivityItem, Contact};

pub(crate) fn activity_from_movement(
    movement: Movement,
    contacts: &[Contact],
    lightning_address: Option<&str>,
    lightning_address_ark_address: Option<&str>,
) -> ActivityItem {
    let amount_sat = activity_amount_sat(&movement);
    let inbound = amount_sat >= 0;
    let payment_amount_sat = activity_payment_amount_sat(&movement, inbound).unwrap_or(amount_sat);
    let destination = if inbound {
        movement.received_on.first()
    } else {
        movement.sent_to.first()
    };
    let ark_address = destination
        .and_then(|destination| ark_address_from_payment_method(&destination.destination))
        .or_else(|| {
            let destinations = if inbound {
                &movement.received_on
            } else {
                &movement.sent_to
            };
            destinations
                .iter()
                .find_map(|destination| ark_address_from_payment_method(&destination.destination))
        });
    let is_lightning_address_receive = inbound
        && ark_address_matches(
            ark_address.as_deref(),
            lightning_address_ark_address.filter(|address| !address.trim().is_empty()),
        );
    let contact = destination
        .and_then(|d| contact_for_payment_method(&d.destination, contacts))
        .cloned();
    let method = if is_lightning_address_receive {
        match lightning_address.filter(|address| !address.trim().is_empty()) {
            Some(address) => format!("Lightning address {}", truncate_middle(address, 28)),
            None => "Lightning address".to_string(),
        }
    } else {
        destination
            .map(|d| {
                format!(
                    "{} {}",
                    d.destination.type_str(),
                    truncate_middle(&d.destination.value_string(), 28)
                )
            })
            .unwrap_or_else(|| activity_subsystem_label(&movement))
    };
    let title = if inbound {
        format!("Received {}", method)
    } else {
        format!("Sent {}", method)
    };
    let fee = movement.offchain_fee.to_sat();
    let subtitle = if fee > 0 {
        format!("{fee} sats fee")
    } else {
        String::new()
    };
    let display_counterparty = if let Some(contact) = &contact {
        if contact.name.is_empty() {
            "Unknown".to_string()
        } else {
            contact.name.clone()
        }
    } else {
        "Unknown".to_string()
    };
    let method_icon = if is_lightning_address_receive {
        "bolt.fill"
    } else {
        activity_method_icon(destination.map(|d| &d.destination), inbound)
    }
    .to_string();
    let method_display = if is_lightning_address_receive {
        "Lightning address".to_string()
    } else {
        activity_method_display(destination.map(|d| &d.destination), &method)
    };
    let message_text = activity_message_text(&subtitle);
    let completed_at = movement
        .time
        .completed_at
        .unwrap_or(movement.time.updated_at);
    let completed_at_unix = completed_at.timestamp().max(0) as u64;
    let timestamp = completed_at.format("%b %-d, %-I:%M %p").to_string();
    let lightning_invoice = movement
        .lightning_invoice()
        .map(|invoice| invoice.to_string());
    let lightning_offer = movement.lightning_offer().map(|offer| offer.to_string());
    let lightning_payment_hash = movement
        .lightning_payment_hash()
        .map(|payment_hash| payment_hash.to_string());
    let lightning_payment_preimage = movement
        .metadata
        .get("payment_preimage")
        .and_then(|value| value.as_str())
        .map(ToString::to_string);
    ActivityItem {
        id: movement.id.to_string(),
        title,
        subtitle,
        display_primary_name: if inbound {
            display_counterparty.clone()
        } else {
            "You".to_string()
        },
        display_verb: "sent".to_string(),
        display_secondary_name: if inbound {
            "you".to_string()
        } else {
            display_counterparty
        },
        message_text,
        method_icon,
        method_display,
        amount_sat,
        payment_amount_sat,
        amount_display: state::format_sats(amount_sat.unsigned_abs()),
        amount_fiat_display: None,
        signed_amount_display: state::format_signed_sats(amount_sat, true),
        icon_kind: if inbound {
            ActivityIconKind::Received
        } else {
            ActivityIconKind::Sent
        },
        status: movement.status.to_string(),
        timestamp,
        completed_at_unix,
        counterparty: contact,
        ark_address,
        lightning_invoice,
        lightning_offer,
        lightning_payment_hash,
        lightning_payment_preimage,
    }
}

fn activity_amount_sat(movement: &Movement) -> i64 {
    if movement.effective_balance.to_sat() >= 0 {
        return movement.effective_balance.to_sat();
    }

    let sent_amount_sat: u64 = movement
        .sent_to
        .iter()
        .map(|destination| destination.amount.to_sat())
        .sum();
    if sent_amount_sat == 0 {
        return movement.effective_balance.to_sat();
    }

    i64::try_from(sent_amount_sat)
        .map(|amount| -amount)
        .unwrap_or_else(|_| movement.effective_balance.to_sat())
}

fn activity_payment_amount_sat(movement: &Movement, inbound: bool) -> Option<i64> {
    let destinations = if inbound {
        &movement.received_on
    } else {
        &movement.sent_to
    };
    let amount = destinations
        .iter()
        .map(|destination| destination.amount.to_sat())
        .sum::<u64>();
    if amount == 0 {
        return None;
    }
    let amount = i64::try_from(amount).ok()?;
    Some(if inbound { amount } else { -amount })
}

fn ark_address_matches(movement_address: Option<&str>, registered_address: Option<&str>) -> bool {
    match (movement_address, registered_address) {
        (Some(movement_address), Some(registered_address)) => {
            movement_address.trim() == registered_address.trim()
        }
        _ => false,
    }
}

pub(crate) fn is_user_visible_movement(movement: &Movement) -> bool {
    !is_round_refresh_movement(movement)
}

fn is_round_refresh_movement(movement: &Movement) -> bool {
    movement.subsystem.name == Subsystem::ROUND.as_name()
        && movement.subsystem.kind == RoundMovement::Refresh.to_string()
}

fn activity_method_icon(destination: Option<&BarkPaymentMethod>, inbound: bool) -> &'static str {
    match destination {
        Some(BarkPaymentMethod::Invoice(_))
        | Some(BarkPaymentMethod::Offer(_))
        | Some(BarkPaymentMethod::LightningAddress(_)) => "bolt.fill",
        Some(BarkPaymentMethod::Ark(_)) => "link",
        Some(BarkPaymentMethod::Bitcoin(_))
        | Some(BarkPaymentMethod::OutputScript(_))
        | Some(BarkPaymentMethod::Custom(_))
        | None => {
            if inbound {
                "arrow.down.left"
            } else {
                "arrow.up.right"
            }
        }
    }
}

fn ark_address_from_payment_method(method: &BarkPaymentMethod) -> Option<String> {
    match method {
        BarkPaymentMethod::Ark(address) => Some(address.to_string()),
        _ => None,
    }
}

fn activity_method_display(destination: Option<&BarkPaymentMethod>, fallback: &str) -> String {
    match destination {
        Some(method) if method.is_lightning() => "Lightning".to_string(),
        Some(BarkPaymentMethod::Ark(_)) => "Ark".to_string(),
        Some(method) if method.is_bitcoin() => "Onchain".to_string(),
        _ => {
            let lower = fallback.to_ascii_lowercase();
            if lower.contains("lightning") || lower.contains("invoice") {
                "Lightning".to_string()
            } else if lower.contains("ark") {
                "Ark".to_string()
            } else if lower.contains("bitcoin") || lower.contains("output-script") {
                "Onchain".to_string()
            } else {
                "Wallet".to_string()
            }
        }
    }
}

fn activity_message_text(subtitle: &str) -> Option<String> {
    let trimmed = subtitle.trim();
    if trimmed.is_empty()
        || trimmed.eq_ignore_ascii_case("lightning")
        || trimmed.eq_ignore_ascii_case("ark")
    {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn activity_subsystem_label(movement: &Movement) -> String {
    let raw = if movement.subsystem.name.trim().is_empty() {
        movement.subsystem.kind.as_str()
    } else {
        movement.subsystem.name.as_str()
    };
    let normalized = raw.trim().trim_start_matches("bark.").to_ascii_lowercase();
    match normalized.as_str() {
        "lightning_send" | "lightning_receive" => "Lightning".to_string(),
        "onboard" | "offboard" | "ark" => "Ark".to_string(),
        "" => "Wallet".to_string(),
        _ => raw
            .trim()
            .trim_start_matches("bark.")
            .replace('_', " ")
            .split_whitespace()
            .map(|word| {
                let mut chars = word.chars();
                match chars.next() {
                    Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                    None => String::new(),
                }
            })
            .collect::<Vec<_>>()
            .join(" "),
    }
}

fn contact_for_payment_method<'a>(
    method: &BarkPaymentMethod,
    contacts: &'a [Contact],
) -> Option<&'a Contact> {
    let value = normalize_contact_match_value(&method.value_string());
    if value.is_empty() {
        return None;
    }

    contacts
        .iter()
        .find(|contact| contact_matches_payment_value(contact, &value))
}

fn contact_matches_payment_value(contact: &Contact, payment_value: &str) -> bool {
    [
        &contact.lightning_address,
        &contact.lnurl,
        &contact.npub,
        &contact.id,
    ]
    .into_iter()
    .map(|value| normalize_contact_match_value(value))
    .filter(|value| !value.is_empty())
    .any(|value| value == payment_value || payment_value.contains(&value))
}

fn normalize_contact_match_value(value: &str) -> String {
    value
        .trim()
        .trim_start_matches("lightning:")
        .trim_start_matches("LIGHTNING:")
        .to_ascii_lowercase()
}

pub(crate) fn truncate_middle(value: &str, max: usize) -> String {
    if value.len() <= max {
        return value.to_string();
    }
    let edge = max.saturating_sub(3) / 2;
    format!("{}...{}", &value[..edge], &value[value.len() - edge..])
}

#[cfg(test)]
mod tests {
    use super::*;
    use bark::ark::Address as ArkAddress;
    use bark::movement::MovementDestination;
    use bark::movement::{MovementId, MovementStatus, MovementSubsystem};
    use bitcoin::{Amount, SignedAmount};
    use std::str::FromStr;

    const ARK_ADDR: &str = "tark1pwh9vsmezqqpharv69q4z8m6x364d5m5prnmcalcalq9pdmzw0y7mpveck4pcfhezqypczkrrj3lkx5ue4qrf4jc7ztpt9htdttmh2judhqnu7aue8p0y9mq47jn9z";
    const LIGHTNING_OFFER: &str = "lno1pqpzwyq2qe3k7enxv4j3pjgrrwzv24nmzfjypx2a8m264ws9vht3uxp5vpypnluuzl67n4waq78syn2tdngnvypje2da9t4emyq25n29m84dszkfggehf3z35uj56pmxqgp5vfme44926w23gc282xn3pp0j7y8pc7je8e8qxrhmtwrjrnj4kzcqyqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqjnrlnqdqf52q7jwgcnxgnuseav37nvs0zn06dyfs79hk7uk8lrxuqzqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqq";

    #[test]
    fn activity_helpers_return_render_ready_labels() {
        let offer = BarkPaymentMethod::from_type_value("offer", LIGHTNING_OFFER).unwrap();
        let ark = BarkPaymentMethod::from_type_value("ark", ARK_ADDR).unwrap();

        assert_eq!(activity_method_icon(Some(&offer), false), "bolt.fill");
        assert_eq!(activity_method_icon(Some(&ark), false), "link");
        assert_eq!(activity_method_icon(None, true), "arrow.down.left");
        assert_eq!(activity_method_icon(None, false), "arrow.up.right");

        assert_eq!(activity_message_text(""), None);
        assert_eq!(activity_message_text("ark"), None);
        assert_eq!(
            activity_message_text("12 sats fee").as_deref(),
            Some("12 sats fee")
        );
    }

    #[test]
    fn normalizes_activity_subsystem_labels() {
        assert_eq!(truncate_middle("abcdef", 12), "abcdef");
        assert_eq!(
            truncate_middle("abcdefghijklmnopqrstuvwxyz", 11),
            "abcd...wxyz"
        );
    }

    #[test]
    fn hides_internal_round_refresh_movements() {
        let refresh = Movement::new(
            MovementId(1),
            MovementStatus::Successful,
            &MovementSubsystem {
                name: Subsystem::ROUND.as_name().to_string(),
                kind: RoundMovement::Refresh.to_string(),
            },
            chrono::Local::now(),
        );
        assert!(!is_user_visible_movement(&refresh));

        let receive = Movement::new(
            MovementId(2),
            MovementStatus::Successful,
            &MovementSubsystem {
                name: "bark.arkoor".to_string(),
                kind: "receive".to_string(),
            },
            chrono::Local::now(),
        );
        assert!(is_user_visible_movement(&receive));
    }

    #[test]
    fn outbound_activity_amount_excludes_fee() {
        let mut movement = Movement::new(
            MovementId(4),
            MovementStatus::Successful,
            &MovementSubsystem {
                name: "bark.lightning".to_string(),
                kind: "send".to_string(),
            },
            chrono::Local::now(),
        );
        movement.effective_balance = SignedAmount::from_sat(-75);
        movement.offchain_fee = Amount::from_sat(20);
        movement.sent_to = vec![MovementDestination::custom(
            "lnbc1invoice".to_string(),
            Amount::from_sat(55),
        )];

        let item = activity_from_movement(movement, &[], None, None);

        assert_eq!(item.amount_sat, -55);
        assert_eq!(item.amount_display, "55 sats");
        assert_eq!(item.signed_amount_display, "-55 sats");
        assert_eq!(item.subtitle, "20 sats fee");
        assert_eq!(item.message_text.as_deref(), Some("20 sats fee"));
    }

    #[test]
    fn labels_registered_lnurl_ark_receives_as_lightning_address() {
        let ark_address = ArkAddress::from_str(ARK_ADDR).unwrap();
        let mut movement = Movement::new(
            MovementId(3),
            MovementStatus::Successful,
            &MovementSubsystem {
                name: "bark.arkoor".to_string(),
                kind: "receive".to_string(),
            },
            chrono::Local::now(),
        );
        movement.effective_balance = SignedAmount::from_sat(1_000);
        movement.received_on = vec![MovementDestination::ark(
            ark_address,
            Amount::from_sat(1_000),
        )];

        let item = activity_from_movement(
            movement,
            &[],
            Some("alice@signet.zaps.rebelwallet.app"),
            Some(ARK_ADDR),
        );

        assert_eq!(
            item.title,
            "Received Lightning address alice@signet...elwallet.app"
        );
        assert_eq!(item.method_display, "Lightning address");
        assert_eq!(item.method_icon, "bolt.fill");
        assert_eq!(item.ark_address.as_deref(), Some(ARK_ADDR));
    }
}
