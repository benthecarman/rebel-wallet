mod derive;
mod types;
#[cfg(test)]
mod tests;

pub use types::*;

pub(crate) use derive::{format_sats, format_signed_sats};
use derive::{
    arkzap_lightning_address, format_fiat_sats, format_non_btc_fiat_sats, is_sendable_search_query,
    receive_request, send_destination_kind, send_error_text, send_search_results,
    should_show_launch_splash, supported_networks, supported_price_currencies,
};

impl AppState {
    pub(crate) fn initial() -> Self {
        Self {
            rev: 0,
            show_launch_splash: true,
            router: Router {
                default_screen: Screen::Setup,
                screen_stack: vec![],
                selected_tab: MainTab::Home,
            },
            setup: SetupState::NeedsSetup,
            wallet: WalletState {
                network: WalletNetwork::Signet,
                network_name: WalletNetwork::Signet.display_name().to_string(),
                default_server_address: WalletNetwork::Signet.server_address().to_string(),
                default_esplora_address: WalletNetwork::Signet.esplora_address().to_string(),
                server_address: WalletNetwork::Signet.server_address().to_string(),
                esplora_address: WalletNetwork::Signet.esplora_address().to_string(),
                price_currency: PriceCurrency::BTC,
                price_currency_code: PriceCurrency::BTC.code().to_string(),
                price_currency_name: PriceCurrency::BTC.display_name().to_string(),
                btc_price: None,
                balance_sat: 0,
                balance_display: format_sats(0),
                balance_fiat_display: None,
                pending_receive_sat: 0,
                pending_receive_display: format_sats(0),
                pending_receive_fiat_display: None,
                pending_send_sat: 0,
                pending_send_display: format_sats(0),
                pending_send_fiat_display: None,
                pending_refresh_sat: 0,
                pending_refresh_display: format_sats(0),
                pending_refresh_fiat_display: None,
                last_sync: None,
            },
            receive: ReceiveState {
                method: ReceiveMethod::Lightning,
                phase: ReceivePhase::Editing,
                ark_address: None,
                lightning_invoice: None,
                receive_request: None,
                lightning_payment_hash: None,
                lightning_status: "idle".to_string(),
                lightning_status_display: "Waiting".to_string(),
                lightning_paid: false,
                amount_sat: 10_000,
                amount_display: format_sats(10_000),
                memo: "Rebel Wallet".to_string(),
            },
            send: SendState {
                destination: String::new(),
                destination_kind: SendDestinationKind::Unknown,
                phase: SendPhase::Drafting,
                search_query: String::new(),
                search_results: vec![],
                global_search_results: vec![],
                can_continue_search: false,
                selected_contact_id: None,
                zap_enabled: false,
                zap_available: false,
                amount_sat: 0,
                amount_display: format_sats(0),
                estimating_fee: false,
                fee_estimate_sat: None,
                fee_estimate_display: None,
                fee_estimate_fiat_display: None,
                total_cost_sat: None,
                total_cost_display: None,
                total_cost_fiat_display: None,
                fee_estimate_error: None,
                memo: String::new(),
                last_result: None,
                success_amount_display: format_sats(0),
                can_submit: false,
                error_text: None,
            },
            lightning_address: LightningAddressState {
                address: None,
                backing_ark_address: None,
            },
            nostr: NostrState {
                npub: None,
                name: "Rebel".to_string(),
                about: String::new(),
                picture: String::new(),
                lud16: String::new(),
                nip05: String::new(),
                deleted: false,
                contacts: vec![],
            },
            supported_networks: supported_networks(),
            supported_price_currencies: supported_price_currencies(),
            direct_messages: vec![],
            activity: vec![],
            recovery_phrase: None,
            toast: None,
            busy: BusyState::default(),
            capability_request: None,
        }
    }

    pub(crate) fn refresh_derived(&mut self) {
        self.show_launch_splash = should_show_launch_splash(self);
        self.supported_networks = supported_networks();
        self.wallet.network_name = self.wallet.network.display_name().to_string();
        self.wallet.default_server_address = self.wallet.network.server_address().to_string();
        self.wallet.default_esplora_address = self.wallet.network.esplora_address().to_string();
        self.supported_price_currencies = supported_price_currencies();
        self.wallet.price_currency_code = self.wallet.price_currency.code().to_string();
        self.wallet.price_currency_name = self.wallet.price_currency.display_name().to_string();
        self.wallet.balance_display = format_sats(self.wallet.balance_sat);
        self.wallet.pending_receive_display = format_sats(self.wallet.pending_receive_sat);
        self.wallet.pending_send_display = format_sats(self.wallet.pending_send_sat);
        self.wallet.pending_refresh_display = format_sats(self.wallet.pending_refresh_sat);
        self.wallet.balance_fiat_display = format_fiat_sats(
            self.wallet.balance_sat,
            self.wallet.btc_price,
            &self.wallet.price_currency,
        );
        self.wallet.pending_receive_fiat_display = format_fiat_sats(
            self.wallet.pending_receive_sat,
            self.wallet.btc_price,
            &self.wallet.price_currency,
        );
        self.wallet.pending_send_fiat_display = format_fiat_sats(
            self.wallet.pending_send_sat,
            self.wallet.btc_price,
            &self.wallet.price_currency,
        );
        self.wallet.pending_refresh_fiat_display = format_fiat_sats(
            self.wallet.pending_refresh_sat,
            self.wallet.btc_price,
            &self.wallet.price_currency,
        );
        for item in &mut self.activity {
            item.amount_display = format_sats(item.amount_sat.unsigned_abs());
            item.amount_fiat_display = format_fiat_sats(
                item.amount_sat.unsigned_abs(),
                self.wallet.btc_price,
                &self.wallet.price_currency,
            );
            item.signed_amount_display = format_signed_sats(item.amount_sat, true);
        }

        self.receive.amount_display = format_sats(self.receive.amount_sat);
        self.receive.receive_request = receive_request(&self.receive);
        self.receive.lightning_status_display = if self.receive.lightning_paid {
            "Paid".to_string()
        } else {
            match self.receive.lightning_status.as_str() {
                "claiming" => "Claiming".to_string(),
                "claimable" => "Claimable".to_string(),
                "paid" => "Paid".to_string(),
                _ => "Waiting".to_string(),
            }
        };

        self.send.amount_display = format_sats(self.send.amount_sat);
        self.send.fee_estimate_display = self.send.fee_estimate_sat.map(format_sats);
        self.send.total_cost_display = self.send.total_cost_sat.map(format_sats);
        self.send.fee_estimate_fiat_display = self.send.fee_estimate_sat.and_then(|amount| {
            format_non_btc_fiat_sats(amount, self.wallet.btc_price, &self.wallet.price_currency)
        });
        self.send.total_cost_fiat_display = self.send.total_cost_sat.and_then(|amount| {
            format_non_btc_fiat_sats(amount, self.wallet.btc_price, &self.wallet.price_currency)
        });
        self.send.search_results = send_search_results(
            &self.send.search_query,
            &self.nostr.contacts,
            &self.send.global_search_results,
            self.nostr.npub.as_deref(),
        );
        self.send.can_continue_search = is_sendable_search_query(&self.send.search_query);
        if self.send.destination.trim().is_empty() && self.send.phase == SendPhase::Editing {
            self.send.phase = SendPhase::Drafting;
        }
        self.send.destination_kind = send_destination_kind(&self.send.destination);
        self.send.error_text = send_error_text(
            self.send.destination_kind.clone(),
            self.send.amount_sat,
            self.send.total_cost_sat,
            self.wallet.balance_sat,
        );
        self.send.can_submit = !self.send.destination.trim().is_empty()
            && self.send.phase != SendPhase::Sending
            && !self.send.estimating_fee
            && self.send.error_text.is_none()
            && (self.send.destination_kind == SendDestinationKind::Lightning
                || self.send.amount_sat > 0);
        if !self.send.zap_available {
            self.send.zap_enabled = false;
        }

        self.lightning_address.address = self
            .lightning_address
            .backing_ark_address
            .as_ref()
            .filter(|address| !address.trim().is_empty())
            .map(|address| arkzap_lightning_address(address));
    }

    pub(crate) fn reset_receive_draft(&mut self) {
        self.receive.method = ReceiveMethod::Lightning;
        self.receive.phase = ReceivePhase::Editing;
        self.receive.ark_address = None;
        self.receive.lightning_invoice = None;
        self.receive.receive_request = None;
        self.receive.lightning_payment_hash = None;
        self.receive.lightning_status = "idle".to_string();
        self.receive.lightning_paid = false;
        self.receive.amount_sat = 0;
    }
}
