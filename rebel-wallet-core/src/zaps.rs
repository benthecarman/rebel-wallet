use std::collections::{HashMap, HashSet};
use std::str::FromStr;
use std::time::Duration;

use anyhow::{anyhow, bail, Context};
use lightning_invoice::Bolt11Invoice;
use nostr_sdk::prelude::{
    Alphabet, Event, EventBuilder, Filter, FinalizeEvent, JsonUtil, Keys, Kind,
    PublicKey as NostrPublicKey, RelayUrl, SingleLetterTag, ZapRequestData,
};
use serde::Deserialize;

use crate::nostr_support::{nostr_client, public_key_from_npub_or_hex, NOSTR_RELAYS};
use crate::payments::{lnurl_pay_url, msats_to_display_sats};
use crate::persistence::ZapReceiptRecord;

#[derive(Clone, Debug)]
pub(crate) struct ZapEndpoint {
    pub(crate) callback: String,
    pub(crate) min_sendable: u64,
    pub(crate) max_sendable: u64,
    pub(crate) lnurl: String,
}

#[derive(Debug, Deserialize)]
struct LnurlZapParams {
    tag: Option<String>,
    callback: String,
    #[serde(rename = "minSendable")]
    min_sendable: u64,
    #[serde(rename = "maxSendable")]
    max_sendable: u64,
    #[serde(rename = "allowsNostr")]
    allows_nostr: Option<bool>,
    #[serde(rename = "nostrPubkey")]
    nostr_pubkey: Option<String>,
}

#[derive(Debug, Deserialize)]
struct LnurlZapInvoice {
    pr: Option<String>,
    status: Option<String>,
    reason: Option<String>,
}

pub(crate) async fn request_zap_invoice(
    destination: &str,
    recipient_pubkey: NostrPublicKey,
    amount_sat: u64,
    comment: &str,
    keys: &Keys,
) -> anyhow::Result<String> {
    if amount_sat == 0 {
        bail!("Enter an amount before sending a zap.");
    }
    let endpoint = fetch_zap_endpoint(destination).await?;
    let amount_msat = amount_sat
        .checked_mul(1_000)
        .ok_or_else(|| anyhow!("zap amount is too large"))?;
    if amount_msat < endpoint.min_sendable || amount_msat > endpoint.max_sendable {
        bail!(
            "Zap amount must be between {} and {} sats.",
            msats_to_display_sats(endpoint.min_sendable),
            msats_to_display_sats(endpoint.max_sendable)
        );
    }

    let relays = zap_relays()?;
    let data = ZapRequestData::new(recipient_pubkey, relays)
        .message(comment.trim())
        .amount(amount_msat)
        .lnurl(endpoint.lnurl.clone());
    let event = EventBuilder::public_zap_request(data).finalize(keys)?;
    let mut callback =
        reqwest::Url::parse(&endpoint.callback).context("LNURL callback is not a valid URL")?;
    callback
        .query_pairs_mut()
        .append_pair("amount", &amount_msat.to_string())
        .append_pair("nostr", &event.as_json())
        .append_pair("lnurl", &endpoint.lnurl);

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(20))
        .build()
        .context("failed to build zap client")?;
    let invoice = client
        .get(callback)
        .send()
        .await
        .context("failed to fetch zap invoice")?
        .error_for_status()
        .context("zap invoice request returned an error")?
        .json::<LnurlZapInvoice>()
        .await
        .context("failed to parse zap invoice response")?;

    if invoice.status.as_deref() == Some("ERROR") {
        bail!(
            "{}",
            invoice
                .reason
                .filter(|reason| !reason.trim().is_empty())
                .unwrap_or_else(|| "LNURL endpoint returned an error".to_string())
        );
    }

    invoice
        .pr
        .filter(|pr| !pr.trim().is_empty())
        .ok_or_else(|| anyhow!("LNURL endpoint did not return an invoice"))
}

async fn fetch_zap_endpoint(destination: &str) -> anyhow::Result<ZapEndpoint> {
    let url = lnurl_pay_url(destination)?;
    let lnurl = encode_lnurl(url.as_str())?;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(20))
        .build()
        .context("failed to build LNURL client")?;
    let params = client
        .get(url)
        .send()
        .await
        .context("failed to fetch LNURL pay request")?
        .error_for_status()
        .context("LNURL pay request returned an error")?
        .json::<LnurlZapParams>()
        .await
        .context("failed to parse LNURL pay request")?;

    if params.tag.as_deref() != Some("payRequest") {
        bail!("LNURL endpoint is not a pay request");
    }
    if params.allows_nostr != Some(true) {
        bail!("Recipient does not support zaps.");
    }
    let _nostr_pubkey = params
        .nostr_pubkey
        .filter(|key| public_key_from_npub_or_hex(key).is_ok())
        .ok_or_else(|| anyhow!("Recipient zap endpoint returned an invalid Nostr pubkey"))?;

    Ok(ZapEndpoint {
        callback: params.callback,
        min_sendable: params.min_sendable,
        max_sendable: params.max_sendable,
        lnurl,
    })
}

fn encode_lnurl(url: &str) -> anyhow::Result<String> {
    let hrp = bech32::Hrp::parse("lnurl").context("invalid LNURL HRP")?;
    bech32::encode::<bech32::Bech32>(hrp, url.as_bytes()).context("failed to encode LNURL")
}

fn zap_relays() -> anyhow::Result<Vec<RelayUrl>> {
    NOSTR_RELAYS
        .iter()
        .map(|relay| RelayUrl::parse(relay).map_err(anyhow::Error::from))
        .collect()
}

pub(crate) async fn fetch_received_zap_receipts(
    own_pubkey: NostrPublicKey,
) -> anyhow::Result<Vec<ZapReceiptRecord>> {
    let client = nostr_client().await?;
    add_zap_scan_relays(&client).await;

    let mut receipts = Vec::new();
    let mut seen = HashSet::new();
    let filter = Filter::new()
        .kind(Kind::ZapReceipt)
        .custom_tag(SingleLetterTag::lowercase(Alphabet::P), own_pubkey.to_hex())
        .limit(200);
    let events = client
        .fetch_events(filter)
        .timeout(Duration::from_secs(10))
        .await?;
    for event in events.into_iter() {
        if !seen.insert(event.id) {
            continue;
        }
        if let Some(receipt) = zap_receipt_from_event(&event, &own_pubkey) {
            receipts.push(receipt);
        }
    }
    Ok(receipts)
}

async fn add_zap_scan_relays(client: &nostr_sdk::prelude::Client) {
    for relay in [
        "wss://nos.lol",
        "wss://relay.nostr.band",
        "wss://nostr.mom",
        "wss://relay.snort.social",
        "wss://purplepag.es",
        "wss://relay.benthecarman.com",
    ] {
        let _ = client.add_relay(relay).await;
    }
    client.connect().await;
}

pub(crate) fn zap_receipt_from_event(
    event: &Event,
    own_pubkey: &NostrPublicKey,
) -> Option<ZapReceiptRecord> {
    if event.kind != Kind::ZapReceipt {
        return None;
    }
    let own_hex = own_pubkey.to_hex();
    let tags = tag_map(event);
    let description = tags.get("description").cloned();
    let zap_request = description
        .as_deref()
        .and_then(|description| Event::from_json(description).ok());
    let request_tags = zap_request.as_ref().map(tag_map).unwrap_or_default();
    let tag_p = tag_values(event, "p");
    let tag_upper_p = tag_values(event, "P");
    let request_p = zap_request
        .as_ref()
        .map(|request| tag_values(request, "p"))
        .unwrap_or_default();
    let request_pubkey = zap_request.as_ref().map(|request| request.pubkey.to_hex());

    if !tag_p.iter().any(|value| value == &own_hex) {
        return None;
    }
    if !request_p.is_empty() && !request_p.iter().any(|value| value == &own_hex) {
        return None;
    }

    let sender_pubkey = tag_upper_p
        .first()
        .cloned()
        .or_else(|| request_pubkey.clone())?;
    if request_pubkey
        .as_ref()
        .is_some_and(|pubkey| pubkey != &sender_pubkey)
    {
        return None;
    }
    if sender_pubkey == own_hex {
        return None;
    }
    let comment = zap_request
        .as_ref()
        .map(|event| event.content.trim().to_string())
        .filter(|content| !content.is_empty());
    let amount_msat = tags
        .get("amount")
        .or_else(|| request_tags.get("amount"))
        .and_then(|value| value.parse::<u64>().ok());
    let invoice = tags.get("bolt11").cloned();
    let lnurl = request_tags.get("lnurl").cloned();
    let payment_hash = invoice
        .as_deref()
        .and_then(|invoice| Bolt11Invoice::from_str(invoice).ok())
        .map(|invoice| invoice.payment_hash().to_string());

    Some(ZapReceiptRecord {
        event_id: event.id.to_hex(),
        sender_pubkey,
        recipient_pubkey: own_hex,
        invoice,
        payment_hash,
        amount_msat,
        lnurl,
        comment,
        created_at: event.created_at.to_string().parse().unwrap_or_default(),
    })
}

fn tag_values(event: &Event, kind: &str) -> Vec<String> {
    event
        .tags
        .iter()
        .filter(|tag| tag.kind() == kind)
        .filter_map(|tag| tag.content().map(str::to_string))
        .collect()
}

fn tag_map(event: &Event) -> HashMap<String, String> {
    event
        .tags
        .iter()
        .filter_map(|tag| {
            tag.content()
                .map(|content| (tag.kind().to_string(), content.to_string()))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr_sdk::prelude::Tag;

    #[test]
    fn ignores_receipt_where_only_uppercase_p_matches_own_pubkey() {
        let own = Keys::generate();
        let sender = Keys::generate();
        let receipt = EventBuilder::new(Kind::ZapReceipt, "")
            .tags([
                Tag::parse(["P", &own.public_key().to_hex()]).unwrap(),
                Tag::parse(["p", &sender.public_key().to_hex()]).unwrap(),
                Tag::parse(["amount", "21000"]).unwrap(),
            ])
            .finalize(&Keys::generate())
            .unwrap();

        assert!(zap_receipt_from_event(&receipt, &own.public_key()).is_none());
    }

    #[test]
    fn parses_received_receipt_using_lowercase_p_as_recipient_and_uppercase_p_as_sender() {
        let own = Keys::generate();
        let sender = Keys::generate();
        let receipt = EventBuilder::new(Kind::ZapReceipt, "")
            .tags([
                Tag::parse(["p", &own.public_key().to_hex()]).unwrap(),
                Tag::parse(["P", &sender.public_key().to_hex()]).unwrap(),
                Tag::parse(["amount", "21000"]).unwrap(),
            ])
            .finalize(&Keys::generate())
            .unwrap();

        let parsed = zap_receipt_from_event(&receipt, &own.public_key()).unwrap();

        assert_eq!(parsed.recipient_pubkey, own.public_key().to_hex());
        assert_eq!(parsed.sender_pubkey, sender.public_key().to_hex());
        assert_eq!(parsed.amount_msat, Some(21_000));
    }

    #[test]
    fn parses_received_receipt_using_lowercase_p_as_recipient() {
        let own = Keys::generate();
        let sender = Keys::generate();
        let request = EventBuilder::new(Kind::ZapRequest, "thanks")
            .tags([
                Tag::parse(["p", &own.public_key().to_hex()]).unwrap(),
                Tag::parse(["amount", "1000000"]).unwrap(),
            ])
            .finalize(&sender)
            .unwrap();
        let receipt = EventBuilder::new(Kind::ZapReceipt, "")
            .tags([
                Tag::parse(["p", &own.public_key().to_hex()]).unwrap(),
                Tag::parse(["P", &sender.public_key().to_hex()]).unwrap(),
                Tag::parse(["description", &request.as_json()]).unwrap(),
            ])
            .finalize(&Keys::generate())
            .unwrap();

        let parsed = zap_receipt_from_event(&receipt, &own.public_key()).unwrap();

        assert_eq!(parsed.recipient_pubkey, own.public_key().to_hex());
        assert_eq!(parsed.sender_pubkey, sender.public_key().to_hex());
        assert_eq!(parsed.amount_msat, Some(1_000_000));
        assert_eq!(parsed.comment, Some("thanks".to_string()));
    }

    #[test]
    fn ignores_outgoing_receipt_indexed_by_uppercase_p() {
        let own = Keys::generate();
        let recipient = Keys::generate();
        let request = EventBuilder::new(Kind::ZapRequest, "")
            .tags([
                Tag::parse(["p", &recipient.public_key().to_hex()]).unwrap(),
                Tag::parse(["amount", "1000000"]).unwrap(),
            ])
            .finalize(&own)
            .unwrap();
        let receipt = EventBuilder::new(Kind::ZapReceipt, "")
            .tags([
                Tag::parse(["p", &recipient.public_key().to_hex()]).unwrap(),
                Tag::parse(["P", &own.public_key().to_hex()]).unwrap(),
                Tag::parse(["description", &request.as_json()]).unwrap(),
            ])
            .finalize(&Keys::generate())
            .unwrap();

        assert!(zap_receipt_from_event(&receipt, &own.public_key()).is_none());
    }

    #[test]
    fn ignores_receipt_when_description_recipient_differs_from_own_pubkey() {
        let own = Keys::generate();
        let sender = Keys::generate();
        let other_recipient = Keys::generate();
        let request = EventBuilder::new(Kind::ZapRequest, "")
            .tags([
                Tag::parse(["p", &other_recipient.public_key().to_hex()]).unwrap(),
                Tag::parse(["amount", "1000000"]).unwrap(),
            ])
            .finalize(&sender)
            .unwrap();
        let receipt = EventBuilder::new(Kind::ZapReceipt, "")
            .tags([
                Tag::parse(["p", &own.public_key().to_hex()]).unwrap(),
                Tag::parse(["P", &sender.public_key().to_hex()]).unwrap(),
                Tag::parse(["description", &request.as_json()]).unwrap(),
            ])
            .finalize(&Keys::generate())
            .unwrap();

        assert!(zap_receipt_from_event(&receipt, &own.public_key()).is_none());
    }

    #[test]
    fn ignores_receipt_when_sender_tag_conflicts_with_request_author() {
        let own = Keys::generate();
        let sender = Keys::generate();
        let other_sender = Keys::generate();
        let request = EventBuilder::new(Kind::ZapRequest, "")
            .tags([
                Tag::parse(["p", &own.public_key().to_hex()]).unwrap(),
                Tag::parse(["amount", "1000000"]).unwrap(),
            ])
            .finalize(&sender)
            .unwrap();
        let receipt = EventBuilder::new(Kind::ZapReceipt, "")
            .tags([
                Tag::parse(["p", &own.public_key().to_hex()]).unwrap(),
                Tag::parse(["P", &other_sender.public_key().to_hex()]).unwrap(),
                Tag::parse(["description", &request.as_json()]).unwrap(),
            ])
            .finalize(&Keys::generate())
            .unwrap();

        assert!(zap_receipt_from_event(&receipt, &own.public_key()).is_none());
    }
}
