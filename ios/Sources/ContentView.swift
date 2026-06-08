import SwiftUI
import PhotosUI
import UIKit

struct ContentView: View {
    @Bindable var manager: AppManager
    @State private var navPath: [Screen] = []
    @State private var selectedCapabilityPhoto: PhotosPickerItem?
    @Environment(\.walletAccent) private var walletAccent

    var body: some View {
        NavigationStack(path: $navPath) {
            root
                .navigationDestination(for: Screen.self) { screen in
                    screenView(for: screen)
                }
        }
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

    @ViewBuilder
    private var root: some View {
        if manager.state.showLaunchSplash {
            LaunchSplashView()
        } else {
            switch manager.state.setup {
            case .ready:
                MainWalletView(manager: manager)
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
            MainWalletView(manager: manager)
        case .send:
            SendView(manager: manager)
        case .receive:
            ReceiveView(manager: manager)
        case .profile:
            ProfileView(manager: manager)
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
