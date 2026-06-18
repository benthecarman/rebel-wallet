use std::str::FromStr;

use bark::ark::Address as ArkAddress;
use bitcoin::Address as BitcoinAddress;

use super::{format_non_btc_fiat_sats, format_sats, normalize_search, AppState};
use crate::payments::{
    is_lnurl_pay_destination, is_valid_lightning_address, lightning_offer_amount_sat,
    send_destination_kind,
};
use crate::{Contact, SendDestinationKind, SendPhase};

pub(super) fn refresh_send_derived(state: &mut AppState) {
    state.send.amount_display = format_sats(state.send.amount_sat);
    state.send.fee_estimate_display = state.send.fee_estimate_sat.map(format_sats);
    state.send.total_cost_display = state.send.total_cost_sat.map(format_sats);
    state.send.fee_estimate_fiat_display = state.send.fee_estimate_sat.and_then(|amount| {
        format_non_btc_fiat_sats(amount, state.wallet.btc_price, &state.wallet.price_currency)
    });
    state.send.total_cost_fiat_display = state.send.total_cost_sat.and_then(|amount| {
        format_non_btc_fiat_sats(amount, state.wallet.btc_price, &state.wallet.price_currency)
    });
    state.send.search_results = send_search_results(
        &state.send.search_query,
        &state.nostr.contacts,
        &state.send.global_search_results,
        state.nostr.npub.as_deref(),
    );
    state.send.can_continue_search = is_sendable_search_query(&state.send.search_query);
    if state.send.destination.trim().is_empty() && state.send.phase == SendPhase::Editing {
        state.send.phase = SendPhase::Drafting;
    }
    state.send.destination_kind = send_destination_kind(&state.send.destination);
    state.send.error_text = send_error_text(
        &state.send.destination,
        state.send.destination_kind.clone(),
        state.send.amount_sat,
        state.send.total_cost_sat,
        state.wallet.balance_sat,
    );
    state.send.can_submit = !state.send.destination.trim().is_empty()
        && state.send.phase != SendPhase::Sending
        && !state.send.estimating_fee
        && state.send.error_text.is_none()
        && match state.send.destination_kind {
            SendDestinationKind::Lightning => true,
            SendDestinationKind::Ark | SendDestinationKind::OnChain => state.send.amount_sat > 0,
            SendDestinationKind::Unknown => false,
        };
    if !state.send.zap_available {
        state.send.zap_enabled = false;
    }
}

fn send_search_results(
    query: &str,
    contacts: &[Contact],
    global_results: &[Contact],
    own_npub: Option<&str>,
) -> Vec<Contact> {
    let needle = normalize_search(query);
    let own_npub = own_npub.map(normalize_search);
    let mut contacts = contacts
        .iter()
        .cloned()
        .chain(global_results.iter().cloned())
        .map(|mut contact| {
            contact.name = contact.name.trim().to_string();
            contact
        })
        .fold(Vec::<Contact>::new(), |mut out, contact| {
            if !out.iter().any(|c| c.npub == contact.npub) {
                out.push(contact);
            }
            out
        });
    contacts.sort_by(|a, b| {
        contact_has_lightning_address(b)
            .cmp(&contact_has_lightning_address(a))
            .then_with(|| normalize_search(&a.name).cmp(&normalize_search(&b.name)))
            .then_with(|| normalize_search(&a.npub).cmp(&normalize_search(&b.npub)))
            .then_with(|| a.id.cmp(&b.id))
    });

    contacts
        .into_iter()
        .filter(|contact| {
            if let Some(own_npub) = &own_npub {
                if normalize_search(&contact.npub) == *own_npub {
                    return false;
                }
            }
            contact_has_lightning_address(contact)
        })
        .filter(|contact| {
            needle.is_empty()
                || normalize_search(&contact.name).contains(&needle)
                || normalize_search(&contact.npub).contains(&needle)
                || normalize_search(&contact.lightning_address).contains(&needle)
                || normalize_search(&contact.lnurl).contains(&needle)
        })
        .collect()
}

pub(crate) fn sort_contacts_by_name_npub(contacts: &mut [Contact]) {
    contacts.sort_by(|a, b| {
        normalize_search(&a.name)
            .cmp(&normalize_search(&b.name))
            .then_with(|| normalize_search(&a.npub).cmp(&normalize_search(&b.npub)))
            .then_with(|| a.id.cmp(&b.id))
    });
}

fn contact_has_lightning_address(contact: &Contact) -> bool {
    is_valid_lightning_address(&contact.lightning_address)
}

fn is_sendable_search_query(query: &str) -> bool {
    let trimmed = query.trim();
    let lower = trimmed.to_ascii_lowercase();
    trimmed.len() >= 6
        && (ArkAddress::from_str(trimmed).is_ok()
            || BitcoinAddress::from_str(trimmed).is_ok()
            || lower.starts_with("lightning:")
            || lower.starts_with("lnbc")
            || lower.starts_with("lntb")
            || lower.starts_with("lno")
            || lower.starts_with("lnurl")
            || trimmed.contains('@')
            || lower.starts_with("http://")
            || lower.starts_with("https://"))
}

fn send_error_text(
    destination: &str,
    destination_kind: SendDestinationKind,
    amount_sat: u64,
    total_cost_sat: Option<u64>,
    balance_sat: u64,
) -> Option<String> {
    let effective_amount_sat =
        if amount_sat == 0 && destination_kind == SendDestinationKind::Lightning {
            lightning_offer_amount_sat(destination).unwrap_or(0)
        } else {
            amount_sat
        };
    if total_cost_sat.unwrap_or(effective_amount_sat) > balance_sat {
        return Some("Insufficient balance for this send.".to_string());
    }
    if send_requires_amount(destination, destination_kind.clone()) && amount_sat == 0 {
        return Some(format!(
            "Enter an amount before sending to {}.",
            match destination_kind {
                SendDestinationKind::OnChain => "an on-chain address",
                SendDestinationKind::Lightning => "this Lightning destination",
                _ => "an Ark address",
            }
        ));
    }
    None
}

fn send_requires_amount(destination: &str, destination_kind: SendDestinationKind) -> bool {
    match destination_kind {
        SendDestinationKind::Ark | SendDestinationKind::OnChain => true,
        SendDestinationKind::Lightning => {
            is_lnurl_pay_destination(destination) || is_amountless_offer(destination)
        }
        SendDestinationKind::Unknown => false,
    }
}

fn is_amountless_offer(destination: &str) -> bool {
    lightning_offer_amount_sat(destination).is_some_and(|amount| amount == 0)
}
