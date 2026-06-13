use std::collections::HashSet;
use std::path::PathBuf;

use crate::nostr_support::public_key_from_npub_or_hex;
use crate::persistence::{PaymentAnnotation, ZapReceiptRecord};
use crate::profile_cache::{load_profile, profile_picture_file_url};
use crate::{ActivityItem, Contact};

pub(super) fn hydrate_contact_picture(
    profile_db: Option<&rusqlite::Connection>,
    data_dir: &PathBuf,
    contact: &mut Contact,
) {
    if contact.picture.starts_with("file://") {
        contact.picture.clear();
    }
    let Ok(pubkey) = public_key_from_npub_or_hex(&contact.npub) else {
        return;
    };
    let pubkey_hex = pubkey.to_hex();
    let Some(entry) = profile_db.and_then(|conn| load_profile(conn, &pubkey_hex).ok().flatten())
    else {
        return;
    };
    if !entry.picture_remote_url.is_empty() && entry.picture_cached_url == entry.picture_remote_url
    {
        if let Some(file_url) = profile_picture_file_url(data_dir, &pubkey_hex) {
            contact.picture = file_url;
            return;
        }
    }
    if contact.picture.is_empty() {
        contact.picture = entry.picture_remote_url;
    }
}

pub(super) fn apply_activity_metadata(
    activity: &mut [ActivityItem],
    contacts: &[Contact],
    annotations: &[PaymentAnnotation],
    zap_receipts: &[ZapReceiptRecord],
) {
    for item in activity.iter_mut() {
        if item.amount_sat < 0 {
            if let Some(annotation) = annotations
                .iter()
                .find(|annotation| annotation_matches_activity(annotation, item))
            {
                if let Some(contact) = annotation
                    .contact_id
                    .as_ref()
                    .and_then(|id| contacts.iter().find(|contact| &contact.id == id))
                    .cloned()
                {
                    apply_activity_contact(item, contact);
                }
            }
        }
    }
    apply_zap_receipts_to_activity(activity, contacts, zap_receipts);
}

fn apply_zap_receipts_to_activity(
    activity: &mut [ActivityItem],
    contacts: &[Contact],
    zap_receipts: &[ZapReceiptRecord],
) {
    let assignments = zap_receipt_activity_assignments(zap_receipts, activity);
    for (activity_index, receipt_index) in assignments {
        let item = &mut activity[activity_index];
        let receipt = &zap_receipts[receipt_index];
        if let Some(contact) = contacts
            .iter()
            .find(|contact| contact_pubkey_hex(contact).as_deref() == Some(&receipt.sender_pubkey))
            .cloned()
        {
            apply_activity_contact(item, contact);
        }
        if item.message_text.is_none() {
            item.message_text = receipt.comment.clone();
        }
        item.method_display = "Zap".to_string();
        item.method_icon = "bolt.fill".to_string();
    }
}

pub(super) fn zap_receipt_activity_assignments(
    receipts: &[ZapReceiptRecord],
    activity: &[ActivityItem],
) -> Vec<(usize, usize)> {
    let mut candidates = activity
        .iter()
        .enumerate()
        .filter(|(_, item)| item.amount_sat > 0)
        .flat_map(|(activity_index, item)| {
            receipts
                .iter()
                .enumerate()
                .filter_map(move |(receipt_index, receipt)| {
                    Some((
                        zap_receipt_match_score(receipt, item)?,
                        activity_index,
                        receipt_index,
                    ))
                })
        })
        .collect::<Vec<_>>();
    candidates.sort_by_key(|(score, activity_index, receipt_index)| {
        (*score, *activity_index, *receipt_index)
    });

    let mut used_activity = HashSet::new();
    let mut used_receipts = HashSet::new();
    let mut assignments = Vec::new();
    for (_, activity_index, receipt_index) in candidates {
        if used_activity.contains(&activity_index) || used_receipts.contains(&receipt_index) {
            continue;
        }
        used_activity.insert(activity_index);
        used_receipts.insert(receipt_index);
        assignments.push((activity_index, receipt_index));
    }
    assignments
}

#[cfg(test)]
pub(super) fn best_zap_receipt_for_activity<'a>(
    receipts: &'a [ZapReceiptRecord],
    item: &ActivityItem,
) -> Option<&'a ZapReceiptRecord> {
    receipts
        .iter()
        .filter_map(|receipt| Some((zap_receipt_match_score(receipt, item)?, receipt)))
        .min_by_key(|(score, _)| *score)
        .map(|(_, receipt)| receipt)
}

fn annotation_matches_activity(annotation: &PaymentAnnotation, item: &ActivityItem) -> bool {
    if annotation.outbound != (item.amount_sat < 0) {
        return false;
    }
    if let (Some(a), Some(b)) = (&annotation.payment_hash, &item.lightning_payment_hash) {
        if !a.is_empty() && a == b {
            return true;
        }
    }
    if let (Some(a), Some(b)) = (&annotation.invoice, &item.lightning_invoice) {
        if !a.is_empty() && a == b {
            return true;
        }
    }
    if !annotation.destination.trim().is_empty() {
        if item
            .ark_address
            .as_ref()
            .is_some_and(|address| address == &annotation.destination)
        {
            return true;
        }
    }
    annotation.amount_sat == item.amount_sat
}

pub(super) fn zap_receipt_match_score(
    receipt: &ZapReceiptRecord,
    item: &ActivityItem,
) -> Option<(u8, u64)> {
    if item.amount_sat <= 0 {
        return None;
    }
    if let (Some(a), Some(b)) = (&receipt.payment_hash, &item.lightning_payment_hash) {
        if !a.is_empty() && a == b {
            return Some((0, 0));
        }
    }
    if let (Some(a), Some(b)) = (&receipt.invoice, &item.lightning_invoice) {
        if !a.is_empty() && a == b {
            return Some((0, 0));
        }
    }
    if !zap_receipt_amount_matches_activity(receipt, item) {
        return None;
    }
    let is_lnurl_zap = receipt
        .lnurl
        .as_ref()
        .is_some_and(|value| !value.is_empty());
    if item.method_display == "Lightning address" && is_lnurl_zap {
        return Some((2, u64::MAX.saturating_sub(receipt.created_at)));
    }
    None
}

fn zap_receipt_amount_matches_activity(receipt: &ZapReceiptRecord, item: &ActivityItem) -> bool {
    receipt.amount_msat.is_some_and(|msat| {
        let rounded_down = msat / 1_000;
        let rounded_up = msat.saturating_add(999) / 1_000;
        let activity_amounts = [
            item.amount_sat.unsigned_abs(),
            item.payment_amount_sat.unsigned_abs(),
        ];
        activity_amounts
            .into_iter()
            .any(|amount| amount == rounded_down || amount == rounded_up)
    })
}

fn apply_activity_contact(item: &mut ActivityItem, contact: Contact) {
    let name = if contact.name.trim().is_empty() {
        "Unknown".to_string()
    } else {
        contact.name.clone()
    };
    if item.amount_sat >= 0 {
        item.display_primary_name = name;
        item.display_secondary_name = "you".to_string();
    } else {
        item.display_primary_name = "You".to_string();
        item.display_secondary_name = name;
    }
    item.counterparty = Some(contact);
}

fn contact_pubkey_hex(contact: &Contact) -> Option<String> {
    public_key_from_npub_or_hex(&contact.npub)
        .ok()
        .map(|pubkey| pubkey.to_hex())
}

pub(super) fn sanitize_persisted_contact_pictures(
    profile_db: Option<&rusqlite::Connection>,
    contacts: &mut [Contact],
) {
    for contact in contacts {
        if !contact.picture.starts_with("file://") {
            continue;
        }
        let Ok(pubkey) = public_key_from_npub_or_hex(&contact.npub) else {
            contact.picture.clear();
            continue;
        };
        contact.picture = profile_db
            .and_then(|conn| load_profile(conn, &pubkey.to_hex()).ok().flatten())
            .map(|entry| entry.picture_remote_url)
            .unwrap_or_default();
    }
}
