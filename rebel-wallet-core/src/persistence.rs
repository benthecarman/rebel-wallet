use serde::{Deserialize, Serialize};

use crate::{NostrState, PriceCurrency, WalletNetwork, WalletState};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct PersistedAppData {
    pub(crate) nostr: NostrState,
    pub(crate) receive_amount_sat: u64,
    pub(crate) receive_memo: String,
    #[serde(default)]
    pub(crate) network: WalletNetwork,
    #[serde(default = "default_server_config")]
    pub(crate) servers: ServerConfig,
    #[serde(default = "default_price_currency")]
    pub(crate) price_currency: PersistedPriceCurrency,
    #[serde(default, skip_serializing)]
    pub(crate) lightning_address_ark_address: Option<String>,
    #[serde(default)]
    pub(crate) custom_lightning_address: Option<String>,
    #[serde(default)]
    pub(crate) custom_lightning_address_name: String,
    #[serde(default)]
    pub(crate) pending_custom_lightning_address: Option<PendingCustomLightningAddress>,
    #[serde(default)]
    pub(crate) payment_annotations: Vec<PaymentAnnotation>,
    #[serde(default)]
    pub(crate) zap_receipts: Vec<ZapReceiptRecord>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct PendingCustomLightningAddress {
    pub(crate) name: String,
    pub(crate) lightning_address: String,
    pub(crate) ark_address: String,
    pub(crate) invoice: String,
    #[serde(alias = "payment_hash")]
    pub(crate) purchase_id: String,
    pub(crate) amount_msats: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct PaymentAnnotation {
    pub(crate) contact_id: Option<String>,
    pub(crate) destination: String,
    pub(crate) invoice: Option<String>,
    pub(crate) payment_hash: Option<String>,
    pub(crate) amount_sat: i64,
    pub(crate) outbound: bool,
    pub(crate) zap: bool,
    pub(crate) created_at: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ZapReceiptRecord {
    pub(crate) event_id: String,
    pub(crate) sender_pubkey: String,
    pub(crate) recipient_pubkey: String,
    pub(crate) invoice: Option<String>,
    pub(crate) payment_hash: Option<String>,
    pub(crate) amount_msat: Option<u64>,
    #[serde(default)]
    pub(crate) lnurl: Option<String>,
    pub(crate) comment: Option<String>,
    pub(crate) created_at: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(transparent)]
pub(crate) struct PersistedPriceCurrency {
    #[serde(with = "price_currency_serde")]
    pub(crate) currency: PriceCurrency,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ServerConfig {
    #[serde(default)]
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
    ServerConfig::for_network(WalletNetwork::default())
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

        assert_eq!(data.network, WalletNetwork::Mainnet);
        assert_eq!(data.lightning_address_ark_address, None);
        assert_eq!(data.custom_lightning_address, None);
        assert_eq!(data.custom_lightning_address_name, "");
        assert!(data.pending_custom_lightning_address.is_none());
        assert!(data.payment_annotations.is_empty());
        assert!(data.zap_receipts.is_empty());
        assert!(!data.nostr.deleted);
    }

    #[test]
    fn app_data_defaults_network_and_servers_to_mainnet() {
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
            "price_currency": "BTC"
        }"#;

        let data: PersistedAppData = serde_json::from_str(raw).unwrap();
        let mainnet = WalletNetwork::default();

        assert_eq!(mainnet, WalletNetwork::Mainnet);
        assert_eq!(data.network, mainnet);
        assert_eq!(data.servers, ServerConfig::for_network(mainnet));
    }
}
