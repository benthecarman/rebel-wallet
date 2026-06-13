use bitcoin::{Amount, Denomination};

use super::types::{
    AppState, Contact, CurrencyOption, NetworkOption, PriceCurrency, ReceiveMethod, ReceiveState,
    SendDestinationKind, SetupState, WalletNetwork,
};

pub(crate) fn arkzap_lightning_address(ark_address: &str) -> String {
    let ark_address = ark_address.trim();
    let domain = if ark_address.starts_with("tark") {
        "signet.arkzap.me"
    } else {
        "arkzap.me"
    };
    format!("{ark_address}@{domain}")
}

pub(super) fn supported_price_currencies() -> Vec<CurrencyOption> {
    [
        PriceCurrency::BTC,
        PriceCurrency::USD,
        PriceCurrency::EUR,
        PriceCurrency::GBP,
    ]
    .into_iter()
    .map(|currency| CurrencyOption {
        code: currency.code().to_string(),
        name: currency.display_name().to_string(),
        currency,
    })
    .collect()
}

pub(super) fn supported_networks() -> Vec<NetworkOption> {
    [WalletNetwork::Signet, WalletNetwork::Mainnet]
        .into_iter()
        .map(|network| NetworkOption {
            name: network.display_name().to_string(),
            caption: network.caption().to_string(),
            network,
        })
        .collect()
}

pub(super) fn should_show_launch_splash(state: &AppState) -> bool {
    if state.rev == 0 {
        return true;
    }
    if state.busy.bootstrapping || state.busy.opening_wallet {
        return true;
    }
    state.setup == SetupState::Ready
        && state.busy.syncing_wallet
        && state.wallet.last_sync.is_none()
}

pub(crate) fn format_sats(amount: u64) -> String {
    format!("{} sats", grouped_digits(amount))
}

pub(super) fn receive_request(receive: &ReceiveState) -> Option<String> {
    match receive.method {
        ReceiveMethod::Lightning => receive.lightning_invoice.clone(),
        ReceiveMethod::Ark => {
            let address = receive.ark_address.as_ref()?;
            if receive.amount_sat == 0 {
                Some(address.clone())
            } else {
                let amount =
                    Amount::from_sat(receive.amount_sat).to_string_in(Denomination::Bitcoin);
                Some(format!("bitcoin:?amount={amount}&ark={address}"))
            }
        }
    }
}

pub(super) fn format_fiat_sats(
    amount_sat: u64,
    btc_price: Option<f64>,
    currency: &PriceCurrency,
) -> Option<String> {
    let price = btc_price?;
    let value = amount_sat as f64 / 100_000_000.0 * price;
    Some(format_fiat(value, currency))
}

pub(super) fn format_non_btc_fiat_sats(
    amount_sat: u64,
    btc_price: Option<f64>,
    currency: &PriceCurrency,
) -> Option<String> {
    if *currency == PriceCurrency::BTC {
        return None;
    }
    format_fiat_sats(amount_sat, btc_price, currency)
}

fn format_fiat(value: f64, currency: &PriceCurrency) -> String {
    let max_fraction_digits = currency.max_fractional_digits();
    let number = if value == 0.0 {
        "0".to_string()
    } else {
        format!("{value:.max_fraction_digits$}")
    };
    format!(
        "{}{}{} {}",
        currency.approximate(),
        currency.symbol(),
        number,
        currency.code()
    )
}

pub(crate) fn format_signed_sats(amount: i64, signed: bool) -> String {
    let magnitude = amount.unsigned_abs();
    let prefix = if amount < 0 {
        "-"
    } else if signed && amount > 0 {
        "+"
    } else {
        ""
    };
    format!("{prefix}{} sats", grouped_digits(magnitude))
}

pub(crate) fn send_destination_kind(destination: &str) -> SendDestinationKind {
    let lower = destination.trim().to_ascii_lowercase();
    if lower.is_empty() {
        SendDestinationKind::Unknown
    } else if lower.starts_with("lightning:")
        || lower.starts_with("ln")
        || is_valid_lightning_address(destination)
    {
        SendDestinationKind::Lightning
    } else {
        SendDestinationKind::Ark
    }
}

pub(super) fn send_search_results(
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
            .then_with(|| b.last_used.cmp(&a.last_used))
            .then_with(|| normalize_search(&a.name).cmp(&normalize_search(&b.name)))
            .then_with(|| a.npub.cmp(&b.npub))
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

fn contact_has_lightning_address(contact: &Contact) -> bool {
    is_valid_lightning_address(&contact.lightning_address)
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

pub(super) fn is_sendable_search_query(query: &str) -> bool {
    let trimmed = query.trim();
    let lower = trimmed.to_ascii_lowercase();
    trimmed.len() >= 6
        && (lower.starts_with("ark")
            || lower.starts_with("lightning:")
            || lower.starts_with("lnbc")
            || lower.starts_with("lntb")
            || lower.starts_with("lnurl")
            || trimmed.contains('@')
            || lower.starts_with("http://")
            || lower.starts_with("https://"))
}

fn normalize_search(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}

pub(super) fn send_error_text(
    destination_kind: SendDestinationKind,
    amount_sat: u64,
    total_cost_sat: Option<u64>,
    balance_sat: u64,
) -> Option<String> {
    if total_cost_sat.unwrap_or(amount_sat) > balance_sat {
        return Some("Insufficient balance for this send.".to_string());
    }
    if destination_kind == SendDestinationKind::Ark && amount_sat == 0 {
        return Some("Enter an amount before sending to an Ark address.".to_string());
    }
    None
}

fn grouped_digits(amount: u64) -> String {
    let digits = amount.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    let first_group_len = digits.len() % 3;

    for (idx, ch) in digits.chars().enumerate() {
        if idx > 0
            && (idx == first_group_len
                || (idx > first_group_len && (idx - first_group_len) % 3 == 0))
        {
            out.push(',');
        }
        out.push(ch);
    }

    out
}
