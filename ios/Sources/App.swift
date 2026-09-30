import SwiftUI

@main
struct RebelWalletApp: App {
    @State private var manager: AppManager
    @State private var easterEgg = WalletEasterEgg()
    @Environment(\.scenePhase) private var scenePhase

    init() {
        let manager = AppManager()
        _manager = State(initialValue: manager)
        BackgroundMaintenance.register(manager: manager)
    }

    var body: some Scene {
        WindowGroup {
            ContentView(manager: manager)
                .environment(\.walletAccent, easterEgg.accentColor)
                .environment(\.walletUsesDellLogo, easterEgg.isDellMode)
                .preferredColorScheme(.dark)
                .onAppear {
                    easterEgg.start()
                    manager.dispatch(.foregrounded)
                }
                .onDisappear {
                    easterEgg.stop()
                }
                .onChange(of: scenePhase) { _, phase in
                    switch phase {
                    case .active:
                        manager.dispatch(.foregrounded)
                        // Re-attempt claiming an in-flight Lightning receive in case
                        // the payment landed while the app was suspended.
                        manager.dispatch(.resumeReceiveMonitor)
                        // Sweep for any Lightning receive whose HTLCs arrived while we
                        // were away, independent of the receive screen, so it can't get
                        // stuck in "claimable".
                        manager.dispatch(.claimPendingLightningReceives)
                        manager.endReceiveBackgroundTask()
                    case .background:
                        manager.dispatch(.backgrounded)
                        BackgroundMaintenance.schedule()
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
