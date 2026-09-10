import SwiftUI
import UIKit

struct SettingsView: View {
    @Bindable var manager: AppManager
    @Environment(\.walletAccent) private var walletAccent

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 18) {
                Button {
                    manager.dispatch(.selectTab(tab: .home))
                } label: {
                    MutinyCircle(size: 44) {
                        Image(systemName: "chevron.left")
                            .font(.headline.weight(.semibold))
                    }
                }
                .buttonStyle(.plain)
                .accessibilityLabel("Back to wallet")

                SettingsCard(title: "General") {
                    SettingsRow(title: "Backup", caption: "Show recovery phrase", accent: rebelGreen) {
                        manager.dispatch(.pushScreen(screen: .backup))
                    }
                    SettingsDivider()
                    SettingsRow(title: "Restore", caption: "Replace this wallet from seed words", accent: walletAccent) {
                        manager.dispatch(.pushScreen(screen: .restore))
                    }
                    SettingsDivider()
                    SettingsRow(title: "Network", caption: manager.state.wallet.networkName) {
                        manager.dispatch(.pushScreen(screen: .network))
                    }
                }

                SettingsCard(title: "Appearance") {
                    SettingsRow(title: "Currency", caption: manager.state.wallet.priceCurrencyCode) {
                        manager.dispatch(.pushScreen(screen: .currency))
                    }
                    SettingsDivider()
                    SettingsRow(title: "Language", caption: "English", disabled: true) {}
                }

                SettingsCard(title: "Social") {
                    SettingsRow(title: "Nostr keys", caption: manager.state.nostr.npub ?? "No Nostr key") {
                        manager.dispatch(.pushScreen(screen: .profile))
                    }
                    SettingsDivider()
                    SettingsRow(
                        title: "Nostr Wallet Connect",
                        caption: "Connect Rebel Wallet to an External App",
                        accent: rebelBlue,
                        badgeText: nwcConnectionBadge
                    ) {
                        manager.dispatch(.pushScreen(screen: .nwc))
                    }
                }

                SettingsCard(title: "Diagnostics") {
                    SettingsRow(title: "Wallet Diagnostics", caption: "VTXOs and refresh history") {
                        manager.dispatch(.pushScreen(screen: .walletDiagnostics))
                    }
                }

                SettingsCard(title: "Danger Zone") {
                    SettingsRow(
                        title: "Delete Wallet",
                        caption: "Remove local wallet data and start over",
                        accent: rebelRed
                    ) {
                        presentDeleteWalletConfirmation()
                    }
                }

            }
            .padding(16)
        }
        .navigationTitle("Settings")
        .background(pageBackground)
        .foregroundStyle(primaryText)
    }

    private func presentDeleteWalletConfirmation() {
        guard let presenter = UIApplication.shared.rebelTopViewController() else {
            return
        }

        let sheet = UIAlertController(
            title: "Delete wallet and start over?",
            message: nil,
            preferredStyle: .actionSheet
        )
        sheet.addAction(UIAlertAction(title: "Delete Wallet", style: .destructive) { _ in
            manager.dispatch(.deleteWallet)
        })
        sheet.addAction(UIAlertAction(title: "Cancel", style: .cancel))

        if UIDevice.current.userInterfaceIdiom == .pad, let popover = sheet.popoverPresentationController {
            popover.sourceView = presenter.view
            popover.sourceRect = CGRect(
                x: presenter.view.bounds.midX,
                y: presenter.view.bounds.maxY,
                width: 1,
                height: 1
            )
            popover.permittedArrowDirections = []
        }

        presenter.present(sheet, animated: true)
    }

    private var nwcConnectionBadge: String? {
        let count = manager.state.nwc.connections.count
        return count > 0 ? "\(count)" : nil
    }

}

struct SettingsCard<Content: View>: View {
    let title: String
    @ViewBuilder let content: Content

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            Text(title)
                .font(.caption.bold())
                .foregroundStyle(mutedText)
                .padding(.horizontal, 14)
                .padding(.top, 12)
                .padding(.bottom, 6)
            content
        }
        .background(surfaceBackground, in: RoundedRectangle(cornerRadius: 12))
        .overlay(RoundedRectangle(cornerRadius: 12).stroke(borderColor))
    }
}

struct SettingsRow: View {
    let title: String
    let caption: String?
    var accent: Color?
    var disabled = false
    var badgeText: String?
    let action: () -> Void

    init(title: String, caption: String? = nil, accent: Color? = nil, disabled: Bool = false, badgeText: String? = nil, action: @escaping () -> Void) {
        self.title = title
        self.caption = caption
        self.accent = accent
        self.disabled = disabled
        self.badgeText = badgeText
        self.action = action
    }

    var body: some View {
        Button(action: action) {
            HStack(spacing: 12) {
                VStack(alignment: .leading, spacing: 4) {
                    Text(title)
                        .font(.body)
                        .foregroundStyle(accent ?? primaryText)
                    if let caption, !caption.isEmpty {
                        Text(caption)
                            .font(.caption)
                            .foregroundStyle(mutedText)
                            .lineLimit(2)
                    }
                }
                Spacer()
                if let badgeText, !badgeText.isEmpty {
                    Text(badgeText)
                        .font(.caption.bold())
                        .foregroundStyle(accent ?? primaryText)
                        .padding(.horizontal, 9)
                        .padding(.vertical, 5)
                        .background((accent ?? primaryText).opacity(0.14), in: Capsule())
                }
                Image(systemName: "chevron.right")
                    .foregroundStyle(mutedText)
            }
            .padding(.horizontal, 14)
            .padding(.vertical, 12)
            .frame(maxWidth: .infinity, alignment: .leading)
            .contentShape(Rectangle())
            .opacity(disabled ? 0.45 : 1)
        }
        .buttonStyle(.plain)
        .disabled(disabled)
    }
}

struct SettingsDivider: View {
    var body: some View {
        Divider()
            .overlay(borderColor)
            .padding(.leading, 14)
    }
}

struct CurrencyView: View {
    @Bindable var manager: AppManager
    @State private var selectedCurrency: PriceCurrency = .btc

    private var hasChanges: Bool {
        selectedCurrency != manager.state.wallet.priceCurrency
    }

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 18) {
                Text("Choose the currency used for balances and price conversions.")
                    .font(.body)
                    .foregroundStyle(mutedText)

                SettingsCard(title: "Select currency") {
                    ForEach(Array(manager.state.supportedPriceCurrencies.enumerated()), id: \.element.currency) { index, option in
                        Button {
                            if selectedCurrency != option.currency {
                                manager.requestHaptic(.selection)
                            }
                            selectedCurrency = option.currency
                        } label: {
                            HStack(spacing: 12) {
                                VStack(alignment: .leading, spacing: 4) {
                                    Text(option.code)
                                        .font(.body)
                                        .foregroundStyle(primaryText)
                                    Text(option.name)
                                        .font(.caption)
                                        .foregroundStyle(mutedText)
                                }
                                Spacer()
                                if selectedCurrency == option.currency {
                                    Image(systemName: "checkmark")
                                        .foregroundStyle(rebelGreen)
                                }
                            }
                            .padding(.horizontal, 14)
                            .padding(.vertical, 12)
                            .frame(maxWidth: .infinity, alignment: .leading)
                            .contentShape(Rectangle())
                        }
                        .buttonStyle(.plain)

                        if index < manager.state.supportedPriceCurrencies.count - 1 {
                            SettingsDivider()
                        }
                    }
                }

                Button {
                    manager.dispatch(.setPriceCurrency(currency: selectedCurrency))
                    manager.dispatch(.popScreen)
                } label: {
                    Label("Select currency", systemImage: "checkmark")
                        .frame(maxWidth: .infinity)
                }
                .buttonStyle(PrimaryButtonStyle(color: rebelBlue))
                .disabled(!hasChanges)
            }
            .padding(16)
        }
        .navigationTitle("Currency")
        .scrollContentBackground(.hidden)
        .background(pageBackground)
        .foregroundStyle(primaryText)
        .onAppear {
            selectedCurrency = manager.state.wallet.priceCurrency
        }
    }
}

struct NetworkView: View {
    @Bindable var manager: AppManager
    @State private var selectedNetwork: WalletNetwork = .mainnet
    @State private var regtestServerAddress = "http://127.0.0.1:3535"
    @State private var regtestEsploraAddress = "http://127.0.0.1:3000"

    private var hasChanges: Bool {
        if selectedNetwork != manager.state.wallet.network {
            return true
        }
        guard selectedNetwork == .regtest else {
            return false
        }
        return regtestServerAddress.trimmedServerAddress != manager.state.wallet.serverAddress
            || regtestEsploraAddress.trimmedServerAddress != manager.state.wallet.esploraAddress
    }

    private var hasValidRegtestFields: Bool {
        selectedNetwork != .regtest
            || (!regtestServerAddress.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
                && !regtestEsploraAddress.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
    }

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 18) {
                Text("Choose the Bitcoin network this wallet connects to.")
                    .font(.body)
                    .foregroundStyle(mutedText)

                SettingsCard(title: "Select network") {
                    ForEach(Array(manager.state.supportedNetworks.enumerated()), id: \.element.network) { index, option in
                        Button {
                            if selectedNetwork != option.network {
                                manager.requestHaptic(.selection)
                            }
                            selectedNetwork = option.network
                        } label: {
                            HStack(spacing: 12) {
                                VStack(alignment: .leading, spacing: 4) {
                                    Text(option.name)
                                        .font(.body)
                                        .foregroundStyle(primaryText)
                                    Text(option.caption)
                                        .font(.caption)
                                        .foregroundStyle(mutedText)
                                }
                                Spacer()
                                if selectedNetwork == option.network {
                                    Image(systemName: "checkmark")
                                        .foregroundStyle(rebelGreen)
                                }
                            }
                            .padding(.horizontal, 14)
                            .padding(.vertical, 12)
                            .frame(maxWidth: .infinity, alignment: .leading)
                            .contentShape(Rectangle())
                        }
                        .buttonStyle(.plain)

                        if index < manager.state.supportedNetworks.count - 1 {
                            SettingsDivider()
                        }
                    }
                }

                if selectedNetwork == .regtest {
                    SettingsCard(title: "Regtest servers") {
                        VStack(alignment: .leading, spacing: 7) {
                            Text("ASP URL")
                                .font(.caption)
                                .foregroundStyle(mutedText)
                            TextField("http://127.0.0.1:3535", text: $regtestServerAddress)
                                .textInputAutocapitalization(.never)
                                .autocorrectionDisabled()
                                .keyboardType(.URL)
                                .foregroundStyle(primaryText)
                        }
                        .padding(14)

                        SettingsDivider()

                        VStack(alignment: .leading, spacing: 7) {
                            Text("Esplora URL")
                                .font(.caption)
                                .foregroundStyle(mutedText)
                            TextField("http://127.0.0.1:3000", text: $regtestEsploraAddress)
                                .textInputAutocapitalization(.never)
                                .autocorrectionDisabled()
                                .keyboardType(.URL)
                                .foregroundStyle(primaryText)
                        }
                        .padding(14)
                    }

                    Text("On a physical iPhone, use the server computer's LAN address instead of 127.0.0.1.")
                        .font(.caption)
                        .foregroundStyle(mutedText)
                }

                if manager.state.busy.openingWallet {
                    HStack(spacing: 10) {
                        ProgressView()
                        Text("Reconnecting wallet")
                            .font(.footnote)
                            .foregroundStyle(mutedText)
                    }
                }

                Button {
                    manager.dispatch(.selectNetwork(
                        network: selectedNetwork,
                        serverAddress: selectedNetwork == .regtest ? regtestServerAddress.trimmedServerAddress : nil,
                        esploraAddress: selectedNetwork == .regtest ? regtestEsploraAddress.trimmedServerAddress : nil
                    ))
                    manager.dispatch(.popScreen)
                } label: {
                    Label("Select network", systemImage: "checkmark")
                        .frame(maxWidth: .infinity)
                }
                .buttonStyle(PrimaryButtonStyle(color: rebelBlue))
                .disabled(!hasChanges || !hasValidRegtestFields || manager.state.busy.openingWallet)
            }
            .padding(16)
        }
        .navigationTitle("Network")
        .scrollContentBackground(.hidden)
        .background(pageBackground)
        .foregroundStyle(primaryText)
        .onAppear {
            selectedNetwork = manager.state.wallet.network
            if selectedNetwork == .regtest {
                regtestServerAddress = manager.state.wallet.serverAddress
                regtestEsploraAddress = manager.state.wallet.esploraAddress
            }
        }
    }
}

private extension String {
    var trimmedServerAddress: String {
        trimmingCharacters(in: .whitespacesAndNewlines)
            .trimmingCharacters(in: CharacterSet(charactersIn: "/"))
    }
}
