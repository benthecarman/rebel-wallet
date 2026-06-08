use serde::{Deserialize, Serialize};

use crate::{NostrState, PriceCurrency, WalletNetwork, WalletState};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct PersistedAppData {
    pub(crate) nostr: NostrState,
    pub(crate) receive_amount_sat: u64,
    pub(crate) receive_memo: String,
    #[serde(default = "default_network")]
    pub(crate) network: WalletNetwork,
    #[serde(default = "default_server_config")]
    pub(crate) servers: ServerConfig,
    #[serde(default = "default_price_currency")]
    pub(crate) price_currency: PersistedPriceCurrency,
    #[serde(default, skip_serializing)]
    pub(crate) lightning_address_ark_address: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(transparent)]
pub(crate) struct PersistedPriceCurrency {
    #[serde(with = "price_currency_serde")]
    pub(crate) currency: PriceCurrency,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct ServerConfig {
    #[serde(default = "default_network")]
    pub(crate) network: WalletNetwork,
    pub(crate) server_address: String,
    #[serde(skip)]
    pub(crate) server_access_token: Option<String>,
    pub(crate) esplora_address: String,
}

impl ServerConfig {
    pub(crate) fn for_network(network: WalletNetwork) -> Self {
        Self {
            network,
            server_address: network.server_address().to_string(),
            server_access_token: network.server_access_token().map(str::to_string),
            esplora_address: network.esplora_address().to_string(),
        }
    }

    pub(crate) fn from_wallet(wallet: &WalletState) -> Self {
        Self {
            network: wallet.network,
            server_address: wallet.server_address.clone(),
            server_access_token: wallet.network.server_access_token().map(str::to_string),
            esplora_address: wallet.esplora_address.clone(),
        }
    }
}

fn default_server_config() -> ServerConfig {
    ServerConfig::for_network(WalletNetwork::Signet)
}

fn default_network() -> WalletNetwork {
    WalletNetwork::Signet
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_data_defaults_missing_arkzap_addresses() {
        let raw = r#"{
            "nostr": {
                "npub": null,
                "name": "Rebel",
                "about": "",
                "picture": "",
                "lud16": "",
                "nip05": "",
                "contacts": []
            },
            "receive_amount_sat": 0,
            "receive_memo": "",
            "servers": {
                "server_address": "https://ark.example.com",
                "esplora_address": "https://esplora.example.com"
            },
            "price_currency": "BTC"
        }"#;

        let data: PersistedAppData = serde_json::from_str(raw).unwrap();

        assert_eq!(data.network, WalletNetwork::Signet);
        assert_eq!(data.lightning_address_ark_address, None);
        assert!(!data.nostr.deleted);
    }
}
