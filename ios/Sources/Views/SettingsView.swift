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
                }

                SettingsCard(title: "Recovery") {
                    SettingsRow(title: "Unilateral exit", caption: "Review Ark exit options", accent: rebelGreen) {
                        manager.dispatch(.pushScreen(screen: .unilateralExit))
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

struct UnilateralExitView: View {
    @Bindable var manager: AppManager
    @State private var showingExitConfirmation = false

    private var canStartExit: Bool {
        manager.state.wallet.balanceSat > 0 && !manager.state.busy.startingUnilateralExit
    }

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 18) {
                VStack(alignment: .leading, spacing: 12) {
                    HStack(spacing: 12) {
                        ZStack {
                            Circle()
                                .fill(rebelGreen.opacity(0.18))
                            Image(systemName: "lock.open.trianglebadge.exclamationmark")
                                .font(.system(size: 22, weight: .semibold))
                                .foregroundStyle(rebelGreen)
                        }
                        .frame(width: 52, height: 52)

                        VStack(alignment: .leading, spacing: 4) {
                            Text("Unilateral exit")
                                .font(.title3.bold())
                            Text("Recover Ark funds without relying on the server.")
                                .font(.subheadline)
                                .foregroundStyle(mutedText)
                        }
                    }

                    Text("A unilateral exit lets you settle Ark funds on Bitcoin if the Ark server is unavailable or uncooperative. Starting an exit marks eligible VTXOs and begins moving them on-chain.")
                        .font(.body)
                        .foregroundStyle(primaryText)
                }
                .frame(maxWidth: .infinity, alignment: .leading)

                SettingsCard(title: "What to know") {
                    UnilateralExitInfoRow(
                        icon: "clock",
                        title: "Timing matters",
                        caption: "Ark coins have expiry and refresh windows. Keep the wallet online periodically so VTXOs can be refreshed before they expire."
                    )
                    SettingsDivider()
                    UnilateralExitInfoRow(
                        icon: "bitcoinsign.circle",
                        title: "Bitcoin fees apply",
                        caption: "An exit settles on-chain, so confirmation time and miner fees depend on the Bitcoin network."
                    )
                    SettingsDivider()
                    UnilateralExitInfoRow(
                        icon: "key",
                        title: "Seed required",
                        caption: "Your recovery phrase is still the root backup. Store it offline and never share it."
                    )
                }

                SettingsCard(title: "Current status") {
                    StatusLine(title: "Network", value: manager.state.wallet.networkName)
                    SettingsDivider()
                    StatusLine(title: "Balance", value: manager.state.wallet.balanceDisplay)
                    SettingsDivider()
                    StatusLine(
                        title: "VTXO maintenance",
                        value: manager.state.busy.maintainingVtxos ? "Refreshing" : "Idle"
                    )
                }

                Button {
                    showingExitConfirmation = true
                } label: {
                    Label(
                        manager.state.busy.startingUnilateralExit ? "Starting exit" : "Start exit",
                        systemImage: "rectangle.portrait.and.arrow.right"
                    )
                    .frame(maxWidth: .infinity)
                }
                .buttonStyle(PrimaryButtonStyle(color: rebelRed))
                .disabled(!canStartExit)

                Button {
                    manager.dispatch(.maintainVtxos)
                } label: {
                    Label(
                        manager.state.busy.maintainingVtxos ? "Refreshing VTXOs" : "Refresh VTXOs",
                        systemImage: "arrow.clockwise"
                    )
                    .frame(maxWidth: .infinity)
                }
                .buttonStyle(PrimaryButtonStyle(color: rebelBlue))
                .disabled(manager.state.busy.maintainingVtxos)

                Text("Final claim and fee-bump controls still need on-chain wallet support.")
                    .font(.caption)
                    .foregroundStyle(mutedText)
                    .frame(maxWidth: .infinity, alignment: .center)
                    .padding(.bottom, 24)
            }
            .padding(16)
        }
        .navigationTitle("Unilateral Exit")
        .scrollContentBackground(.hidden)
        .background(pageBackground)
        .foregroundStyle(primaryText)
        .confirmationDialog(
            "Start unilateral exit?",
            isPresented: $showingExitConfirmation,
            titleVisibility: .visible
        ) {
            Button("Start exit", role: .destructive) {
                manager.dispatch(.startUnilateralExit)
            }
            Button("Cancel", role: .cancel) {}
        } message: {
            Text("This marks eligible Ark VTXOs for unilateral exit and begins moving them on-chain. Use this only if you need to recover without the Ark server.")
        }
    }
}

private struct UnilateralExitInfoRow: View {
    let icon: String
    let title: String
    let caption: String

    var body: some View {
        HStack(alignment: .top, spacing: 12) {
            Image(systemName: icon)
                .font(.system(size: 18, weight: .semibold))
                .foregroundStyle(rebelGreen)
                .frame(width: 28, height: 28)

            VStack(alignment: .leading, spacing: 4) {
                Text(title)
                    .font(.body)
                    .foregroundStyle(primaryText)
                Text(caption)
                    .font(.caption)
                    .foregroundStyle(mutedText)
                    .fixedSize(horizontal: false, vertical: true)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
        }
        .padding(.horizontal, 14)
        .padding(.vertical, 12)
    }
}

private struct StatusLine: View {
    let title: String
    let value: String

    var body: some View {
        HStack(spacing: 12) {
            Text(title)
                .font(.body)
                .foregroundStyle(primaryText)
            Spacer()
            Text(value)
                .font(.caption.bold())
                .foregroundStyle(mutedText)
                .multilineTextAlignment(.trailing)
        }
        .padding(.horizontal, 14)
        .padding(.vertical, 12)
    }
}

struct NetworkView: View {
    @Bindable var manager: AppManager
    @State private var selectedNetwork: WalletNetwork = .signet

    private var hasChanges: Bool {
        selectedNetwork != manager.state.wallet.network
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

                if manager.state.busy.openingWallet {
                    HStack(spacing: 10) {
                        ProgressView()
                        Text("Reconnecting wallet")
                            .font(.footnote)
                            .foregroundStyle(mutedText)
                    }
                }

                Button {
                    manager.dispatch(.selectNetwork(network: selectedNetwork))
                    manager.dispatch(.popScreen)
                } label: {
                    Label("Select network", systemImage: "checkmark")
                        .frame(maxWidth: .infinity)
                }
                .buttonStyle(PrimaryButtonStyle(color: rebelBlue))
                .disabled(!hasChanges || manager.state.busy.openingWallet)
            }
            .padding(16)
        }
        .navigationTitle("Network")
        .scrollContentBackground(.hidden)
        .background(pageBackground)
        .foregroundStyle(primaryText)
        .onAppear {
            selectedNetwork = manager.state.wallet.network
        }
    }
}
