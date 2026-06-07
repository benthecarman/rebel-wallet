import SwiftUI

struct MainWalletView: View {
    @Bindable var manager: AppManager

    var body: some View {
        ZStack(alignment: .bottomTrailing) {
            Group {
                switch manager.state.router.selectedTab {
                case .home:
                    HomeView(manager: manager)
                case .activity:
                    ActivityView(manager: manager)
                case .contacts:
                    ContactsView(manager: manager)
                case .settings:
                    SettingsView(manager: manager)
                }
            }
            MutinyFab(manager: manager)
        }
        .background(pageBackground.ignoresSafeArea())
    }
}

struct HomeView: View {
    @Bindable var manager: AppManager
    @State private var selectedActivityId: String?

    private var selectedActivity: ActivityItem? {
        manager.state.activity.first { $0.id == selectedActivityId }
    }

    var body: some View {
        ScrollView {
            VStack(spacing: 16) {
                WalletHeader(manager: manager)

                VStack(alignment: .leading, spacing: 0) {
                    if manager.state.activity.isEmpty {
                        MutinyEmptyHome()
                    } else {
                        ForEach(manager.state.activity, id: \.id) { item in
                            Button {
                                selectedActivityId = item.id
                            } label: {
                                ActivityRow(item: item)
                            }
                            .buttonStyle(.plain)
                            Divider().overlay(borderColor)
                        }
                    }
                }
                .padding(.bottom, 88)
            }
            .padding(.horizontal, 16)
            .padding(.top, 14)
        }
        .foregroundStyle(primaryText)
        .background(pageBackground)
        .activityPreviewSheet(item: selectedActivity, selectedActivityId: $selectedActivityId)
    }
}

struct WalletHeader: View {
    @Bindable var manager: AppManager

    var body: some View {
        VStack(spacing: 8) {
            HStack(spacing: 14) {
                Button {
                    manager.dispatch(.pushScreen(screen: .profile))
                } label: {
                    ProfileAvatar(url: manager.state.nostr.picture, size: 48)
                }
                .buttonStyle(.plain)
                Spacer(minLength: 8)
                MutinyBalanceButton(wallet: manager.state.wallet)
                    .frame(maxWidth: .infinity)
                Button {
                    manager.dispatch(.selectTab(tab: .settings))
                } label: {
                    MutinyCircle(size: 48) {
                        RebelMark(size: 28)
                    }
                }
            }
            if manager.state.wallet.pendingRefreshSat > 0 {
                HStack(spacing: 6) {
                    Image(systemName: "arrow.triangle.2.circlepath")
                    Text("\(manager.state.wallet.pendingRefreshDisplay) refreshing")
                }
                .font(.caption)
                .foregroundStyle(mutedText)
                .frame(maxWidth: .infinity)
            }
        }
    }
}

struct MutinyBalanceButton: View {
    let wallet: WalletState
    @State private var displayMode: BalanceDisplayMode = .sats

    private var canShowCurrency: Bool {
        wallet.priceCurrency != .btc && wallet.balanceFiatDisplay != nil
    }

    private var balanceText: String {
        switch displayMode {
        case .sats:
            wallet.balanceDisplay
        case .currency:
            wallet.balanceFiatDisplay ?? wallet.balanceDisplay
        case .privacy:
            "****"
        }
    }

    var body: some View {
        Button {
            advanceDisplayMode()
        } label: {
            Text(balanceText)
                .font(.system(size: 25, weight: .light, design: .default))
                .lineLimit(1)
                .minimumScaleFactor(0.7)
                .frame(minHeight: 48)
                .frame(maxWidth: .infinity)
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .frame(minHeight: 48)
        .frame(maxWidth: .infinity)
        .padding(.horizontal, 14)
        .background(Color.black, in: RoundedRectangle(cornerRadius: 8))
        .overlay(alignment: .top) {
            RoundedRectangle(cornerRadius: 8)
                .stroke(Color.white.opacity(0.35), lineWidth: 1)
                .mask(alignment: .top) { Rectangle().frame(height: 1) }
        }
        .overlay(alignment: .bottom) {
            RoundedRectangle(cornerRadius: 8)
                .stroke(Color.white.opacity(0.08), lineWidth: 1)
                .mask(alignment: .bottom) { Rectangle().frame(height: 1) }
        }
        .onChange(of: wallet.priceCurrency) { _, _ in
            normalizeDisplayMode()
        }
        .onChange(of: wallet.balanceFiatDisplay) { _, _ in
            normalizeDisplayMode()
        }
    }

    private func advanceDisplayMode() {
        switch displayMode {
        case .sats:
            displayMode = canShowCurrency ? .currency : .privacy
        case .currency:
            displayMode = .privacy
        case .privacy:
            displayMode = .sats
        }
    }

    private func normalizeDisplayMode() {
        if displayMode == .currency && !canShowCurrency {
            displayMode = .sats
        }
    }
}

private enum BalanceDisplayMode {
    case sats
    case currency
    case privacy
}

struct MutinyEmptyHome: View {
    var body: some View {
        VStack(spacing: 14) {
            Image(systemName: "bolt.circle")
                .font(.system(size: 42, weight: .light))
                .foregroundStyle(mutedText)
            Text("No payments yet")
                .font(.headline)
            Text("Use the plus button to send, receive, or scan.")
                .font(.subheadline)
                .foregroundStyle(mutedText)
                .multilineTextAlignment(.center)
        }
        .frame(maxWidth: .infinity)
        .padding(.vertical, 56)
    }
}

struct MutinyFab: View {
    @Bindable var manager: AppManager
    @State private var open = false

    var body: some View {
        VStack(alignment: .trailing, spacing: 14) {
            if open {
                VStack(alignment: .leading, spacing: 0) {
                    FabMenuButton(title: "Send", icon: "arrow.up.right") {
                        open = false
                        manager.dispatch(.pushScreen(screen: .send))
                    }
                    Divider().overlay(borderColor)
                    FabMenuButton(title: "Receive", icon: "arrow.down.left") {
                        open = false
                        manager.dispatch(.pushScreen(screen: .receive))
                    }
                    Divider().overlay(borderColor)
                    FabMenuButton(title: "Scan", icon: "qrcode.viewfinder") {
                        open = false
                        manager.dispatch(.requestQrScan)
                    }
                }
                .padding(.horizontal, 8)
                .fixedSize()
                .background(surfaceBackground.opacity(0.94), in: RoundedRectangle(cornerRadius: 12))
                .overlay(RoundedRectangle(cornerRadius: 12).stroke(borderColor))
            }
            Button {
                open.toggle()
            } label: {
                MutinyCircle(size: 64, color: rebelRed) {
                    Image(systemName: "plus")
                        .font(.system(size: 30, weight: .semibold))
                }
            }
        }
        .padding(.trailing, 24)
        .padding(.bottom, 26)
    }
}

struct FabMenuButton: View {
    let title: String
    let icon: String
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            HStack(spacing: 12) {
                Image(systemName: icon)
                    .frame(width: 24)
                Text(title)
                    .font(.body)
            }
            .foregroundStyle(primaryText)
            .frame(width: 132, alignment: .leading)
            .padding(.vertical, 12)
            .padding(.horizontal, 6)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
    }
}

struct BalancePanel: View {
    let wallet: WalletState

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            Text("Balance")
                .font(.subheadline)
                .foregroundStyle(mutedText)
            Text(wallet.balanceDisplay)
                .font(.system(size: 42, weight: .bold, design: .rounded))
            if let fiatDisplay = wallet.balanceFiatDisplay {
                Text(fiatDisplay)
                    .font(.title3.weight(.semibold))
                    .foregroundStyle(mutedText)
            }
            HStack {
                StatPill(title: "Claimable", value: wallet.pendingReceiveDisplay, caption: wallet.pendingReceiveFiatDisplay)
                StatPill(title: "Sending", value: wallet.pendingSendDisplay, caption: wallet.pendingSendFiatDisplay)
            }
            if let lastSync = wallet.lastSync {
                Text("Last sync \(lastSync)")
                    .font(.caption)
                    .foregroundStyle(mutedText)
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(18)
        .foregroundStyle(primaryText)
        .background(surfaceBackground, in: RoundedRectangle(cornerRadius: 8))
        .overlay(RoundedRectangle(cornerRadius: 8).stroke(borderColor))
    }
}
