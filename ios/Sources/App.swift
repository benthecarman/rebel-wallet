import SwiftUI

@main
struct RebelWalletApp: App {
    @State private var manager = AppManager()
    @State private var easterEgg = WalletEasterEgg()
    @Environment(\.scenePhase) private var scenePhase

    var body: some Scene {
        WindowGroup {
            ContentView(manager: manager)
                .environment(\.walletAccent, easterEgg.accentColor)
                .environment(\.walletUsesDellLogo, easterEgg.isDellMode)
                .preferredColorScheme(.dark)
                .onAppear {
                    easterEgg.start()
                }
                .onDisappear {
                    easterEgg.stop()
                }
                .onChange(of: scenePhase) { _, phase in
                    switch phase {
                    case .active:
                        manager.dispatch(.maintainVtxos)
                        // Re-attempt claiming an in-flight Lightning receive in case
                        // the payment landed while the app was suspended.
                        manager.dispatch(.resumeReceiveMonitor)
                        manager.endReceiveBackgroundTask()
                    case .background:
                        // Keep the core running briefly so an in-flight Lightning
                        // receive can still be claimed while backgrounded.
                        manager.beginReceiveBackgroundTaskIfNeeded()
                    default:
                        break
                    }
                }
        }
    }
}
