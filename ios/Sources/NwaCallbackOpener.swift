import Foundation
import UIKit

enum NwaCallbackOpener {
    @MainActor
    static func open(_ callback: URL) async -> Bool {
        let callbackTarget = targetDescription(callback)
        let bundleId = Bundle.main.bundleIdentifier ?? "unknown"
        let universalLinksOnly = callback.scheme?.lowercased() == "https"
        NwcWakeInbox.appendDebug(
            source: "App",
            message: "NWA callback open requested wallet_bundle=\(bundleId) target=\(callbackTarget) universal_links_only=\(universalLinksOnly)"
        )

        let opened = await withCheckedContinuation { continuation in
            let completion: (Bool) -> Void = { opened in
                continuation.resume(returning: opened)
            }
            if universalLinksOnly {
                UIApplication.shared.open(
                    callback,
                    options: [.universalLinksOnly: true],
                    completionHandler: completion
                )
            } else {
                UIApplication.shared.open(callback, completionHandler: completion)
            }
        }

        NwcWakeInbox.appendDebug(
            source: "App",
            message: "NWA callback open result opened=\(opened) target=\(callbackTarget)"
        )
        return opened
    }

    private static func targetDescription(_ callback: URL) -> String {
        guard
            let components = URLComponents(url: callback, resolvingAgainstBaseURL: false),
            let scheme = components.scheme?.lowercased(),
            let host = components.host?.lowercased()
        else {
            return "invalid"
        }

        let port = components.port.map { ":\($0)" } ?? ""
        return "\(scheme)://\(host)\(port)\(components.path)"
    }
}
