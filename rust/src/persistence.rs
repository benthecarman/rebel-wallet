use nostr_sdk::prelude::Url;
use serde::{Deserialize, Serialize};

use crate::{NostrState, PriceCurrency, WalletState, SIGNET_ESPLORA, SIGNET_SERVER};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct PersistedAppData {
    pub(crate) nostr: NostrState,
    pub(crate) receive_amount_sat: u64,
    pub(crate) receive_memo: String,
    #[serde(default = "default_server_config")]
    pub(crate) servers: ServerConfig,
    #[serde(default = "default_price_currency")]
    pub(crate) price_currency: PersistedPriceCurrency,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(transparent)]
pub(crate) struct PersistedPriceCurrency {
    #[serde(with = "price_currency_serde")]
    pub(crate) currency: PriceCurrency,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct ServerConfig {
    pub(crate) server_address: String,
    pub(crate) esplora_address: String,
}

impl ServerConfig {
    pub(crate) fn from_wallet(wallet: &WalletState) -> Self {
        Self {
            server_address: wallet.server_address.clone(),
            esplora_address: wallet.esplora_address.clone(),
        }
    }
}

pub(crate) fn validate_server_url(label: &str, raw: &str) -> Result<(), String> {
    let parsed = Url::parse(raw).map_err(|_| format!("{label} must be a valid URL."))?;
    match parsed.scheme() {
        "http" | "https" => Ok(()),
        _ => Err(format!("{label} must use http or https.")),
    }
}

fn default_server_config() -> ServerConfig {
    ServerConfig {
        server_address: SIGNET_SERVER.to_string(),
        esplora_address: SIGNET_ESPLORA.to_string(),
    }
}

fn default_price_currency() -> PersistedPriceCurrency {
    PersistedPriceCurrency {
        currency: PriceCurrency::BTC,
    }
}

mod price_currency_serde {
    use serde::{Deserialize, Deserializer, Serializer};

    use crate::PriceCurrency;

    pub(super) fn serialize<S>(currency: &PriceCurrency, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(currency.code())
    }

    pub(super) fn deserialize<'de, D>(deserializer: D) -> Result<PriceCurrency, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        match value.to_ascii_uppercase().as_str() {
            "BTC" => Ok(PriceCurrency::BTC),
            "USD" => Ok(PriceCurrency::USD),
            "EUR" => Ok(PriceCurrency::EUR),
            "GBP" => Ok(PriceCurrency::GBP),
            _ => Ok(PriceCurrency::BTC),
        }
    }
}
