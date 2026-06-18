use nostr_sdk::prelude::PublicKey as NostrPublicKey;

use super::{profile_picture_download_key, AppCore};
use crate::nostr_support::{
    primal_profile_contacts, public_key_from_npub_or_hex, FetchedProfileContact,
};
use crate::profile_cache::{
    download_profile_picture, load_profile, profile_picture_file_url, save_profile,
    ProfileCacheEntry,
};
use crate::updates::{AsyncMsg, CoreMsg};
use crate::Contact;

impl AppCore {
    pub(super) fn cache_fetched_profile_contacts(
        &mut self,
        records: Vec<FetchedProfileContact>,
    ) -> Vec<Contact> {
        records
            .into_iter()
            .map(|record| self.cache_fetched_profile_contact(record))
            .collect()
    }

    /// Saves fetched profile metadata and returns a render-ready contact.
    ///
    /// This is the only actor path that should turn fetched profile metadata
    /// into UI contact state. It preserves the remote URL in SQLite and swaps
    /// the render URL to a cached `file://` pfp when the cache is current.
    pub(super) fn cache_fetched_profile_contact(
        &mut self,
        record: FetchedProfileContact,
    ) -> Contact {
        let mut contact = record.contact;
        let cached = self
            .profile_db
            .as_ref()
            .and_then(|conn| load_profile(conn, &record.pubkey_hex).ok().flatten());
        let cached_file_url = cached
            .as_ref()
            .filter(|entry| {
                !record.picture_remote_url.is_empty()
                    && entry.picture_cached_url == record.picture_remote_url
            })
            .and_then(|_| profile_picture_file_url(&self.cache_dir, &record.pubkey_hex));

        if let Some(file_url) = cached_file_url {
            contact.picture = file_url;
        } else if !record.picture_remote_url.is_empty() {
            contact.picture = record.picture_remote_url.clone();
        }

        if let Some(conn) = self.profile_db.as_ref() {
            let previous = cached.unwrap_or_else(|| ProfileCacheEntry {
                pubkey: record.pubkey_hex.clone(),
                metadata_json: "{}".to_string(),
                name: String::new(),
                picture_remote_url: String::new(),
                picture_cached_url: String::new(),
                picture_cached_at: 0,
                lightning_address: String::new(),
                lnurl: String::new(),
                event_created_at: 0,
            });
            let same_remote = previous.picture_remote_url == record.picture_remote_url;
            let entry = ProfileCacheEntry {
                pubkey: record.pubkey_hex,
                metadata_json: record.metadata_json,
                name: contact.name.clone(),
                picture_remote_url: record.picture_remote_url.clone(),
                picture_cached_url: if same_remote {
                    previous.picture_cached_url
                } else {
                    String::new()
                },
                picture_cached_at: if same_remote {
                    previous.picture_cached_at
                } else {
                    0
                },
                lightning_address: contact.lightning_address.clone(),
                lnurl: contact.lnurl.clone(),
                event_created_at: record.event_created_at,
            };
            let _ = save_profile(conn, &entry);
        }

        contact
    }

    pub(super) fn prefetch_profile_pictures(&mut self, contact_ids: Vec<String>) {
        let contacts = contact_ids
            .into_iter()
            .filter_map(|contact_id| {
                self.state
                    .send
                    .search_results
                    .iter()
                    .chain(self.state.send.global_search_results.iter())
                    .chain(self.state.nostr.contacts.iter())
                    .find(|contact| contact.id == contact_id)
                    .cloned()
            })
            .collect();
        self.prefetch_profile_pictures_for_contacts(contacts);
    }

    pub(super) fn prefetch_activity_profile_pictures(&mut self) {
        let mut contacts = Vec::new();
        for contact in self
            .state
            .activity
            .iter()
            .filter_map(|item| item.counterparty.clone())
        {
            if !contacts
                .iter()
                .any(|existing: &Contact| existing.npub == contact.npub)
            {
                contacts.push(contact);
            }
        }
        self.prefetch_profile_pictures_for_contacts(contacts);
    }

    pub(super) fn refresh_cached_contact_profiles_on_startup(&mut self) {
        let contacts = self.state.nostr.contacts.clone();
        self.prefetch_profile_pictures_for_contacts(contacts.clone());
        let pubkeys = contacts
            .into_iter()
            .filter_map(|contact| public_key_from_npub_or_hex(&contact.npub).ok())
            .filter(|pubkey| self.profile_info_requests.insert(pubkey.to_hex()))
            .collect();
        self.spawn_primal_profile_prefetch(pubkeys);
    }

    fn prefetch_profile_pictures_for_contacts(&mut self, contacts: Vec<Contact>) {
        let mut missing_profile_pubkeys = Vec::new();
        for contact in contacts {
            let Ok(pubkey) = public_key_from_npub_or_hex(&contact.npub) else {
                continue;
            };
            let pubkey_hex = pubkey.to_hex();
            if !self.prefetch_profile_picture_for_pubkey(&pubkey_hex, &contact.picture)
                && self.profile_info_requests.insert(pubkey_hex.clone())
            {
                missing_profile_pubkeys.push(pubkey);
            }
        }

        self.spawn_primal_profile_prefetch(missing_profile_pubkeys);
    }

    /// Ensures a profile picture flows through the Rust disk cache instead of
    /// leaving views to fetch remote pfp URLs directly.
    ///
    /// Returns `false` when no remote URL is known yet; callers that have a
    /// pubkey can then fetch profile metadata and retry through this function.
    pub(super) fn prefetch_profile_picture_for_pubkey(
        &mut self,
        pubkey_hex: &str,
        picture: &str,
    ) -> bool {
        let mut remote_url = picture.to_string();
        if remote_url.starts_with("file://") {
            return true;
        }
        if remote_url.trim().is_empty() {
            remote_url = self
                .profile_db
                .as_ref()
                .and_then(|conn| load_profile(conn, pubkey_hex).ok().flatten())
                .map(|entry| entry.picture_remote_url)
                .unwrap_or_default();
        }

        if remote_url.trim().is_empty() {
            return false;
        }

        let cached_file_url = self
            .profile_db
            .as_ref()
            .and_then(|conn| load_profile(conn, pubkey_hex).ok().flatten())
            .filter(|entry| entry.picture_cached_url == remote_url)
            .and_then(|_| profile_picture_file_url(&self.cache_dir, pubkey_hex));
        if cached_file_url.is_some() {
            self.refresh_contact_picture_for_pubkey(pubkey_hex);
            self.refresh_own_profile_picture_for_pubkey(pubkey_hex);
            return true;
        }

        self.spawn_profile_picture_download(pubkey_hex.to_string(), remote_url);
        true
    }

    fn spawn_primal_profile_prefetch(&self, pubkeys: Vec<NostrPublicKey>) {
        if pubkeys.is_empty() {
            return;
        }

        let tx = self.tx.clone();
        self.rt.spawn(async move {
            let pubkey_hexes = pubkeys.iter().map(|key| key.to_hex()).collect::<Vec<_>>();
            match primal_profile_contacts(pubkeys, true).await {
                Ok(records) => {
                    let _ = tx.send(CoreMsg::Async(AsyncMsg::PrimalProfilesLoaded { records }));
                }
                Err(_) => {
                    let _ = tx.send(CoreMsg::Async(AsyncMsg::PrimalProfilesFailed {
                        pubkeys: pubkey_hexes,
                    }));
                }
            }
        });
    }

    fn spawn_profile_picture_download(&mut self, pubkey: String, remote_url: String) {
        if remote_url.trim().is_empty() {
            return;
        }
        let Some(scheme) = remote_url.split(':').next().map(|s| s.to_ascii_lowercase()) else {
            return;
        };
        if scheme != "https" && scheme != "http" {
            return;
        }

        let download_key = profile_picture_download_key(&pubkey, &remote_url);
        if !self.profile_picture_downloads.insert(download_key) {
            return;
        }

        let tx = self.tx.clone();
        let cache_dir = self.cache_dir.clone();
        let semaphore = self.profile_picture_download_semaphore.clone();
        self.rt.spawn(async move {
            let client = reqwest::Client::new();
            let failed_pubkey = pubkey.clone();
            let failed_remote_url = remote_url.clone();
            match download_profile_picture(client, cache_dir, pubkey, remote_url, semaphore).await {
                Ok((pubkey, remote_url)) => {
                    let _ = tx.send(CoreMsg::Async(AsyncMsg::ProfilePictureCached {
                        pubkey,
                        remote_url,
                    }));
                }
                Err(err) => {
                    let _ = err;
                    let _ = tx.send(CoreMsg::Async(AsyncMsg::ProfilePictureCacheFailed {
                        pubkey: failed_pubkey,
                        remote_url: failed_remote_url,
                    }));
                }
            }
        });
    }

    pub(super) fn refresh_contact_picture_for_pubkey(&mut self, pubkey: &str) {
        let Some(file_url) = profile_picture_file_url(&self.cache_dir, pubkey) else {
            return;
        };
        for contact in &mut self.state.nostr.contacts {
            let Ok(contact_pubkey) = public_key_from_npub_or_hex(&contact.npub) else {
                continue;
            };
            if contact_pubkey.to_hex() == pubkey {
                contact.picture = file_url.clone();
            }
        }
        for contact in &mut self.state.send.global_search_results {
            let Ok(contact_pubkey) = public_key_from_npub_or_hex(&contact.npub) else {
                continue;
            };
            if contact_pubkey.to_hex() == pubkey {
                contact.picture = file_url.clone();
            }
        }
        self.save_app_data();
    }

    pub(super) fn refresh_own_profile_picture_for_pubkey(&mut self, pubkey: &str) {
        let Some(npub) = self.state.nostr.npub.as_deref() else {
            return;
        };
        let Ok(own_pubkey) = public_key_from_npub_or_hex(npub) else {
            return;
        };
        if own_pubkey.to_hex() != pubkey {
            return;
        }
        let Some(file_url) = profile_picture_file_url(&self.cache_dir, pubkey) else {
            return;
        };
        self.state.nostr.picture_display_url = file_url;
        self.save_app_data();
    }

    pub(super) fn refresh_activity_picture_for_pubkey(&mut self, pubkey: &str) {
        let Some(file_url) = profile_picture_file_url(&self.cache_dir, pubkey) else {
            return;
        };
        for item in &mut self.state.activity {
            let Some(contact) = item.counterparty.as_mut() else {
                continue;
            };
            let Ok(contact_pubkey) = public_key_from_npub_or_hex(&contact.npub) else {
                continue;
            };
            if contact_pubkey.to_hex() == pubkey {
                contact.picture = file_url.clone();
            }
        }
    }
}
