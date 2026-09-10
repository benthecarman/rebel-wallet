import SwiftUI

@main
struct RebelWalletApp: App {
    @UIApplicationDelegateAdaptor(RebelWalletAppDelegate.self) private var appDelegate
    @State private var manager: AppManager?
    @State private var pendingOpenURL: URL?
    @State private var easterEgg = WalletEasterEgg()
    @Environment(\.scenePhase) private var scenePhase

    var body: some Scene {
        WindowGroup {
            Group {
                if let manager {
                    ContentView(manager: manager)
                } else {
                    LaunchSplashView()
                }
            }
                .environment(\.walletAccent, easterEgg.accentColor)
                .environment(\.walletUsesDellLogo, easterEgg.isDellMode)
                .preferredColorScheme(.dark)
                .task {
                    await loadManagerIfNeeded()
                }
                .onOpenURL { url in
                    if let manager {
                        manager.nwc.handleOpenURL(url)
                    } else {
                        pendingOpenURL = url
                    }
                }
                .onAppear {
                    easterEgg.start()
                }
                .onDisappear {
                    easterEgg.stop()
                }
                .onChange(of: scenePhase) { _, phase in
                    guard let manager else { return }
                    switch phase {
                    case .active:
                        runActivePhaseWork(manager)
                    case .background:
                        manager.dispatch(.backgrounded)
                        // Keep the core running briefly so an in-flight Lightning
                        // receive can still be claimed while backgrounded.
                        manager.beginReceiveBackgroundTaskIfNeeded()
                    default:
                        break
                    }
                }
        }
    }

    @MainActor
    private func loadManagerIfNeeded() async {
        guard manager == nil else { return }
        let storagePaths = await AppManager.prepareStorage()
        let loadedManager = AppManager(storagePaths: storagePaths)
        manager = loadedManager
        if scenePhase == .active {
            runActivePhaseWork(loadedManager)
        }
        if let pendingOpenURL {
            loadedManager.nwc.handleOpenURL(pendingOpenURL)
            self.pendingOpenURL = nil
        }
    }

    private func runActivePhaseWork(_ manager: AppManager) {
        manager.nwc.drainQueuedWakeRequests()
        manager.dispatch(.foregrounded)
        // Re-attempt claiming an in-flight Lightning receive in case the
        // payment landed while the app was suspended.
        manager.dispatch(.resumeReceiveMonitor)
        // Sweep independently of the receive screen so an HTLC cannot remain
        // stuck in the claimable state.
        manager.dispatch(.claimPendingLightningReceives)
        manager.endReceiveBackgroundTask()
    }
}
