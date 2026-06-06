import SwiftUI

@main
struct RebelWalletApp: App {
    @State private var manager = AppManager()

    var body: some Scene {
        WindowGroup {
            ContentView(manager: manager)
                .preferredColorScheme(.dark)
        }
    }
}
