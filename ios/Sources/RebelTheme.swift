import SwiftUI

let rebelBlue = Color(red: 0.28, green: 0.45, blue: 0.82)
let rebelGreen = Color(red: 0.11, green: 0.65, blue: 0.47)
let rebelRed = Color(red: 0.96, green: 0.14, blue: 0.39)
let dellBlue = Color(red: 0.00, green: 0.46, blue: 0.72)
let pageBackground = Color(red: 0.05, green: 0.05, blue: 0.05)
let surfaceBackground = Color(red: 0.12, green: 0.12, blue: 0.12)
let raisedSurface = Color(red: 0.17, green: 0.17, blue: 0.17)
let primaryText = Color.white
let mutedText = Color(red: 0.64, green: 0.64, blue: 0.64)
let borderColor = Color.white.opacity(0.10)

private struct WalletAccentKey: EnvironmentKey {
    static let defaultValue = rebelRed
}

private struct WalletUsesDellLogoKey: EnvironmentKey {
    static let defaultValue = false
}

extension EnvironmentValues {
    var walletAccent: Color {
        get { self[WalletAccentKey.self] }
        set { self[WalletAccentKey.self] = newValue }
    }

    var walletUsesDellLogo: Bool {
        get { self[WalletUsesDellLogoKey.self] }
        set { self[WalletUsesDellLogoKey.self] = newValue }
    }
}
