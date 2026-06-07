use anyhow::Context;
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use nostr_sdk::prelude::{
    Client as NostrClient, EventBuilder, FinalizeEvent, FromBech32, JsonUtil, Keys, Kind, Metadata,
    PublicKey as NostrPublicKey, Tag, Url,
};
use reqwest::multipart;
use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::time::now_unix;
use crate::{Contact, NostrState};

const NOSTR_RELAYS: [&str; 3] = [
    "wss://relay.damus.io",
    "wss://nostr.wine",
    "wss://relay.primal.net",
];

pub(crate) async fn nostr_client() -> anyhow::Result<NostrClient> {
    let client = NostrClient::default();
    for relay in NOSTR_RELAYS {
        client.add_relay(relay).await?;
    }
    client.connect().await;
    Ok(client)
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
}
