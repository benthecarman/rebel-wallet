import SwiftUI

@main
struct RebelWalletApp: App {
    @State private var manager = AppManager()
    @Environment(\.scenePhase) private var scenePhase

    var body: some Scene {
        WindowGroup {
            ContentView(manager: manager)
                .preferredColorScheme(.dark)
                .onChange(of: scenePhase) { _, phase in
                    if phase == .active {
                        manager.dispatch(.maintainVtxos)
                    }
                }
        }
    }
}
