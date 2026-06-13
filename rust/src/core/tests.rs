use nostr_sdk::prelude::{Alphabet, FromBech32, SecretKey as NostrSecretKey, SingleLetterTag};

use crate::{ActivityIconKind, LightningAddressState};

use super::activity_meta::{
    best_zap_receipt_for_activity, zap_receipt_activity_assignments, zap_receipt_match_score,
};
use super::*;

#[test]
fn derives_nostr_key_from_wallet_seed_path() {
    let keys = derive_nostr_keys_from_mnemonic(
        "leader monkey parrot ring guide accident before fence cannon height naive bean",
    )
    .unwrap();

    assert_eq!(
        keys.secret_key().as_secret_bytes(),
        NostrSecretKey::parse(
            "7f7ff03d123792d6ac594bfa67bf6d0c0ab55b6b1fdb6249303fe861f1ccba9a",
        )
        .unwrap()
        .as_secret_bytes(),
    );
}

#[test]
fn matches_lightning_address_zap_receipt_by_destination_amount() {
    let receipt = ZapReceiptRecord {
        event_id: "zap-1".to_string(),
        sender_pubkey: "sender".to_string(),
        recipient_pubkey: "recipient".to_string(),
        invoice: None,
        payment_hash: None,
        amount_msat: Some(21_000),
        lnurl: Some("lnurl1test".to_string()),
        comment: None,
        created_at: 1,
    };
    let item = test_activity_item("Lightning address", 20, 21);

    assert!(best_zap_receipt_for_activity(&[receipt], &item).is_some());
}

#[test]
fn does_not_match_non_lightning_address_activity_by_amount_only() {
    let receipt = ZapReceiptRecord {
        event_id: "zap-1".to_string(),
        sender_pubkey: "sender".to_string(),
        recipient_pubkey: "recipient".to_string(),
        invoice: None,
        payment_hash: None,
        amount_msat: Some(21_000),
        lnurl: Some("lnurl1test".to_string()),
        comment: None,
        created_at: 1,
    };
    let item = test_activity_item("Ark", 21, 21);

    assert!(best_zap_receipt_for_activity(&[receipt], &item).is_none());
}

#[test]
fn does_not_match_ark_activity_by_amount_even_when_time_is_close() {
    let receipt = ZapReceiptRecord {
        event_id: "zap-1".to_string(),
        sender_pubkey: "sender".to_string(),
        recipient_pubkey: "recipient".to_string(),
        invoice: None,
        payment_hash: None,
        amount_msat: Some(1_000_000),
        lnurl: Some("lnurl1test".to_string()),
        comment: None,
        created_at: 1_781_055_500,
    };
    let mut item = test_activity_item("Ark", 1_000, 1_000);
    item.completed_at_unix = 1_781_056_000;

    assert!(best_zap_receipt_for_activity(&[receipt], &item).is_none());
}

#[test]
fn picks_exact_payment_hash_before_amount_fallback() {
    let older = ZapReceiptRecord {
        event_id: "zap-older".to_string(),
        sender_pubkey: "wrong-sender".to_string(),
        recipient_pubkey: "recipient".to_string(),
        invoice: None,
        payment_hash: None,
        amount_msat: Some(1_000_000),
        lnurl: Some("lnurl1test".to_string()),
        comment: None,
        created_at: 1_781_055_100,
    };
    let closer = ZapReceiptRecord {
        event_id: "zap-closer".to_string(),
        sender_pubkey: "right-sender".to_string(),
        recipient_pubkey: "recipient".to_string(),
        invoice: None,
        payment_hash: Some("payment-hash".to_string()),
        amount_msat: Some(1_000_000),
        lnurl: Some("lnurl1test".to_string()),
        comment: None,
        created_at: 1_781_055_980,
    };
    let mut item = test_activity_item("Lightning address", 1_000, 1_000);
    item.lightning_payment_hash = Some("payment-hash".to_string());
    let receipts = vec![older, closer];

    let receipt = best_zap_receipt_for_activity(&receipts, &item).unwrap();

    assert_eq!(receipt.sender_pubkey, "right-sender");
}

#[test]
fn prefers_lnurl_zap_receipt_for_lightning_address_amount_fallback() {
    let wrong = ZapReceiptRecord {
        event_id: "zap-wrong".to_string(),
        sender_pubkey: "wrong-sender".to_string(),
        recipient_pubkey: "recipient".to_string(),
        invoice: None,
        payment_hash: None,
        amount_msat: Some(1_000_000),
        lnurl: None,
        comment: None,
        created_at: 1_705_622_583,
    };
    let expected = ZapReceiptRecord {
        event_id: "zap-expected".to_string(),
        sender_pubkey: "expected-sender".to_string(),
        recipient_pubkey: "recipient".to_string(),
        invoice: None,
        payment_hash: None,
        amount_msat: Some(1_000_000),
        lnurl: Some("lnurl1test".to_string()),
        comment: None,
        created_at: 1_701_463_372,
    };
    let item = test_activity_item("Lightning address", 1_000, 1_000);
    let receipts = vec![wrong, expected];

    let receipt = best_zap_receipt_for_activity(&receipts, &item).unwrap();

    assert_eq!(receipt.sender_pubkey, "expected-sender");
}

#[test]
fn assigns_each_zap_receipt_to_only_one_activity() {
    let receipt = ZapReceiptRecord {
        event_id: "zap-1".to_string(),
        sender_pubkey: "sender".to_string(),
        recipient_pubkey: "recipient".to_string(),
        invoice: None,
        payment_hash: Some("payment-hash".to_string()),
        amount_msat: Some(1_000_000),
        lnurl: Some("lnurl1test".to_string()),
        comment: None,
        created_at: 1_781_055_500,
    };
    let mut first = test_activity_item("Ark", 1_000, 1_000);
    first.id = "activity-1".to_string();
    first.lightning_payment_hash = Some("payment-hash".to_string());
    first.completed_at_unix = 1_781_055_500;
    let mut second = test_activity_item("Ark", 1_000, 1_000);
    second.id = "activity-2".to_string();
    second.lightning_payment_hash = Some("payment-hash".to_string());
    second.completed_at_unix = 1_781_055_510;
    let activity = vec![first, second];

    let assignments = zap_receipt_activity_assignments(&[receipt], &activity);

    assert_eq!(assignments.len(), 1);
    assert_eq!(assignments[0].1, 0);
}

#[test]
fn assigns_each_activity_to_only_one_zap_receipt() {
    let older = ZapReceiptRecord {
        event_id: "zap-older".to_string(),
        sender_pubkey: "older-sender".to_string(),
        recipient_pubkey: "recipient".to_string(),
        invoice: None,
        payment_hash: Some("payment-hash".to_string()),
        amount_msat: Some(1_000_000),
        lnurl: Some("lnurl1test".to_string()),
        comment: None,
        created_at: 1_781_055_100,
    };
    let closer = ZapReceiptRecord {
        event_id: "zap-closer".to_string(),
        sender_pubkey: "closer-sender".to_string(),
        recipient_pubkey: "recipient".to_string(),
        invoice: None,
        payment_hash: Some("payment-hash".to_string()),
        amount_msat: Some(1_000_000),
        lnurl: Some("lnurl1test".to_string()),
        comment: None,
        created_at: 1_781_055_490,
    };
    let mut item = test_activity_item("Ark", 1_000, 1_000);
    item.lightning_payment_hash = Some("payment-hash".to_string());
    item.completed_at_unix = 1_781_055_500;
    let receipts = vec![older, closer];
    let activity = vec![item];

    let assignments = zap_receipt_activity_assignments(&receipts, &activity);

    assert_eq!(assignments, vec![(0, 0)]);
}

#[test]
fn assigns_one_lnurl_amount_fallback_when_one_receipt_matches_multiple_activities() {
    let receipt = ZapReceiptRecord {
        event_id: "zap-1".to_string(),
        sender_pubkey: "sender".to_string(),
        recipient_pubkey: "recipient".to_string(),
        invoice: None,
        payment_hash: None,
        amount_msat: Some(1_000_000),
        lnurl: Some("lnurl1test".to_string()),
        comment: None,
        created_at: 1_781_055_500,
    };
    let mut first = test_activity_item("Lightning address", 1_000, 1_000);
    first.id = "activity-1".to_string();
    let mut second = test_activity_item("Lightning address", 1_000, 1_000);
    second.id = "activity-2".to_string();

    let assignments = zap_receipt_activity_assignments(&[receipt], &[first, second]);

    assert_eq!(assignments, vec![(0, 0)]);
}

#[test]
fn assigns_one_lnurl_amount_fallback_when_one_activity_matches_multiple_receipts() {
    let older = ZapReceiptRecord {
        event_id: "zap-older".to_string(),
        sender_pubkey: "older-sender".to_string(),
        recipient_pubkey: "recipient".to_string(),
        invoice: None,
        payment_hash: None,
        amount_msat: Some(1_000_000),
        lnurl: Some("lnurl1test".to_string()),
        comment: None,
        created_at: 1_781_055_100,
    };
    let newer = ZapReceiptRecord {
        event_id: "zap-newer".to_string(),
        sender_pubkey: "newer-sender".to_string(),
        recipient_pubkey: "recipient".to_string(),
        invoice: None,
        payment_hash: None,
        amount_msat: Some(1_000_000),
        lnurl: Some("lnurl1test".to_string()),
        comment: None,
        created_at: 1_781_055_500,
    };
    let item = test_activity_item("Lightning address", 1_000, 1_000);

    let assignments = zap_receipt_activity_assignments(&[older, newer], &[item]);

    assert_eq!(assignments, vec![(0, 1)]);
}

#[tokio::test]
#[ignore]
async fn e2e_matches_real_wallet_zap_receipts_to_activity() {
    let expected_sender = NostrPublicKey::from_bech32(
        "nprofile1qqs8r0afe0uyzyx7v9lftyppkzxxj5j0e2ssx0laqc4t6zhzv4a6ynqjgyx99",
    )
    .expect("expected sender nprofile")
    .to_hex();
    let wrong_sender = NostrPublicKey::from_bech32(
        "npub1p4kg8zxukpym3h20erfa3samj00rm2gt4q5wfuyu3tg0x3jg3gesvncxf8",
    )
    .expect("wrong sender npub")
    .to_hex();
    println!("expected_sender={expected_sender}");
    println!("wrong_sender={wrong_sender}");
    let mnemonic = std::env::var("REBEL_WALLET_E2E_MNEMONIC")
        .expect("set REBEL_WALLET_E2E_MNEMONIC for this ignored test");
    let mnemonic = Mnemonic::from_str(&mnemonic).expect("valid mnemonic");
    let data_dir = tempfile::tempdir().expect("temp data dir");
    let wallet = open_bark_wallet(
        data_dir.path().to_path_buf(),
        &mnemonic,
        WalletOpenMode::Restore,
        ServerConfig::for_network(WalletNetwork::Mainnet),
    )
    .await
    .expect("open wallet");
    wallet.sync().await;

    let keys = derive_nostr_keys_from_mnemonic(&mnemonic.to_string()).expect("nostr keys");
    println!(
        "derived_npub={}",
        keys.public_key().to_bech32().expect("derived npub")
    );
    let mut receipts = fetch_received_zap_receipts(keys.public_key())
        .await
        .expect("fetch derived zap receipts");
    let reported_pubkey = std::env::var("REBEL_WALLET_E2E_NPUB")
        .ok()
        .and_then(|npub| public_key_from_npub_or_hex(&npub).ok())
        .unwrap_or_else(|| {
            public_key_from_npub_or_hex(
                "npub1u8lnhlw5usp3t9vmpz60ejpyt649z33hu82wc2hpv6m5xdqmuxhs46turz",
            )
            .expect("reported npub")
        });
    if reported_pubkey != keys.public_key() {
        let reported_receipts = fetch_received_zap_receipts(reported_pubkey)
            .await
            .expect("fetch reported zap receipts");
        println!("reported_pubkey_receipts={}", reported_receipts.len());
        receipts.extend(reported_receipts);
    }
    let client = nostr_client().await.expect("nostr client");
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
    for (label, tag) in [
        ("raw lowercase p", SingleLetterTag::lowercase(Alphabet::P)),
        ("raw uppercase P", SingleLetterTag::uppercase(Alphabet::P)),
    ] {
        let events = client
            .fetch_events(
                Filter::new()
                    .kind(Kind::ZapReceipt)
                    .custom_tag(tag, reported_pubkey.to_hex())
                    .limit(200),
            )
            .timeout(Duration::from_secs(10))
            .await
            .expect("raw zap fetch");
        println!("{label} events={}", events.len());
        for event in events
            .into_iter()
            .filter(|event| event.created_at.as_secs() > 1_780_000_000)
        {
            let parsed = crate::zaps::zap_receipt_from_event(&event, &reported_pubkey);
            println!(
                "{label} recent id={} created_at={} parsed={}",
                event.id,
                event.created_at.as_secs(),
                parsed.is_some()
            );
        }
    }
    let history = wallet.history().await.expect("wallet history");
    let backing_ark_address = history
        .iter()
        .filter(|movement| {
            is_user_visible_movement(movement) && movement.effective_balance.to_sat() > 0
        })
        .find_map(|movement| {
            movement
                .received_on
                .first()
                .map(|destination| destination.destination.value_string())
        });
    println!("backing_ark_address={backing_ark_address:?}");
    for movement in history.iter().filter(|movement| {
        is_user_visible_movement(movement) && movement.effective_balance.to_sat() > 0
    }) {
        let movement_hash = movement
            .lightning_payment_hash()
            .map(|hash| hash.to_string());
        println!(
            "movement id={} effective_sat={} completed_at={:?} updated_at={} movement_hash={:?} input_vtxos={} output_vtxos={}",
            movement.id,
            movement.effective_balance.to_sat(),
            movement.time.completed_at,
            movement.time.updated_at,
            movement_hash,
            movement.input_vtxos.len(),
            movement.output_vtxos.len()
        );
        for id in movement
            .output_vtxos
            .iter()
            .chain(movement.input_vtxos.iter())
        {
            let Ok(vtxo) = wallet.get_full_vtxo(*id).await else {
                println!("  vtxo id={id} unavailable");
                continue;
            };
            let policy_hash = match vtxo.policy() {
                VtxoPolicy::ServerHtlcSend(policy) => {
                    Some(("server_htlc_send", policy.payment_hash.to_string()))
                }
                VtxoPolicy::ServerHtlcRecv(policy) => {
                    Some(("server_htlc_recv", policy.payment_hash.to_string()))
                }
                VtxoPolicy::Pubkey(_) => None,
            };
            let witness_hashes = vtxo
                .transactions()
                .flat_map(|item| item.tx.input)
                .flat_map(|input| input.witness.to_vec())
                .filter(|element| element.len() == 32)
                .filter_map(|element| Preimage::from_slice(&element).ok())
                .map(|preimage| preimage.compute_payment_hash().to_string())
                .collect::<Vec<_>>();
            println!(
                "  vtxo id={id} policy_hash={policy_hash:?} witness_hashes={witness_hashes:?}"
            );
        }
    }
    let synced = wallet_synced_msg(
        &wallet,
        &[],
        &LightningAddressState {
            address: None,
            backing_ark_address,
        },
        &[],
        &receipts,
        false,
    )
    .await
    .expect("synced activity");
    let AsyncMsg::WalletSynced { mut activity, .. } = synced else {
        panic!("expected wallet synced");
    };
    for item in activity
        .iter_mut()
        .filter(|item| item.amount_sat > 0 && item.payment_amount_sat.unsigned_abs() == 1_000)
    {
        item.method_display = "Lightning address".to_string();
    }

    println!("receipts={}", receipts.len());
    for receipt in receipts.iter().filter(|receipt| {
        receipt.created_at > 1_780_000_000
            || receipt
                .amount_msat
                .is_some_and(|amount| amount == 1_000_000 || amount == 1_000)
    }) {
        println!(
            "receipt event={} created_at={} amount_msat={:?} lnurl={} hash={:?} sender={}",
            receipt.event_id,
            receipt.created_at,
            receipt.amount_msat,
            receipt.lnurl.is_some(),
            receipt.payment_hash,
            receipt.sender_pubkey
        );
    }

    let assignments = zap_receipt_activity_assignments(&receipts, &activity);
    let mut matched = 0;
    for item in activity.iter().filter(|item| item.amount_sat > 0) {
        let receipt = assignments
            .iter()
            .find(|(activity_index, _)| &activity[*activity_index].id == &item.id)
            .map(|(_, receipt_index)| &receipts[*receipt_index]);
        if receipt.is_some() {
            matched += 1;
        }
        let mut candidates = receipts
            .iter()
            .filter_map(|receipt| Some((zap_receipt_match_score(receipt, item)?, receipt)))
            .collect::<Vec<_>>();
        candidates.sort_by_key(|(score, _)| *score);
        for (score, receipt) in candidates.iter().take(8) {
            println!(
                "  candidate score={score:?} event={} created_at={} amount_msat={:?} lnurl={} sender={}",
                receipt.event_id,
                receipt.created_at,
                receipt.amount_msat,
                receipt.lnurl.is_some(),
                receipt.sender_pubkey
            );
        }
        println!(
            "activity id={} completed_at_unix={} amount_sat={} payment_amount_sat={} method={} hash={:?} invoice_present={} matched_sender={:?}",
            item.id,
            item.completed_at_unix,
            item.amount_sat,
            item.payment_amount_sat,
            item.method_display,
            item.lightning_payment_hash,
            item.lightning_invoice.is_some(),
            receipt.map(|receipt| receipt.sender_pubkey.as_str())
        );
    }

    println!("matched_inbound_count={matched}");
    let expected_match = assignments.iter().any(|(activity_index, receipt_index)| {
        let item = &activity[*activity_index];
        item.amount_sat > 0
            && item.payment_amount_sat.unsigned_abs() == 1_000
            && receipts[*receipt_index].sender_pubkey == expected_sender
    });
    let wrong_match = assignments.iter().any(|(activity_index, receipt_index)| {
        let item = &activity[*activity_index];
        item.amount_sat > 0
            && item.payment_amount_sat.unsigned_abs() == 1_000
            && receipts[*receipt_index].sender_pubkey == wrong_sender
    });
    assert!(
        expected_match,
        "expected a 1000-sat activity to pair with the requested nprofile"
    );
    assert!(
        !wrong_match,
        "a 1000-sat activity still pairs with the known wrong npub"
    );
    assert!(!activity.is_empty(), "expected synced wallet activity");
}

fn test_activity_item(
    method_display: &str,
    amount_sat: i64,
    payment_amount_sat: i64,
) -> ActivityItem {
    ActivityItem {
        id: "activity-1".to_string(),
        title: String::new(),
        subtitle: String::new(),
        display_primary_name: "Unknown".to_string(),
        display_verb: "sent".to_string(),
        display_secondary_name: "you".to_string(),
        message_text: None,
        method_icon: "bolt.fill".to_string(),
        method_display: method_display.to_string(),
        amount_sat,
        payment_amount_sat,
        amount_display: String::new(),
        amount_fiat_display: None,
        signed_amount_display: String::new(),
        icon_kind: ActivityIconKind::Received,
        status: String::new(),
        timestamp: String::new(),
        completed_at_unix: 0,
        counterparty: None,
        ark_address: None,
        lightning_invoice: None,
        lightning_payment_hash: None,
        lightning_payment_preimage: None,
    }
}
