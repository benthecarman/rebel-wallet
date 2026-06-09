use anyhow::Context;
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use futures_util::{SinkExt, StreamExt};
use nostr_sdk::prelude::{
    Client as NostrClient, EventBuilder, FinalizeEvent, FromBech32, JsonUtil, Keys, Kind, Metadata,
    PublicKey as NostrPublicKey, Tag, ToBech32, Url,
};
use reqwest::multipart;
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::time::Duration;
use tokio_tungstenite::{connect_async, tungstenite::Message};

use crate::time::now_unix;
use crate::{Contact, NostrState};

pub(crate) const NOSTR_RELAYS: [&str; 3] = [
    "wss://relay.damus.io",
    "wss://nostr.wine",
    "wss://relay.primal.net",
];
const PRIMAL_URL: &str = "wss://cache2.primal.net/v1";

pub(crate) async fn nostr_client() -> anyhow::Result<NostrClient> {
    let client = NostrClient::default();
    for relay in NOSTR_RELAYS {
        client.add_relay(relay).await?;
    }
    client.connect().await;
    Ok(client)
}

#[derive(Clone, Debug)]
pub(crate) struct PrimalProfileContact {
    pub(crate) pubkey_hex: String,
    pub(crate) metadata_json: String,
    pub(crate) event_created_at: u64,
    pub(crate) picture_remote_url: String,
    pub(crate) contact: Contact,
}

pub(crate) async fn primal_follow_contacts(
    pubkey: NostrPublicKey,
) -> anyhow::Result<Vec<PrimalProfileContact>> {
    let pubkey_hex = pubkey.to_hex();
    let events = primal_request(json!(["contact_list", { "pubkey": pubkey_hex }])).await?;
    let mut latest_contact_list: Option<PrimalEvent> = None;
    let mut metadata_events = Vec::new();

    for event in events.into_iter().filter_map(primal_event_from_value) {
        match event.kind {
            0 => metadata_events.push(event),
            3 => {
                if latest_contact_list
                    .as_ref()
                    .map(|current| event.created_at > current.created_at)
                    .unwrap_or(true)
                {
                    latest_contact_list = Some(event);
                }
            }
            _ => {}
        }
    }

    let Some(contact_list) = latest_contact_list else {
        return Ok(Vec::new());
    };

    let pubkeys = contact_list
        .tags
        .iter()
        .filter(|tag| tag.first().map(|value| value.as_str()) == Some("p"))
        .filter_map(|tag| tag.get(1))
        .filter_map(|value| NostrPublicKey::from_hex(value).ok())
        .collect::<Vec<_>>();

    if pubkeys.is_empty() {
        return Ok(Vec::new());
    }

    if metadata_events.is_empty() {
        metadata_events = primal_request(json!([
            "user_infos",
            { "pubkeys": pubkeys.iter().map(|key| key.to_hex()).collect::<Vec<_>>() }
        ]))
        .await?
        .into_iter()
        .filter_map(primal_event_from_value)
        .filter(|event| event.kind == 0)
        .collect();
    }

    let mut contacts = Vec::new();
    for key in pubkeys {
        contacts.push(contact_from_primal_profile(
            key,
            latest_metadata_for_pubkey(&metadata_events, &key.to_hex()),
            true,
        ));
    }
    Ok(contacts)
}

pub(crate) async fn primal_search_profiles(
    query: &str,
) -> anyhow::Result<Vec<PrimalProfileContact>> {
    let query = query.trim();
    if query.len() < 2 {
        return Ok(Vec::new());
    }

    let mut events = if query.to_ascii_lowercase().starts_with("npub") {
        let key = public_key_from_npub_or_hex(query)?;
        primal_request(json!(["user_infos", { "pubkeys": [key.to_hex()] }])).await?
    } else {
        primal_request(json!(["user_search", { "query": query, "limit": 10 }])).await?
    };

    events.sort_by(|a, b| {
        let a_created = a
            .get("created_at")
            .and_then(Value::as_u64)
            .unwrap_or_default();
        let b_created = b
            .get("created_at")
            .and_then(Value::as_u64)
            .unwrap_or_default();
        b_created.cmp(&a_created)
    });

    let mut contacts = Vec::new();
    for event in events.into_iter().filter_map(primal_event_from_value) {
        if event.kind != 0 {
            continue;
        }
        let Ok(key) = NostrPublicKey::from_hex(&event.pubkey) else {
            continue;
        };
        let contact = contact_from_primal_profile(key, Some(&event), false);
        if !contacts
            .iter()
            .any(|c: &PrimalProfileContact| c.contact.npub == contact.contact.npub)
        {
            contacts.push(contact);
        }
    }
    Ok(contacts)
}

pub(crate) async fn primal_profile_contacts(
    pubkeys: Vec<NostrPublicKey>,
    followed: bool,
) -> anyhow::Result<Vec<PrimalProfileContact>> {
    if pubkeys.is_empty() {
        return Ok(Vec::new());
    }

    let events = primal_request(json!([
        "user_infos",
        { "pubkeys": pubkeys.iter().map(|key| key.to_hex()).collect::<Vec<_>>() }
    ]))
    .await?
    .into_iter()
    .filter_map(primal_event_from_value)
    .filter(|event| event.kind == 0)
    .collect::<Vec<_>>();

    let contacts = pubkeys
        .into_iter()
        .map(|key| {
            let pubkey_hex = key.to_hex();
            contact_from_primal_profile(
                key,
                latest_metadata_for_pubkey(&events, &pubkey_hex),
                followed,
            )
        })
        .collect();
    Ok(contacts)
}

async fn primal_request(cache_body: Value) -> anyhow::Result<Vec<Value>> {
    let sub_id = format!("rebel-{}", now_unix());
    let request = json!(["REQ", sub_id, { "cache": cache_body }]).to_string();
    let (mut socket, _) = connect_async(PRIMAL_URL).await?;
    socket.send(Message::Text(request.into())).await?;

    let mut results = Vec::new();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            anyhow::bail!("Primal request timed out");
        }

        let message = match tokio::time::timeout(remaining, socket.next()).await {
            Ok(Some(Ok(message))) => message,
            Ok(Some(Err(e))) => return Err(e.into()),
            Ok(None) => break,
            Err(_) => anyhow::bail!("Primal request timed out"),
        };
        let Message::Text(text) = message else {
            continue;
        };
        let Ok(envelope) = serde_json::from_str::<Value>(&text) else {
            continue;
        };
        let Some(items) = envelope.as_array() else {
            continue;
        };
        let message_type = items.first().and_then(Value::as_str);
        let message_sub_id = items.get(1).and_then(Value::as_str);
        if message_type == Some("NOTICE") {
            continue;
        }
        if message_sub_id != Some(sub_id.as_str()) {
            continue;
        }
        match message_type {
            Some("EVENT") => {
                if let Some(value) = items.get(2) {
                    results.push(value.clone());
                }
            }
            Some("EOSE") => break,
            Some("CLOSED") => {
                let reason = items.get(2).and_then(Value::as_str).unwrap_or("closed");
                anyhow::bail!("Primal request closed: {reason}");
            }
            _ => {}
        }
    }
    let _ = socket.close(None).await;
    Ok(results)
}

#[derive(Clone, Debug, Deserialize)]
struct PrimalEvent {
    pubkey: String,
    created_at: u64,
    kind: u64,
    tags: Vec<Vec<String>>,
    content: String,
}

#[derive(Clone, Debug, Deserialize)]
struct PrimalProfile {
    name: Option<String>,
    display_name: Option<String>,
    picture: Option<String>,
    image: Option<String>,
    lud16: Option<String>,
    lud06: Option<String>,
}

fn primal_event_from_value(value: Value) -> Option<PrimalEvent> {
    serde_json::from_value(value).ok()
}

fn latest_metadata_for_pubkey<'a>(
    events: &'a [PrimalEvent],
    pubkey_hex: &str,
) -> Option<&'a PrimalEvent> {
    events
        .iter()
        .filter(|event| event.pubkey == pubkey_hex && event.kind == 0)
        .max_by_key(|event| event.created_at)
}

fn contact_from_primal_profile(
    key: NostrPublicKey,
    event: Option<&PrimalEvent>,
    followed: bool,
) -> PrimalProfileContact {
    let npub = key.to_bech32().unwrap_or_else(|_| key.to_hex());
    let pubkey_hex = key.to_hex();
    let metadata_json = event
        .map(|event| event.content.clone())
        .unwrap_or_else(|| "{}".to_string());
    let event_created_at = event.map(|event| event.created_at).unwrap_or_default();
    let profile =
        event.and_then(|event| serde_json::from_str::<PrimalProfile>(&event.content).ok());
    let name = profile
        .as_ref()
        .and_then(|profile| profile.display_name.clone().or(profile.name.clone()))
        .filter(|name| !name.trim().is_empty())
        .unwrap_or_else(|| truncate_pubkey(&npub));
    let picture = profile
        .as_ref()
        .and_then(|profile| profile.image.clone().or(profile.picture.clone()))
        .unwrap_or_default();
    let lightning_address = profile
        .as_ref()
        .and_then(|profile| profile.lud16.clone())
        .unwrap_or_default();
    let lnurl = profile
        .as_ref()
        .and_then(|profile| profile.lud06.clone())
        .unwrap_or_default();

    PrimalProfileContact {
        pubkey_hex,
        metadata_json,
        event_created_at,
        picture_remote_url: picture.clone(),
        contact: Contact {
            id: contact_id(&npub),
            npub,
            name,
            followed,
            picture,
            lightning_address,
            lnurl,
            last_used: if followed { now_unix() } else { 0 },
        },
    }
}

fn truncate_pubkey(npub: &str) -> String {
    if npub.len() <= 18 {
        npub.to_string()
    } else {
        format!("{}...{}", &npub[..10], &npub[npub.len() - 5..])
    }
}

pub(crate) async fn upload_profile_picture(
    keys: Keys,
    image_base64: String,
) -> anyhow::Result<String> {
    const URL: &str = "https://nostr.build/api/v2/upload/profile";
    let image_bytes = BASE64
        .decode(image_base64.trim())
        .context("invalid base64 image data")?;
    let payload_hash = Sha256::digest(&image_bytes);
    let payload_hash = payload_hash
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    let auth_event = EventBuilder::new(Kind::Custom(27235), "")
        .tag(Tag::parse(["u".to_string(), URL.to_string()])?)
        .tag(Tag::parse(["method".to_string(), "POST".to_string()])?)
        .tag(Tag::parse(["payload".to_string(), payload_hash])?)
        .finalize(&keys)?;
    let auth = BASE64.encode(auth_event.as_json());
    let part = multipart::Part::bytes(image_bytes)
        .file_name("rebel-profile.jpg")
        .mime_str("image/jpeg")?;
    let form = multipart::Form::new().part("fileToUpload", part);
    let response = reqwest::Client::new()
        .post(URL)
        .header("Authorization", format!("Nostr {auth}"))
        .multipart(form)
        .send()
        .await
        .context("upload request failed")?;
    if !response.status().is_success() {
        anyhow::bail!("nostr.build returned {}", response.status());
    }
    let body: NostrBuildUploadResponse = response.json().await?;
    if body.status.as_deref() != Some("success") {
        anyhow::bail!(
            "{}",
            body.message
                .unwrap_or_else(|| "nostr.build upload failed".to_string())
        );
    }
    body.data
        .into_iter()
        .find_map(|item| item.url)
        .context("nostr.build response did not include an image URL")
}

#[derive(Debug, Deserialize)]
struct NostrBuildUploadResponse {
    status: Option<String>,
    message: Option<String>,
    data: Vec<NostrBuildUploadItem>,
}

#[derive(Debug, Deserialize)]
struct NostrBuildUploadItem {
    url: Option<String>,
}

pub(crate) fn metadata_from_state(nostr: &NostrState) -> anyhow::Result<Metadata> {
    let mut metadata = Metadata::new()
        .name(nostr.name.clone())
        .display_name(nostr.name.clone());
    if !nostr.about.is_empty() {
        metadata = metadata.about(nostr.about.clone());
    }
    if !nostr.picture.is_empty() {
        metadata = metadata.picture(Url::parse(&nostr.picture)?);
    }
    if !nostr.lud16.is_empty() {
        metadata = metadata.lud16(nostr.lud16.clone());
    }
    if !nostr.nip05.is_empty() {
        metadata = metadata.nip05(nostr.nip05.clone());
    }
    Ok(metadata)
}

pub(crate) fn deleted_profile_content() -> String {
    json!({
        "name": "Deleted",
        "display_name": "Deleted",
        "displayname": "Deleted",
        "about": "Deleted",
        "picture": Value::Null,
        "lud16": Value::Null,
        "nip05": Value::Null,
        "website": Value::Null,
        "banner": Value::Null,
        "deleted": true,
    })
    .to_string()
}

pub(crate) fn apply_metadata_content(nostr: &mut NostrState, content: &str) -> anyhow::Result<()> {
    let raw: Value = serde_json::from_str(content)?;
    if raw.get("deleted").and_then(Value::as_bool).unwrap_or(false) {
        mark_profile_deleted(nostr);
        return Ok(());
    }

    let metadata = Metadata::from_json(content.to_string())?;
    nostr.name = metadata
        .display_name
        .or(metadata.name)
        .unwrap_or(nostr.name.clone());
    nostr.about = metadata.about.unwrap_or_default();
    nostr.picture = metadata.picture.map(|u| u.to_string()).unwrap_or_default();
    nostr.lud16 = metadata.lud16.unwrap_or_default();
    nostr.nip05 = metadata.nip05.unwrap_or_default();
    nostr.deleted = false;
    Ok(())
}

pub(crate) fn mark_profile_deleted(nostr: &mut NostrState) {
    nostr.name = "Deleted".to_string();
    nostr.about = "Deleted".to_string();
    nostr.picture.clear();
    nostr.lud16.clear();
    nostr.nip05.clear();
    nostr.deleted = true;
}

pub(crate) fn public_key_from_npub_or_hex(value: &str) -> anyhow::Result<NostrPublicKey> {
    let trimmed = value.trim();
    if trimmed.starts_with("npub") {
        NostrPublicKey::from_bech32(trimmed).context("invalid npub")
    } else {
        NostrPublicKey::from_hex(trimmed).context("invalid Nostr pubkey")
    }
}

pub(crate) fn merge_contacts(existing: &mut Vec<Contact>, fetched: Vec<Contact>) {
    for contact in fetched {
        if let Some(current) = existing.iter_mut().find(|c| c.npub == contact.npub) {
            current.followed = contact.followed;
            current.last_used = now_unix();
            if current.name.is_empty() {
                current.name = contact.name;
            }
            if current.picture.is_empty() {
                current.picture = contact.picture;
            }
            if should_replace_lightning_address(
                &current.lightning_address,
                &contact.lightning_address,
            ) {
                current.lightning_address = contact.lightning_address;
            }
            if current.lnurl.is_empty() {
                current.lnurl = contact.lnurl;
            }
        } else {
            existing.push(contact);
        }
    }
    existing.sort_by(|a, b| b.last_used.cmp(&a.last_used).then(a.name.cmp(&b.name)));
}

pub(crate) fn contact_id(input: &str) -> String {
    let hex = input
        .chars()
        .filter(|c| c.is_ascii_hexdigit())
        .take(16)
        .collect::<String>();
    if hex.is_empty() {
        format!("contact-{}", now_unix())
    } else {
        hex
    }
}

fn should_replace_lightning_address(current: &str, fetched: &str) -> bool {
    !fetched.trim().is_empty()
        && (current.trim().is_empty()
            || (!is_valid_lightning_address(current) && is_valid_lightning_address(fetched)))
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

#[cfg(test)]
mod tests {
    use super::*;

    fn contact(npub: &str, name: &str, followed: bool, last_used: u64) -> Contact {
        Contact {
            id: contact_id(npub),
            npub: npub.to_string(),
            name: name.to_string(),
            followed,
            picture: String::new(),
            lightning_address: String::new(),
            lnurl: String::new(),
            last_used,
        }
    }

    #[test]
    fn merge_contacts_preserves_local_names_and_updates_follow_state() {
        let mut existing = vec![contact("npub1234", "Local Alice", false, 10)];
        let fetched = vec![
            contact("npub1234", "Remote Alice", true, 1),
            contact("npub9999", "Bob", true, 2),
        ];

        merge_contacts(&mut existing, fetched);

        let alice = existing.iter().find(|c| c.npub == "npub1234").unwrap();
        assert_eq!(alice.name, "Local Alice");
        assert!(alice.followed);
        assert_eq!(existing.len(), 2);
    }

    #[test]
    fn merge_contacts_replaces_invalid_lightning_address_with_valid_fetched_value() {
        let mut existing = vec![Contact {
            lightning_address: "not-valid".to_string(),
            ..contact("npub1234", "Alice", true, 10)
        }];
        let fetched = vec![Contact {
            lightning_address: "alice@example.com".to_string(),
            ..contact("npub1234", "Alice", true, 1)
        }];

        merge_contacts(&mut existing, fetched);

        assert_eq!(existing[0].lightning_address, "alice@example.com");
    }

    #[test]
    fn deleted_profile_content_tombstones_profile_fields() {
        let content: Value = serde_json::from_str(&deleted_profile_content()).unwrap();

        assert_eq!(content["name"], "Deleted");
        assert_eq!(content["display_name"], "Deleted");
        assert_eq!(content["displayname"], "Deleted");
        assert_eq!(content["about"], "Deleted");
        assert_eq!(content["picture"], Value::Null);
        assert_eq!(content["lud16"], Value::Null);
        assert_eq!(content["nip05"], Value::Null);
        assert_eq!(content["deleted"], true);
    }

    #[test]
    fn apply_metadata_content_marks_deleted_profile() {
        let mut nostr = NostrState {
            npub: None,
            name: "Alice".to_string(),
            about: "hello".to_string(),
            picture: "https://example.com/a.png".to_string(),
            lud16: "alice@example.com".to_string(),
            nip05: "alice@example.com".to_string(),
            deleted: false,
            contacts: vec![],
        };

        apply_metadata_content(&mut nostr, &deleted_profile_content()).unwrap();

        assert_eq!(nostr.name, "Deleted");
        assert_eq!(nostr.about, "Deleted");
        assert!(nostr.picture.is_empty());
        assert!(nostr.lud16.is_empty());
        assert!(nostr.nip05.is_empty());
        assert!(nostr.deleted);
    }
}
