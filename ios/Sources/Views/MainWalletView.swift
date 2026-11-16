import SwiftUI

struct MainWalletView: View {
    @Bindable var manager: AppManager
    @Binding var balanceDisplayMode: BalanceDisplayMode
    @State private var displayedPanel: WalletPanel = .tab(.home)
    @State private var panelMovesForward = true
    @Environment(\.walletSidebarVisible) private var sidebarVisible

    private var showsFab: Bool {
        // The sidebar has send/receive/scan, so the FAB would duplicate it.
        displayedPanel.showsFab && !sidebarVisible
    }

    private var selectedTab: MainTab {
        manager.state.router.selectedTab
    }

    private var panelTransition: AnyTransition {
        return .asymmetric(
            insertion: .move(edge: panelMovesForward ? .trailing : .leading).combined(with: .opacity),
            removal: .move(edge: panelMovesForward ? .leading : .trailing).combined(with: .opacity)
        )
    }

    var body: some View {
        ZStack(alignment: .bottomTrailing) {
            Group {
                switch displayedPanel {
                case .tab(let tab):
                    switch tab {
                    case .home:
                        HomeView(manager: manager, balanceDisplayMode: $balanceDisplayMode, openProfile: openProfile)
                    case .activity:
                        ActivityView(manager: manager)
                    case .contacts:
                        ContactsView(manager: manager)
                    case .settings:
                        SettingsView(manager: manager)
                    }
                case .profile:
                    ProfileView(manager: manager, close: closeProfile)
                }
            }
            .id(displayedPanel)
            .transition(panelTransition)

            if showsFab {
                MutinyFab(manager: manager)
            }
        }
        .background(pageBackground.ignoresSafeArea())
        .onAppear {
            displayedPanel = .tab(selectedTab)
        }
        .onChange(of: selectedTab) { _, new in
            let newPanel = WalletPanel.tab(new)
            guard newPanel != displayedPanel else { return }
            panelMovesForward = newPanel.order >= displayedPanel.order
            withAnimation(.spring(response: 0.34, dampingFraction: 0.86)) {
                displayedPanel = newPanel
            }
        }
    }

    private func openProfile() {
        guard displayedPanel != .profile else { return }
        panelMovesForward = false
        withAnimation(.spring(response: 0.34, dampingFraction: 0.86)) {
            displayedPanel = .profile
        }
    }

    private func closeProfile() {
        panelMovesForward = true
        withAnimation(.spring(response: 0.34, dampingFraction: 0.86)) {
            displayedPanel = .tab(selectedTab)
        }
    }
}

struct HomeView: View {
    @Bindable var manager: AppManager
    @Binding var balanceDisplayMode: BalanceDisplayMode
    let openProfile: () -> Void
    @State private var selectedActivityId: String?
    @State private var pullDistance: CGFloat = 0
    @State private var refreshingActivity = false
    @State private var crossedRefreshThreshold = false

    private let refreshThreshold: CGFloat = 86

    private var selectedActivity: ActivityItem? {
        manager.state.activity.first { $0.id == selectedActivityId }
    }

    private var refreshProgress: CGFloat {
        min(1, pullDistance / refreshThreshold)
    }

    private var refreshIndicatorHeight: CGFloat {
        refreshingActivity ? 54 : min(54, pullDistance * 0.72)
    }

    @Environment(\.walletSidebarVisible) private var sidebarVisible

    var body: some View {
        Group {
            if sidebarVisible {
                activityPane
            } else {
                stackedLayout
            }
        }
        .foregroundStyle(primaryText)
        .background(pageBackground)
        .activityPreviewSheet(item: selectedActivity, selectedActivityId: $selectedActivityId, manager: manager)
    }

    private var stackedLayout: some View {
        VStack(spacing: 16) {
            WalletHeader(
                manager: manager,
                balanceDisplayMode: $balanceDisplayMode,
                openProfile: openProfile
            )
                .padding(.horizontal, 16)
                .padding(.top, 14)
                .readableColumn()

            activityList(
                bottomPadding: 88,
                emptyHint: "Use the plus button to send, receive, or scan."
            )
        }
    }

    /// Content pane beside the wallet sidebar on wide displays.
    private var activityPane: some View {
        VStack(alignment: .leading, spacing: 4) {
            Text("Activity")
                .font(.title2.bold())
                .padding(.horizontal, 16)
                .padding(.top, 14)

            activityList(
                bottomPadding: 24,
                emptyHint: "Send, receive, or scan to get started."
            )
        }
    }

    private func activityList(bottomPadding: CGFloat, emptyHint: String) -> some View {
        ScrollView {
            Color.clear
                .frame(height: refreshIndicatorHeight)

            VStack(alignment: .leading, spacing: 0) {
                if manager.state.activity.isEmpty {
                    MutinyEmptyHome(hint: emptyHint)
                } else {
                    ForEach(manager.state.activity, id: \.id) { item in
                        Button {
                            selectedActivityId = item.id
                        } label: {
                            ActivityRow(
                                item: item,
                                balanceDisplayMode: balanceDisplayMode,
                                imageNormalizer: manager.rust
                            )
                        }
                        .buttonStyle(.plain)
                        Divider().overlay(borderColor)
                    }
                }
            }
            .padding(.bottom, bottomPadding)
            .padding(.horizontal, 16)
            .readableColumn()
        }
        .scrollBounceBehavior(.always, axes: .vertical)
        .overlay(alignment: .top) {
            HomeActivityRefreshIndicator(
                progress: refreshProgress,
                refreshing: refreshingActivity
            )
            .frame(height: refreshIndicatorHeight)
            .opacity(refreshingActivity || refreshProgress > 0.04 ? 1 : 0)
            .allowsHitTesting(false)
        }
        .simultaneousGesture(
            DragGesture(minimumDistance: 1)
                .onChanged { value in
                    guard !refreshingActivity else { return }
                    pullDistance = max(0, value.translation.height)
                    if !crossedRefreshThreshold && pullDistance >= refreshThreshold {
                        crossedRefreshThreshold = true
                        manager.requestHaptic(.impactMedium)
                    } else if crossedRefreshThreshold && pullDistance < refreshThreshold * 0.72 {
                        crossedRefreshThreshold = false
                    }
                }
                .onEnded { value in
                    guard !refreshingActivity else { return }
                    let distance = max(0, value.translation.height)
                    if distance >= refreshThreshold {
                        startActivityRefresh()
                    } else {
                        crossedRefreshThreshold = false
                        withAnimation(.spring(response: 0.3, dampingFraction: 0.82)) {
                            pullDistance = 0
                        }
                    }
                }
        )
        .onChange(of: manager.state.router.selectedTab) { _, _ in
            if !refreshingActivity {
                pullDistance = 0
            }
        }
    }

    private func startActivityRefresh() {
        refreshingActivity = true
        Task {
            await manager.syncWalletForRefresh()
            await MainActor.run {
                withAnimation(.spring(response: 0.34, dampingFraction: 0.82)) {
                    refreshingActivity = false
                    crossedRefreshThreshold = false
                    pullDistance = 0
                }
            }
        }
    }
}

private struct HomeActivityRefreshIndicator: View {
    let progress: CGFloat
    let refreshing: Bool
    @Environment(\.walletAccent) private var walletAccent

    private var normalizedProgress: CGFloat {
        min(1, max(0, progress))
    }

    var body: some View {
        TimelineView(.animation) { context in
            let rotation = refreshing
                ? context.date.timeIntervalSinceReferenceDate.truncatingRemainder(dividingBy: 1.1) / 1.1 * 360
                : Double(normalizedProgress) * 220

            ZStack {
                Circle()
                    .fill(raisedSurface)

                Circle()
                    .trim(from: 0, to: refreshing ? 0.34 : 0.16 + normalizedProgress * 0.66)
                    .stroke(
                        AngularGradient(
                            colors: [walletAccent, rebelBlue, rebelGreen, walletAccent],
                            center: .center
                        ),
                        style: StrokeStyle(lineWidth: 3.5, lineCap: .round)
                    )
                    .rotationEffect(.degrees(rotation))

                Image(systemName: refreshing ? "bolt.fill" : "arrow.down")
                    .font(.system(size: 15, weight: .bold))
                    .foregroundStyle(refreshing ? walletAccent : primaryText)
                    .rotationEffect(.degrees(refreshing ? 0 : normalizedProgress * 180))
                    .scaleEffect(0.78 + normalizedProgress * 0.22)
            }
            .frame(width: 34, height: 34)
            .frame(maxWidth: .infinity)
            .scaleEffect(refreshing ? 1 : 0.78 + normalizedProgress * 0.22)
            .animation(.spring(response: 0.28, dampingFraction: 0.78), value: normalizedProgress)
            .animation(.spring(response: 0.28, dampingFraction: 0.78), value: refreshing)
        }
    }
}

struct WalletHeader: View {
    @Bindable var manager: AppManager
    @Binding var balanceDisplayMode: BalanceDisplayMode
    let openProfile: () -> Void

    private var pendingRefreshText: String {
        switch balanceDisplayMode {
        case .sats:
            return manager.state.wallet.pendingRefreshDisplay
        case .currency:
            return manager.state.wallet.pendingRefreshFiatDisplay
                ?? manager.state.wallet.pendingRefreshDisplay
        case .privacy:
            return "****"
        }
    }

    var body: some View {
        VStack(spacing: 8) {
            HStack(spacing: 14) {
                Button {
                    openProfile()
                } label: {
                    ProfileAvatar(url: manager.state.nostr.pictureDisplayUrl, size: 48, imageNormalizer: manager.rust)
                }
                .buttonStyle(.plain)
                Spacer(minLength: 8)
                MutinyBalanceButton(wallet: manager.state.wallet, displayMode: $balanceDisplayMode) {
                    manager.requestHaptic(.selection)
                }
                    .frame(maxWidth: .infinity)
                Button {
                    withAnimation(.spring(response: 0.34, dampingFraction: 0.86)) {
                        manager.dispatch(.selectTab(tab: .settings))
                    }
                } label: {
                    MutinyCircle(size: 48) {
                        RebelMark(size: 28)
                    }
                }
            }
            if manager.state.wallet.pendingRefreshSat > 0 {
                HStack(spacing: 6) {
                    Image(systemName: "arrow.triangle.2.circlepath")
                    Text("\(pendingRefreshText) refreshing")
                }
                .font(.caption)
                .foregroundStyle(mutedText)
                .frame(maxWidth: .infinity)
            }
        }
    }
}

private extension MainTab {
    var order: Int {
        switch self {
        case .home:
            return 0
        case .activity:
            return 1
        case .contacts:
            return 2
        case .settings:
            return 3
        }
    }
}

private enum WalletPanel: Equatable, Hashable {
    case profile
    case tab(MainTab)

    var order: Int {
        switch self {
        case .profile:
            return -1
        case .tab(let tab):
            return tab.order
        }
    }

    var showsFab: Bool {
        switch self {
        case .profile:
            return false
        case .tab:
            return true
        }
    }
}

struct MutinyBalanceButton: View {
    let wallet: WalletState
    @Binding var displayMode: BalanceDisplayMode
    let onCycle: () -> Void

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
        onCycle()
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

enum BalanceDisplayMode {
    case sats
    case currency
    case privacy
}

struct MutinyEmptyHome: View {
    var hint = "Use the plus button to send, receive, or scan."

    var body: some View {
        VStack(spacing: 14) {
            Image(systemName: "bolt.circle")
                .font(.system(size: 42, weight: .light))
                .foregroundStyle(mutedText)
            Text("No payments yet")
                .font(.headline)
            Text(hint)
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
    @Environment(\.walletAccent) private var walletAccent

    var body: some View {
        VStack(alignment: .trailing, spacing: 14) {
            if open {
                VStack(alignment: .leading, spacing: 0) {
                    FabMenuButton(title: "Send", icon: "arrow.up.right") {
                        manager.requestHaptic(.impactLight)
                        open = false
                        manager.dispatch(.pushScreen(screen: .send))
                    }
                    Divider().overlay(borderColor)
                    FabMenuButton(title: "Receive", icon: "arrow.down.left") {
                        manager.requestHaptic(.impactLight)
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
                manager.requestHaptic(.selection)
                open.toggle()
            } label: {
                MutinyCircle(size: 64, color: walletAccent) {
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
            WalletStatusDetails(wallet: wallet)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(18)
        .foregroundStyle(primaryText)
        .background(surfaceBackground, in: RoundedRectangle(cornerRadius: 8))
        .overlay(RoundedRectangle(cornerRadius: 8).stroke(borderColor))
    }
}

struct WalletStatusDetails: View {
    let wallet: WalletState

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            HStack {
                StatPill(title: "Claimable", value: wallet.pendingReceiveDisplay, caption: wallet.pendingReceiveFiatDisplay)
                StatPill(title: "Sending", value: wallet.pendingSendDisplay, caption: wallet.pendingSendFiatDisplay)
            }
            if wallet.stuckReceiveSat > 0 {
                HStack(spacing: 6) {
                    Image(systemName: "exclamationmark.triangle.fill")
                    Text("\(wallet.stuckReceiveDisplay) stuck — a Lightning receive could not be claimed")
                }
                .font(.caption)
                .foregroundStyle(.orange)
                .frame(maxWidth: .infinity, alignment: .leading)
            }
            if let syncError = wallet.syncError {
                Label(syncError, systemImage: "arrow.trianglehead.2.clockwise.rotate.90")
                    .font(.caption)
                    .foregroundStyle(.orange)
            }
            if let lastSync = wallet.lastSync {
                Text("Last sync \(lastSync)")
                    .font(.caption)
                    .foregroundStyle(mutedText)
            }
        }
    }
}

/// Persistent left pane of the wide layout: profile, balance, wallet status
/// and payment actions. Its buttons swap what the content pane shows rather
/// than stacking screens.
struct HomeSidebar: View {
    @Bindable var manager: AppManager
    @Binding var balanceDisplayMode: BalanceDisplayMode

    private var name: String {
        manager.state.nostr.name.isEmpty ? "Rebel" : manager.state.nostr.name
    }

    private var lightningAddress: String {
        manager.state.nostr.lud16.trimmingCharacters(in: .whitespacesAndNewlines)
    }

    private var pendingRefreshText: String {
        switch balanceDisplayMode {
        case .sats:
            return manager.state.wallet.pendingRefreshDisplay
        case .currency:
            return manager.state.wallet.pendingRefreshFiatDisplay
                ?? manager.state.wallet.pendingRefreshDisplay
        case .privacy:
            return "****"
        }
    }

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 18) {
                HStack(spacing: 12) {
                    Button {
                        show(.profile)
                    } label: {
                        HStack(spacing: 12) {
                            ProfileAvatar(
                                url: manager.state.nostr.pictureDisplayUrl,
                                size: 56,
                                imageNormalizer: manager.rust
                            )
                            VStack(alignment: .leading, spacing: 3) {
                                Text(name)
                                    .font(.headline)
                                    .lineLimit(1)
                                Text(lightningAddress.isEmpty
                                     ? "View profile"
                                     : truncateMiddle(lightningAddress, maxLength: 26, prefixCount: 12))
                                    .font(.caption)
                                    .foregroundStyle(lightningAddress.isEmpty ? mutedText : rebelGreen)
                                    .lineLimit(1)
                            }
                        }
                        .contentShape(Rectangle())
                    }
                    .buttonStyle(.plain)
                    .accessibilityLabel("Open profile")

                    Spacer(minLength: 8)

                    Button {
                        manager.dispatch(.updateScreenStack(stack: []))
                        withAnimation(.spring(response: 0.34, dampingFraction: 0.86)) {
                            manager.dispatch(.selectTab(tab: .settings))
                        }
                    } label: {
                        MutinyCircle(size: 44) {
                            RebelMark(size: 26)
                        }
                    }
                    .accessibilityLabel("Settings")
                }

                VStack(alignment: .leading, spacing: 10) {
                    MutinyBalanceButton(wallet: manager.state.wallet, displayMode: $balanceDisplayMode) {
                        manager.requestHaptic(.selection)
                    }
                    if manager.state.wallet.pendingRefreshSat > 0 {
                        HStack(spacing: 6) {
                            Image(systemName: "arrow.triangle.2.circlepath")
                            Text("\(pendingRefreshText) refreshing")
                        }
                        .font(.caption)
                        .foregroundStyle(mutedText)
                    }
                    WalletStatusDetails(wallet: manager.state.wallet)
                }

                HStack(spacing: 10) {
                    HomeActionButton(title: "Send", icon: "arrow.up.right") {
                        manager.requestHaptic(.impactLight)
                        show(.send)
                    }
                    HomeActionButton(title: "Receive", icon: "arrow.down.left") {
                        manager.requestHaptic(.impactLight)
                        show(.receive)
                    }
                    HomeActionButton(title: "Scan", icon: "qrcode.viewfinder") {
                        manager.dispatch(.requestQrScan)
                    }
                }
            }
            .padding(16)
        }
        .scrollBounceBehavior(.basedOnSize)
    }

    /// Replaces whatever the content pane shows with `screen`.
    private func show(_ screen: Screen) {
        guard manager.state.router.screenStack != [screen] else { return }
        manager.dispatch(.updateScreenStack(stack: []))
        manager.dispatch(.pushScreen(screen: screen))
    }
}

struct HomeActionButton: View {
    let title: String
    let icon: String
    let action: () -> Void
    @Environment(\.walletAccent) private var walletAccent

    var body: some View {
        Button(action: action) {
            VStack(spacing: 8) {
                Image(systemName: icon)
                    .font(.title3.weight(.semibold))
                    .foregroundStyle(walletAccent)
                Text(title)
                    .font(.subheadline.weight(.medium))
            }
            .foregroundStyle(primaryText)
            .frame(maxWidth: .infinity)
            .padding(.vertical, 14)
            .background(raisedSurface, in: RoundedRectangle(cornerRadius: 12))
            .overlay(RoundedRectangle(cornerRadius: 12).stroke(borderColor))
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
    }
}
