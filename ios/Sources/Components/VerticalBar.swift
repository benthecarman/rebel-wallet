import SwiftUI
import UIKit

/// Reports whether the system places a vertical bar (the side rail on
/// iPhone Duo, folded or unfolded) beside this view, and keeps reporting
/// as the fold state or orientation changes.
struct VerticalBarReader: UIViewRepresentable {
    let onChange: (Bool) -> Void

    func makeUIView(context: Context) -> ReaderView {
        ReaderView(onChange: onChange)
    }

    func updateUIView(_ uiView: ReaderView, context: Context) {
        uiView.onChange = onChange
    }

    final class ReaderView: UIView {
        var onChange: (Bool) -> Void
        private var lastReported: Bool?

        init(onChange: @escaping (Bool) -> Void) {
            self.onChange = onChange
            super.init(frame: .zero)
            isUserInteractionEnabled = false
            if #available(iOS 27.1, *) {
                registerForTraitChanges(UITraitCollection.systemTraitsAffectingVerticalBarEdge) {
                    (view: ReaderView, _: UITraitCollection) in
                    view.report()
                }
            }
        }

        @available(*, unavailable)
        required init?(coder: NSCoder) {
            fatalError("init(coder:) has not been implemented")
        }

        override func didMoveToWindow() {
            super.didMoveToWindow()
            report()
        }

        private func report() {
            guard window != nil else { return }
            var hasBar = false
            if #available(iOS 27.1, *) {
                hasBar = traitCollection.verticalBarEdge != .unspecified
            }
            guard hasBar != lastReported else { return }
            lastReported = hasBar
            // Defer so SwiftUI state isn't mutated mid-update.
            DispatchQueue.main.async { [onChange] in
                onChange(hasBar)
            }
        }
    }
}

/// Wallet controls hosted in the vertical bar instead of floating over
/// content: settings (or back to the wallet) at the top and the payment
/// button that replaces the floating action button at the bottom.
struct WalletRailToolbar: ToolbarContent {
    @Bindable var manager: AppManager
    var showsSettings = true
    var showsBackToWallet = false
    var showsPayments = true

    var body: some ToolbarContent {
        if showsBackToWallet {
            ToolbarItem(placement: .topBarTrailing) {
                Button {
                    manager.dispatch(.selectTab(tab: .home))
                } label: {
                    Label("Back to wallet", systemImage: "chevron.left")
                }
            }
        } else if showsSettings {
            ToolbarItem(placement: .topBarTrailing) {
                Button {
                    manager.dispatch(.updateScreenStack(stack: []))
                    withAnimation(.spring(response: 0.34, dampingFraction: 0.86)) {
                        manager.dispatch(.selectTab(tab: .settings))
                    }
                } label: {
                    RebelMark(size: 26)
                }
                .accessibilityLabel("Settings")
            }
        }

        if showsPayments {
            ToolbarItem(placement: .bottomBar) {
                RailPaymentButton(manager: manager)
            }
        }
    }
}

/// The floating action button's rail counterpart: a prominent accent button
/// that pops over the same Send / Receive / Scan list. A plain `Menu` can't
/// take the prominent style in the toolbar, so this pairs a `Button` with a
/// popover instead.
private struct RailPaymentButton: View {
    @Bindable var manager: AppManager
    @State private var open = false
    @Environment(\.walletAccent) private var walletAccent

    var body: some View {
        styled(
            Button {
                manager.requestHaptic(.selection)
                open.toggle()
            } label: {
                Label("Send, receive, or scan", systemImage: "plus")
            }
        )
        .tint(walletAccent)
        .popover(isPresented: $open) {
            VStack(alignment: .leading, spacing: 0) {
                FabMenuButton(title: "Send", icon: "arrow.up.right") {
                    manager.requestHaptic(.impactLight)
                    open = false
                    openPaymentScreen(.send)
                }
                Divider().overlay(borderColor)
                FabMenuButton(title: "Receive", icon: "arrow.down.left") {
                    manager.requestHaptic(.impactLight)
                    open = false
                    openPaymentScreen(.receive)
                }
                Divider().overlay(borderColor)
                FabMenuButton(title: "Scan", icon: "qrcode.viewfinder") {
                    open = false
                    manager.dispatch(.requestQrScan)
                }
            }
            .padding(.horizontal, 8)
            .fixedSize()
            .presentationCompactAdaptation(.popover)
        }
    }

    @ViewBuilder
    private func styled(_ button: some View) -> some View {
        if #available(iOS 26.0, *) {
            button.buttonStyle(.glassProminent)
        } else {
            button.buttonStyle(.borderedProminent)
        }
    }

    /// Opens Send or Receive as the only screen on the stack, so tapping it
    /// from another payment screen swaps rather than stacks.
    private func openPaymentScreen(_ screen: Screen) {
        guard manager.state.router.screenStack != [screen] else { return }
        manager.dispatch(.updateScreenStack(stack: []))
        manager.dispatch(.pushScreen(screen: screen))
    }
}
