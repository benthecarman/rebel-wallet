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
                    if phase == .active {
                        manager.dispatch(.maintainVtxos)
                    }
                }
        }
    }
}
