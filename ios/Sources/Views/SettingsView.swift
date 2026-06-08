import SwiftUI

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
                        manager.dispatch(.showSeed)
                        manager.dispatch(.pushScreen(screen: .backup))
                    }
                    SettingsDivider()
                    SettingsRow(title: "Restore", caption: "Replace this wallet from seed words", accent: walletAccent) {
                        manager.dispatch(.pushScreen(screen: .restore))
                    }
                    SettingsDivider()
                    SettingsRow(title: "Servers", caption: manager.state.wallet.serverAddress) {
                        manager.dispatch(.pushScreen(screen: .servers))
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
                }

                Text("Secrets are stored in iOS Keychain. Wallet data uses local sqlite.")
                    .font(.caption)
                    .foregroundStyle(mutedText)
                    .frame(maxWidth: .infinity, alignment: .center)
                    .padding(.bottom, 24)
            }
            .padding(16)
        }
        .navigationTitle("Settings")
        .background(pageBackground)
        .foregroundStyle(primaryText)
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
    let action: () -> Void

    init(title: String, caption: String? = nil, accent: Color? = nil, disabled: Bool = false, action: @escaping () -> Void) {
        self.title = title
        self.caption = caption
        self.accent = accent
        self.disabled = disabled
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

struct ServersView: View {
    @Bindable var manager: AppManager
    @State private var serverAddress = ""
    @State private var esploraAddress = ""
    @State private var lnurlServerAddress = ""

    private var canSave: Bool {
        !serverAddress.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty &&
            !esploraAddress.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty &&
            !lnurlServerAddress.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty &&
            !manager.state.busy.openingWallet
    }

    private var hasChanges: Bool {
        normalized(serverAddress) != manager.state.wallet.serverAddress ||
            normalized(esploraAddress) != manager.state.wallet.esploraAddress ||
            normalized(lnurlServerAddress) != manager.state.wallet.lnurlServerAddress
    }

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 18) {
                Text("Configure the Ark, Esplora, and Lightning address endpoints used by this Signet wallet.")
                    .font(.body)
                    .foregroundStyle(mutedText)

                SettingsCard(title: "Network") {
                    ServerTextField(title: "Ark server", text: $serverAddress)
                    SettingsDivider()
                    ServerTextField(title: "Esplora", text: $esploraAddress)
                    SettingsDivider()
                    ServerTextField(title: "LNURL server", text: $lnurlServerAddress)
                }

                if manager.state.busy.openingWallet {
                    HStack(spacing: 10) {
                        ProgressView()
                        Text("Reconnecting wallet")
                            .font(.footnote)
                            .foregroundStyle(mutedText)
                    }
                }

                HStack(spacing: 12) {
                    Button {
                        serverAddress = manager.state.wallet.defaultServerAddress
                        esploraAddress = manager.state.wallet.defaultEsploraAddress
                        lnurlServerAddress = manager.state.wallet.defaultLnurlServerAddress
                    } label: {
                        Label("Defaults", systemImage: "arrow.counterclockwise")
                            .frame(maxWidth: .infinity)
                    }
                    .buttonStyle(SecondaryButtonStyle())

                    Button {
                        manager.dispatch(.configureServers(
                            serverAddress: normalized(serverAddress),
                            esploraAddress: normalized(esploraAddress),
                            lnurlServerAddress: normalized(lnurlServerAddress)
                        ))
                    } label: {
                        Label("Save", systemImage: "checkmark")
                            .frame(maxWidth: .infinity)
                    }
                    .buttonStyle(PrimaryButtonStyle(color: rebelBlue))
                    .disabled(!canSave || !hasChanges)
                }
            }
            .padding(16)
        }
        .navigationTitle("Servers")
        .scrollContentBackground(.hidden)
        .background(pageBackground)
        .foregroundStyle(primaryText)
        .onAppear {
            serverAddress = manager.state.wallet.serverAddress
            esploraAddress = manager.state.wallet.esploraAddress
            lnurlServerAddress = manager.state.wallet.lnurlServerAddress
        }
    }

    private func normalized(_ value: String) -> String {
        value.trimmingCharacters(in: .whitespacesAndNewlines)
            .trimmingCharacters(in: CharacterSet(charactersIn: "/"))
    }

}

struct ServerTextField: View {
    let title: String
    @Binding var text: String

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text(title)
                .font(.caption.bold())
                .foregroundStyle(mutedText)
            TextField(title, text: $text)
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
                .keyboardType(.URL)
                .font(.body.monospaced())
                .foregroundStyle(primaryText)
        }
        .padding(.horizontal, 14)
        .padding(.vertical, 12)
    }
}
