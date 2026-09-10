import Foundation
import NwcMobileApple

enum NwcWakeInboxEvents {
    static let queueDidChange = Notification.Name("RebelWalletNwcWakeQueueDidChange")
    static let debugDidChange = Notification.Name("RebelWalletNwcWakeDebugDidChange")
}

struct NwcSettlementMonitorPlatformConfiguration {
    let serverURL: String
    let installID: String
}

extension NwcQueuedWakeRequest {
    init?(validatedUserInfo userInfo: [AnyHashable: Any]) {
        let receivedAt = UInt64(Date().timeIntervalSince1970)
        guard
            let data = try? JSONSerialization.data(withJSONObject: userInfo),
            let payloadJson = String(data: data, encoding: .utf8),
            let request = parseMobileWakePayloadJson(
                payloadJson: payloadJson,
                receivedAtSeconds: receivedAt
            )
        else {
            return nil
        }
        self.init(
            payload: NwcWakePayload(
                relayURL: request.relayUrl,
                eventIDHex: request.eventIdHex,
                walletServicePublicKeyHex: request.walletServicePublicKeyHex,
                embeddedEventJSON: request.embeddedEventJson
            ),
            receivedAtSeconds: request.receivedAtSeconds,
            settlementCheck: request.settlementCheck
        )
    }

    static let parseFailureMessage = "Ignored malformed or unrelated wake notification"
}

enum NwcWakeInbox {
    private static let queueKey = "nwcWakeQueue"
    private static let snapshotKey = "nwcWakeSnapshot"
    private static let legacyProcessedEventIdsKey = "nwcWakeProcessedEventIds"
    private static let settlementServerURLKey = "nwcSettlementServerURL"
    private static let settlementInstallIDKey = "nwcSettlementInstallID"

    static func storeSettlementMonitorConfiguration(serverURL: String?, installID: String) {
        guard let defaults = appGroupDefaults() else { return }
        if let serverURL, !serverURL.isEmpty {
            defaults.set(serverURL, forKey: settlementServerURLKey)
        } else {
            defaults.removeObject(forKey: settlementServerURLKey)
        }
        if !installID.isEmpty {
            defaults.set(installID, forKey: settlementInstallIDKey)
        }
    }

    static func settlementMonitorConfiguration() -> NwcSettlementMonitorPlatformConfiguration? {
        guard
            let defaults = appGroupDefaults(),
            let serverURL = defaults.string(forKey: settlementServerURLKey),
            !serverURL.isEmpty,
            let installID = defaults.string(forKey: settlementInstallIDKey),
            !installID.isEmpty
        else {
            return nil
        }
        return NwcSettlementMonitorPlatformConfiguration(
            serverURL: serverURL,
            installID: installID
        )
    }

    static func enqueue(_ request: NwcQueuedWakeRequest) {
        do {
            guard let store = wakeStore() else { throw CocoaError(.fileNoSuchFile) }
            try store.enqueue(request, legacyQueueKey: queueKey)
        } catch {
            NSLog("Could not persist nwc_wake request: %@", String(describing: error))
            return
        }
        NotificationCenter.default.post(name: NwcWakeInboxEvents.queueDidChange, object: nil)
    }

    static func pendingRequests() -> [NwcQueuedWakeRequest] {
        do {
            guard let store = wakeStore() else { throw CocoaError(.fileNoSuchFile) }
            return try store.pendingRequests(legacyQueueKey: queueKey)
        } catch {
            NSLog("Could not read nwc_wake requests: %@", String(describing: error))
            return []
        }
    }

    static func remove(eventIds: Set<String>) {
        guard !eventIds.isEmpty else { return }
        do {
            guard let store = wakeStore() else { throw CocoaError(.fileNoSuchFile) }
            let changed = try store.remove(eventIDs: eventIds)
            if changed {
                NotificationCenter.default.post(name: NwcWakeInboxEvents.queueDidChange, object: nil)
            }
        } catch {
            NSLog("Could not acknowledge nwc_wake requests: %@", String(describing: error))
        }
    }

    static func appendDebug(source: String, message: String) {
#if DEBUG
        guard let store = wakeStore() else {
            NSLog("Could not open app group defaults for nwc_wake debug log")
            return
        }
        do {
            try store.appendDebug(source: source, message: message)
        } catch {
            NSLog("Could not persist nwc_wake debug log: %@", String(describing: error))
            return
        }
        NotificationCenter.default.post(name: NwcWakeInboxEvents.debugDidChange, object: nil)
#endif
    }

    static func debugEntries() -> [NwcWakeDebugEntry] {
#if DEBUG
        guard let store = wakeStore() else {
            return []
        }
        do {
            return try store.debugEntries()
        } catch {
            NSLog("Could not read nwc_wake debug log: %@", String(describing: error))
            return []
        }
#else
        return []
#endif
    }

    static func clearDebugEntries() {
#if DEBUG
        guard let store = wakeStore() else {
            return
        }
        store.clearDebugEntries()
        NotificationCenter.default.post(name: NwcWakeInboxEvents.debugDidChange, object: nil)
#endif
    }

    static func removeLegacySnapshot() {
        let vault = NwcKeychainVault(
            service: "com.rebelwallet.app.nwc-wake",
            accessGroup: keychainAccessGroup
        )
        if let store = wakeStore() {
            if !store.removeLegacyState(
                defaultsKeys: [snapshotKey, legacyProcessedEventIdsKey],
                keychainEntries: [(vault: vault, key: snapshotKey)]
            ) {
                NSLog("Could not remove legacy nwc_wake Keychain state; cleanup will retry")
            }
        } else {
            appGroupDefaults()?.removeObject(forKey: snapshotKey)
            appGroupDefaults()?.removeObject(forKey: legacyProcessedEventIdsKey)
            if !vault.deleteValue(forKey: snapshotKey) {
                NSLog("Could not remove legacy nwc_wake Keychain state; cleanup will retry")
            }
        }
    }

    static func extensionDataDirectoryPath() -> String? {
        try? wakeStore()?.dataDirectoryURL().path
    }

    private static func wakeStore() -> NwcAppGroupWakeStore? {
        guard let appGroupIdentifier = appGroupIdentifier() else { return nil }
        return NwcAppGroupWakeStore(appGroupIdentifier: appGroupIdentifier)
    }

    private static func appGroupDefaults() -> UserDefaults? {
        guard let appGroupId = appGroupIdentifier() else { return nil }
        return UserDefaults(suiteName: appGroupId)
    }

    private static func appGroupIdentifier() -> String? {
        guard
            let appGroupId = Bundle.main.object(
                forInfoDictionaryKey: "RebelWalletAppGroupIdentifier"
            ) as? String,
            !appGroupId.isEmpty
        else {
            return nil
        }
        return appGroupId
    }

    private static var keychainAccessGroup: String? {
        guard let value = Bundle.main.object(forInfoDictionaryKey: "RebelWalletKeychainAccessGroup") as? String else {
            return nil
        }
        let trimmed = value.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty, !trimmed.hasPrefix("$(") else {
            return nil
        }
        return trimmed
    }
}
