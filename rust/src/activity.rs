use bark::movement::{Movement, PaymentMethod as BarkPaymentMethod};
use bark::subsystem::{RoundMovement, Subsystem};

use crate::{state, ActivityIconKind, ActivityItem, Contact};

pub(crate) fn activity_from_movement(movement: Movement, contacts: &[Contact]) -> ActivityItem {
    let amount_sat = movement.effective_balance.to_sat();
    let inbound = amount_sat >= 0;
    let destination = if inbound {
        movement.received_on.first()
    } else {
        movement.sent_to.first()
    };
    let contact = destination
        .and_then(|d| contact_for_payment_method(&d.destination, contacts))
        .cloned();
    let method = destination
        .map(|d| {
            format!(
                "{} {}",
                d.destination.type_str(),
                truncate_middle(&d.destination.value_string(), 28)
            )
        })
        .unwrap_or_else(|| activity_subsystem_label(&movement));
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
    let method_icon = activity_method_icon(&method, inbound).to_string();
    let method_display = activity_method_display(destination.map(|d| &d.destination), &method);
    let message_text = activity_message_text(&subtitle);
    let timestamp = movement
        .time
        .completed_at
        .unwrap_or(movement.time.updated_at)
        .format("%b %-d, %-I:%M %p")
        .to_string();
    let lightning_invoice = movement
        .lightning_invoice()
        .map(|invoice| invoice.to_string());
    let lightning_payment_hash = movement
        .lightning_payment_hash()
        .map(|payment_hash| payment_hash.to_string());
    let lightning_payment_preimage = movement
        .metadata
        .get("payment_preimage")
        .and_then(|value| value.as_str())
        .map(ToString::to_string);
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
        amount_display: state::format_sats(amount_sat.unsigned_abs()),
        signed_amount_display: state::format_signed_sats(amount_sat, true),
        icon_kind: if inbound {
            ActivityIconKind::Received
        } else {
            ActivityIconKind::Sent
        },
        status: movement.status.to_string(),
        timestamp,
        counterparty: contact,
        ark_address,
        lightning_invoice,
        lightning_payment_hash,
        lightning_payment_preimage,
    }
}

pub(crate) fn is_user_visible_movement(movement: &Movement) -> bool {
    !is_round_refresh_movement(movement)
}

fn is_round_refresh_movement(movement: &Movement) -> bool {
    movement.subsystem.name == Subsystem::ROUND.as_name()
        && movement.subsystem.kind == RoundMovement::Refresh.to_string()
}

fn activity_method_icon(method: &str, inbound: bool) -> &'static str {
    let lower = method.to_ascii_lowercase();
    if lower.contains("lightning") || lower.contains("invoice") {
        "bolt.fill"
    } else if lower.contains("ark") {
        "link"
    } else if inbound {
        "arrow.down.left"
    } else {
        "arrow.up.right"
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
    use bark::movement::{MovementId, MovementStatus, MovementSubsystem};

    #[test]
    fn activity_helpers_return_render_ready_labels() {
        assert_eq!(
            activity_method_icon("Lightning lnbc1...", true),
            "bolt.fill"
        );
        assert_eq!(activity_method_icon("Ark address", false), "link");
        assert_eq!(activity_method_icon("Wallet", true), "arrow.down.left");
        assert_eq!(activity_method_icon("Wallet", false), "arrow.up.right");

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
}
