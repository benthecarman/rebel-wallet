use super::*;

#[test]
fn formats_unsigned_and_signed_sats() {
    assert_eq!(format_sats(0), "0 sats");
    assert_eq!(format_sats(1_234_567), "1,234,567 sats");
    assert_eq!(format_signed_sats(42, true), "+42 sats");
    assert_eq!(format_signed_sats(-42, true), "-42 sats");
    assert_eq!(format_signed_sats(42, false), "42 sats");
}

#[test]
fn derives_send_destination_kind_and_validation() {
    let mut state = AppState::initial();
    state.wallet.balance_sat = 1_000;

    state.send.destination = "lightning:lnbc1example".to_string();
    state.refresh_derived();
    assert_eq!(state.send.destination_kind, SendDestinationKind::Lightning);
    assert!(state.send.can_submit);
    assert_eq!(state.send.error_text, None);

    state.send.destination = "ark1example".to_string();
    state.send.amount_sat = 0;
    state.refresh_derived();
    assert_eq!(state.send.destination_kind, SendDestinationKind::Ark);
    assert!(!state.send.can_submit);
    assert_eq!(
        state.send.error_text.as_deref(),
        Some("Enter an amount before sending to an Ark address.")
    );

    state.send.amount_sat = 2_000;
    state.refresh_derived();
    assert!(!state.send.can_submit);
    assert_eq!(
        state.send.error_text.as_deref(),
        Some("Insufficient balance for this send.")
    );

    state.send.destination = " Alice@Example.com ".to_string();
    state.send.amount_sat = 1;
    state.refresh_derived();
    assert_eq!(state.send.destination_kind, SendDestinationKind::Lightning);
    assert!(state.send.can_submit);

    state.send.amount_sat = 900;
    state.send.total_cost_sat = Some(1_001);
    state.refresh_derived();
    assert!(!state.send.can_submit);
    assert_eq!(
        state.send.error_text.as_deref(),
        Some("Insufficient balance for this send.")
    );
}

#[test]
fn hides_zap_when_not_available() {
    let mut state = AppState::initial();
    state.send.destination = "lnbc1example".to_string();
    state.send.zap_available = false;
    state.send.zap_enabled = true;

    state.refresh_derived();

    assert!(!state.send.zap_available);
    assert!(!state.send.zap_enabled);
}

#[test]
fn derives_receive_status_display() {
    let mut state = AppState::initial();
    state.receive.lightning_status = "claimable".to_string();
    state.refresh_derived();
    assert_eq!(state.receive.lightning_status_display, "Claimable");

    state.receive.lightning_paid = true;
    state.refresh_derived();
    assert_eq!(state.receive.lightning_status_display, "Paid");
}

#[test]
fn derives_receive_request_for_ark_and_lightning() {
    let mut state = AppState::initial();
    state.receive.method = ReceiveMethod::Ark;
    state.receive.ark_address = Some("tark1fdafa".to_string());
    state.receive.amount_sat = 0;
    state.refresh_derived();
    assert_eq!(state.receive.receive_request.as_deref(), Some("tark1fdafa"));

    state.receive.amount_sat = 50_000;
    state.refresh_derived();
    assert_eq!(
        state.receive.receive_request.as_deref(),
        Some("bitcoin:?amount=0.0005&ark=tark1fdafa")
    );

    state.receive.method = ReceiveMethod::Lightning;
    state.receive.lightning_invoice = Some("lnbc1example".to_string());
    state.refresh_derived();
    assert_eq!(
        state.receive.receive_request.as_deref(),
        Some("lnbc1example")
    );
}

#[test]
fn reset_receive_draft_restores_default_method() {
    let mut state = AppState::initial();
    state.receive.method = ReceiveMethod::Ark;
    state.receive.phase = ReceivePhase::ShowingRequest;
    state.receive.ark_address = Some("tark1fdafa".to_string());
    state.receive.receive_request = Some("tark1fdafa".to_string());
    state.receive.amount_sat = 50_000;

    state.reset_receive_draft();

    assert_eq!(state.receive.method, ReceiveMethod::Lightning);
    assert_eq!(state.receive.phase, ReceivePhase::Editing);
    assert_eq!(state.receive.ark_address, None);
    assert_eq!(state.receive.receive_request, None);
    assert_eq!(state.receive.amount_sat, 0);
}

#[test]
fn derives_platform_render_metadata() {
    let mut state = AppState::initial();
    state.rev = 1;
    state.busy.bootstrapping = false;
    state.busy.opening_wallet = false;
    state.refresh_derived();

    assert!(!state.show_launch_splash);
    assert_eq!(state.wallet.price_currency_code, "BTC");
    assert_eq!(state.wallet.price_currency_name, "Bitcoin");
    assert_eq!(state.supported_price_currencies.len(), 4);

    state.setup = SetupState::Ready;
    state.busy.syncing_wallet = true;
    state.wallet.last_sync = None;
    state.refresh_derived();
    assert!(state.show_launch_splash);
}

#[test]
fn derives_network_profiles() {
    let mut state = AppState::initial();
    state.refresh_derived();

    assert_eq!(state.supported_networks.len(), 2);
    assert!(state
        .supported_networks
        .iter()
        .any(|network| network.network == WalletNetwork::Signet));
    assert!(state
        .supported_networks
        .iter()
        .any(|network| network.network == WalletNetwork::Mainnet));
    assert_eq!(
        WalletNetwork::Signet.db_file_name(),
        "rebel-wallet-signet.sqlite"
    );
    assert_eq!(
        WalletNetwork::Mainnet.db_file_name(),
        "rebel-wallet-mainnet.sqlite"
    );
    assert_eq!(WalletNetwork::Signet.server_access_token(), None);
    assert_eq!(WalletNetwork::Mainnet.server_access_token(), None);
}

#[test]
fn derives_activity_fiat_displays_from_selected_currency() {
    let mut state = AppState::initial();
    state.wallet.price_currency = PriceCurrency::USD;
    state.wallet.btc_price = Some(100_000.0);
    state.activity = vec![ActivityItem {
        id: "activity-1".to_string(),
        title: String::new(),
        subtitle: String::new(),
        display_primary_name: "You".to_string(),
        display_verb: "sent".to_string(),
        display_secondary_name: "Alice".to_string(),
        message_text: None,
        method_icon: "bolt.fill".to_string(),
        method_display: "Lightning".to_string(),
        amount_sat: -50_000,
        payment_amount_sat: -50_000,
        amount_display: String::new(),
        amount_fiat_display: None,
        signed_amount_display: String::new(),
        icon_kind: ActivityIconKind::Sent,
        status: "complete".to_string(),
        timestamp: String::new(),
        completed_at_unix: 0,
        counterparty: None,
        ark_address: None,
        lightning_invoice: None,
        lightning_payment_hash: None,
        lightning_payment_preimage: None,
    }];

    state.refresh_derived();

    let item = &state.activity[0];
    assert_eq!(item.amount_display, "50,000 sats");
    assert_eq!(item.amount_fiat_display.as_deref(), Some("$50.00 USD"));
    assert_eq!(item.signed_amount_display, "-50,000 sats");
}

#[test]
fn derives_send_search_results_from_contacts() {
    let mut state = AppState::initial();
    state.nostr.contacts = vec![
        Contact {
            id: "1".to_string(),
            npub: "npubalice".to_string(),
            name: "Alice".to_string(),
            followed: true,
            picture: String::new(),
            lightning_address: "alice@example.com".to_string(),
            lnurl: String::new(),
            last_used: 10,
        },
        Contact {
            id: "2".to_string(),
            npub: "npubbob".to_string(),
            name: "Bob".to_string(),
            followed: true,
            picture: String::new(),
            lightning_address: String::new(),
            lnurl: "lnurl1bob".to_string(),
            last_used: 20,
        },
        Contact {
            id: "4".to_string(),
            npub: "npubdave".to_string(),
            name: "dave".to_string(),
            followed: true,
            picture: String::new(),
            lightning_address: "not-a-lightning-address".to_string(),
            lnurl: String::new(),
            last_used: 30,
        },
    ];
    state.send.global_search_results = vec![Contact {
        id: "3".to_string(),
        npub: "npubcarol".to_string(),
        name: "Carol".to_string(),
        followed: false,
        picture: String::new(),
        lightning_address: "carol@example.com".to_string(),
        lnurl: String::new(),
        last_used: 0,
    }];
    state.send.search_query = "ali".to_string();
    state.refresh_derived();

    assert_eq!(state.send.search_results.len(), 1);
    assert_eq!(state.send.search_results[0].name, "Alice");
    assert!(!state.send.can_continue_search);

    state.send.search_query = "alice@example.com".to_string();
    state.refresh_derived();
    assert!(state.send.can_continue_search);

    state.send.search_query = "car".to_string();
    state.refresh_derived();
    assert_eq!(state.send.search_results.len(), 1);
    assert_eq!(state.send.search_results[0].name, "Carol");

    state.send.search_query = "bob".to_string();
    state.refresh_derived();
    assert!(state.send.search_results.is_empty());

    state.send.search_query = "dave".to_string();
    state.refresh_derived();
    assert!(state.send.search_results.is_empty());
}

#[test]
fn send_search_results_include_full_contact_list() {
    let mut state = AppState::initial();
    state.nostr.contacts = (0..75)
        .map(|idx| Contact {
            id: format!("contact-{idx}"),
            npub: format!("npub{idx}"),
            name: format!("Contact {idx}"),
            followed: true,
            picture: String::new(),
            lightning_address: format!("contact{idx}@example.com"),
            lnurl: String::new(),
            last_used: idx,
        })
        .collect();

    state.refresh_derived();

    assert_eq!(state.send.search_results.len(), 75);
}

#[test]
fn send_search_results_sort_names_ignoring_case() {
    let mut state = AppState::initial();
    state.nostr.contacts = vec![
        Contact {
            id: "1".to_string(),
            npub: "npub1".to_string(),
            name: "bravo".to_string(),
            followed: true,
            picture: String::new(),
            lightning_address: "bravo@example.com".to_string(),
            lnurl: String::new(),
            last_used: 0,
        },
        Contact {
            id: "2".to_string(),
            npub: "npub2".to_string(),
            name: "Alpha".to_string(),
            followed: true,
            picture: String::new(),
            lightning_address: "alpha@example.com".to_string(),
            lnurl: String::new(),
            last_used: 0,
        },
    ];

    state.refresh_derived();

    assert_eq!(state.send.search_results[0].name, "Alpha");
    assert_eq!(state.send.search_results[1].name, "bravo");
}

#[test]
fn send_search_results_trim_names_and_finalize_sort_by_npub() {
    let mut state = AppState::initial();
    state.nostr.contacts = vec![
        Contact {
            id: "1".to_string(),
            npub: "npub-b".to_string(),
            name: "  Same  ".to_string(),
            followed: true,
            picture: String::new(),
            lightning_address: "b@example.com".to_string(),
            lnurl: String::new(),
            last_used: 0,
        },
        Contact {
            id: "2".to_string(),
            npub: "npub-a".to_string(),
            name: "same".to_string(),
            followed: true,
            picture: String::new(),
            lightning_address: "a@example.com".to_string(),
            lnurl: String::new(),
            last_used: 0,
        },
    ];

    state.refresh_derived();

    assert_eq!(state.send.search_results[0].npub, "npub-a");
    assert_eq!(state.send.search_results[0].name, "same");
    assert_eq!(state.send.search_results[1].npub, "npub-b");
    assert_eq!(state.send.search_results[1].name, "Same");
}

#[test]
fn derives_arkzap_lightning_address() {
    let mut state = AppState::initial();
    state.nostr.lud16 = "saved@example.com".to_string();
    state.lightning_address.backing_ark_address = Some("tark1example".to_string());
    state.refresh_derived();
    assert_eq!(
        state.lightning_address.address.as_deref(),
        Some("tark1example@signet.arkzap.me")
    );
    assert_eq!(state.nostr.lud16, "saved@example.com");

    state.wallet.network = WalletNetwork::Mainnet;
    state.lightning_address.backing_ark_address = Some("ark1example".to_string());
    state.refresh_derived();
    assert_eq!(
        state.lightning_address.address.as_deref(),
        Some("ark1example@arkzap.me")
    );
    assert_eq!(state.nostr.lud16, "saved@example.com");
}
