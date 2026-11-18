import SwiftUI
import PhotosUI
import UIKit

struct ContentView: View {
    @Bindable var manager: AppManager
    @State private var navPath: [Screen] = []
    @State private var selectedCapabilityPhoto: PhotosPickerItem?
    @State private var balanceDisplayMode: BalanceDisplayMode = .sats
    @State private var hasVerticalBar = false
    @Environment(\.walletAccent) private var walletAccent
    @Environment(\.horizontalSizeClass) private var horizontalSizeClass

    /// On wide displays (unfolded iPhone Duo, iPad) the wallet sidebar stays
    /// on the left and every screen opens in the pane beside it.
    private var showsSidebar: Bool {
        guard horizontalSizeClass == .regular, !manager.state.showLaunchSplash else { return false }
        if case .ready = manager.state.setup { return true }
        return false
    }

    var body: some View {
        GeometryReader { proxy in
            HStack(spacing: 0) {
                if showsSidebar {
                    HomeSidebar(manager: manager, balanceDisplayMode: $balanceDisplayMode)
                        .frame(width: sidebarWidth(in: proxy))
                    Rectangle()
                        .fill(borderColor)
                        .frame(width: 1)
                        .ignoresSafeArea(edges: .vertical)
                }
                NavigationStack(path: $navPath) {
                    root
                        .navigationDestination(for: Screen.self) { screen in
                            screenView(for: screen)
                                .toolbar {
                                    // Beside the sidebar every pane screen
                                    // keeps the wallet controls in the rail.
                                    if showsSidebar && hasVerticalBar {
                                        WalletRailToolbar(manager: manager)
                                    }
                                }
                        }
                }
            }
        }
        .background(VerticalBarReader { hasVerticalBar = $0 })
        .environment(\.walletSidebarVisible, showsSidebar)
        .environment(\.walletHasVerticalBar, hasVerticalBar)
        .tint(walletAccent)
        .foregroundStyle(primaryText)
        .background(pageBackground.ignoresSafeArea())
        .onChange(of: manager.state.router.screenStack) { _, new in
            navPath = new
        }
        .onChange(of: navPath) { old, new in
            guard new != manager.state.router.screenStack else { return }
            manager.dispatch(.updateScreenStack(stack: new))
        }
        .overlay(alignment: .bottom) {
            if let toast = manager.state.toast {
                ToastView(text: toast) {
                    manager.dispatch(.clearToast)
                }
            }
        }
        .sheet(isPresented: Binding(
            get: { manager.state.capabilityRequest?.kind == .qrScan },
            set: { if !$0 { manager.dispatch(.cancelCapabilityRequest) } }
        )) {
            QRScannerSheet { value in
                manager.dispatch(.completeQrScan(value: value))
            }
        }
        .photosPicker(isPresented: Binding(
            get: { manager.state.capabilityRequest?.kind == .photoPick },
            set: { if !$0 { manager.dispatch(.cancelCapabilityRequest) } }
        ), selection: $selectedCapabilityPhoto, matching: .images)
        .onChange(of: selectedCapabilityPhoto) { _, item in
            guard let item else { return }
            Task {
                let data = try? await item.loadTransferable(type: Data.self)
                manager.dispatch(.completePhotoPick(imageBase64: data?.base64EncodedString()))
                selectedCapabilityPhoto = nil
            }
        }
        .onChange(of: manager.state.capabilityRequest) { _, request in
            guard request?.kind == .clipboardRead else { return }
            manager.dispatch(.completeClipboardRead(value: UIPasteboard.general.string))
        }
    }

    /// Puts the divider on the screen's vertical center line, which is the
    /// hinge on an unfolded iPhone Duo.
    private func sidebarWidth(in proxy: GeometryProxy) -> CGFloat {
        let insets = proxy.safeAreaInsets
        let screenWidth = proxy.size.width + insets.leading + insets.trailing
        return max(0, screenWidth / 2 - insets.leading)
    }

    @ViewBuilder
    private var root: some View {
        if manager.state.showLaunchSplash {
            LaunchSplashView()
        } else {
            switch manager.state.setup {
            case .ready:
                MainWalletView(manager: manager, balanceDisplayMode: $balanceDisplayMode)
            case .needsSetup, .error:
                SetupView(manager: manager)
            }
        }
    }

    @ViewBuilder
    private func screenView(for screen: Screen) -> some View {
        switch screen {
        case .setup:
            SetupView(manager: manager)
        case .home:
            MainWalletView(manager: manager, balanceDisplayMode: $balanceDisplayMode)
        case .send:
            SendView(manager: manager)
        case .receive:
            ReceiveView(manager: manager)
        case .profile:
            ProfileView(manager: manager)
        case .lightningAddress:
            LightningAddressView(manager: manager)
        case .backup:
            BackupView(manager: manager)
        case .restore:
            RestoreWalletView(manager: manager)
        case .network:
            NetworkView(manager: manager)
        case .currency:
            CurrencyView(manager: manager)
        case .contactDetail(let contactId):
            ContactDetailView(manager: manager, contactId: contactId)
        }
    }
}
