use anyhow::{anyhow, bail, Context};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug)]
pub(crate) struct RegisterQuote {
    pub(crate) name: String,
    pub(crate) ark_address: String,
    pub(crate) message: String,
}

#[derive(Clone, Debug)]
pub(crate) struct RegisterResult {
    pub(crate) id: i32,
    pub(crate) name: String,
    pub(crate) lightning_address: String,
    pub(crate) ark_address: String,
    pub(crate) fee_sats: u64,
    pub(crate) invoice: String,
    pub(crate) state: String,
    pub(crate) active: bool,
}

#[derive(Clone, Debug)]
pub(crate) struct RegisterStatus {
    pub(crate) id: i32,
    pub(crate) name: String,
    pub(crate) lightning_address: String,
    pub(crate) ark_address: String,
    pub(crate) fee_sats: u64,
    pub(crate) active: bool,
    pub(crate) invoice: String,
    pub(crate) state: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RegisterQuoteResponse {
    name: String,
    ark_address: String,
    message: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RegisterRequest<'a> {
    name: &'a str,
    ark_address: &'a str,
    signature: &'a str,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RegisterResponse {
    status: String,
    custom_address: String,
    invoice: CustomAddressInvoiceResponse,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RegisterStatusResponse {
    status: String,
    custom_address: String,
    invoice: CustomAddressInvoiceResponse,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CustomAddressInvoiceResponse {
    id: i32,
    name: String,
    ark_address: String,
    fee_sats: u64,
    invoice: String,
    state: String,
    active: bool,
}

#[derive(Deserialize)]
struct ErrorResponse {
    status: Option<String>,
    reason: Option<String>,
}

pub(crate) async fn quote_registration(
    client: &reqwest::Client,
    domain: &str,
    name: &str,
    ark_address: &str,
) -> anyhow::Result<RegisterQuote> {
    let mut url = registration_url(domain)?;
    url.path_segments_mut()
        .map_err(|_| anyhow!("invalid custom address domain"))?
        .extend(["custom-addresses", "auth-message"]);
    url.query_pairs_mut()
        .append_pair("name", name)
        .append_pair("arkAddress", ark_address);

    let response = client
        .get(url)
        .send()
        .await
        .context("failed to request custom address quote")?;
    let quote: RegisterQuoteResponse = parse_json_response(response).await?;
    Ok(RegisterQuote {
        name: quote.name,
        ark_address: quote.ark_address,
        message: quote.message,
    })
}

pub(crate) async fn register_address(
    client: &reqwest::Client,
    domain: &str,
    name: &str,
    ark_address: &str,
    signature: &str,
) -> anyhow::Result<RegisterResult> {
    let mut url = registration_url(domain)?;
    url.path_segments_mut()
        .map_err(|_| anyhow!("invalid custom address domain"))?
        .extend(["custom-addresses"]);

    let response = client
        .post(url)
        .json(&RegisterRequest {
            name,
            ark_address,
            signature,
        })
        .send()
        .await
        .context("failed to submit custom address registration")?;
    let response: RegisterResponse = parse_json_response(response).await?;
    if response.status.eq_ignore_ascii_case("ERROR") {
        bail!("custom address server returned an error");
    }
    Ok(RegisterResult {
        id: response.invoice.id,
        name: response.invoice.name,
        lightning_address: response.custom_address,
        ark_address: response.invoice.ark_address,
        fee_sats: response.invoice.fee_sats,
        invoice: response.invoice.invoice,
        state: response.invoice.state,
        active: response.invoice.active,
    })
}

pub(crate) async fn verify_registration(
    client: &reqwest::Client,
    domain: &str,
    purchase_id: &str,
) -> anyhow::Result<RegisterStatus> {
    let mut url = registration_url(domain)?;
    url.path_segments_mut()
        .map_err(|_| anyhow!("invalid custom address domain"))?
        .extend(["custom-addresses", purchase_id]);

    let response = client
        .get(url)
        .send()
        .await
        .context("failed to check custom address registration")?;
    let status: RegisterStatusResponse = parse_json_response(response).await?;
    if status.status.eq_ignore_ascii_case("ERROR") {
        bail!("custom address registration was not found");
    }
    let invoice = status.invoice;
    Ok(RegisterStatus {
        id: invoice.id,
        name: invoice.name,
        lightning_address: status.custom_address,
        ark_address: invoice.ark_address,
        fee_sats: invoice.fee_sats,
        active: invoice.active,
        invoice: invoice.invoice,
        state: invoice.state,
    })
}

fn registration_url(domain: &str) -> anyhow::Result<reqwest::Url> {
    let domain = domain.trim();
    if domain.is_empty() {
        bail!("custom address domain is empty");
    }
    let base = if domain.starts_with("https://") || domain.starts_with("http://") {
        domain.to_string()
    } else {
        format!("https://{domain}")
    };
    reqwest::Url::parse(&base).context("invalid custom address domain")
}

async fn parse_json_response<T: for<'de> Deserialize<'de>>(
    response: reqwest::Response,
) -> anyhow::Result<T> {
    let status = response.status();
    let text = response.text().await.context("failed to read response")?;
    if !status.is_success() {
        if let Ok(error) = serde_json::from_str::<ErrorResponse>(&text) {
            if let Some(reason) = error.reason.filter(|reason| !reason.trim().is_empty()) {
                bail!("{reason}");
            }
        }
        bail!("server returned {status}");
    }

    if let Ok(error) = serde_json::from_str::<ErrorResponse>(&text) {
        if error.status.as_deref() == Some("ERROR") {
            bail!(
                "{}",
                error
                    .reason
                    .filter(|reason| !reason.trim().is_empty())
                    .unwrap_or_else(|| "custom address server returned an error".to_string())
            );
        }
    }

    serde_json::from_str(&text).context("failed to parse custom address response")
}

pub(crate) fn amount_msats_to_sat(amount_msats: u64) -> anyhow::Result<u64> {
    if !amount_msats.is_multiple_of(1_000) {
        bail!("custom address fee must be denominated in whole sats");
    }
    Ok(amount_msats / 1_000)
}

pub(crate) fn validate_custom_address_name(name: &str) -> Option<String> {
    let name = name.trim();
    if name.is_empty() {
        return Some("Enter a custom name.".to_string());
    }
    if name.len() < 3 || name.len() > 32 {
        return Some("Name must be 3 to 32 characters.".to_string());
    }
    if !name
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
    {
        return Some("Use letters, numbers, dashes, or underscores.".to_string());
    }
    if !name
        .as_bytes()
        .first()
        .is_some_and(|b| b.is_ascii_alphanumeric())
        || !name
            .as_bytes()
            .last()
            .is_some_and(|b| b.is_ascii_alphanumeric())
    {
        return Some("Name must start and end with a letter or number.".to_string());
    }
    None
}

#[cfg(test)]
mod tests {
    use super::{amount_msats_to_sat, validate_custom_address_name};

    #[test]
    fn validates_custom_address_names() {
        assert_eq!(validate_custom_address_name("alice"), None);
        assert_eq!(validate_custom_address_name("alice_sats-12"), None);
        assert_eq!(
            validate_custom_address_name(""),
            Some("Enter a custom name.".to_string())
        );
        assert_eq!(
            validate_custom_address_name("al"),
            Some("Name must be 3 to 32 characters.".to_string())
        );
        assert_eq!(
            validate_custom_address_name("alice!"),
            Some("Use letters, numbers, dashes, or underscores.".to_string())
        );
        assert_eq!(
            validate_custom_address_name("-alice"),
            Some("Name must start and end with a letter or number.".to_string())
        );
    }

    #[test]
    fn converts_registration_fee_msats() {
        assert_eq!(amount_msats_to_sat(10_000_000).unwrap(), 10_000);
        assert!(amount_msats_to_sat(1_001).is_err());
    }
}
