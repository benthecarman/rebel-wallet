import Foundation
import NwcMobileApple
import Observation
import UIKit

/// Bounded Apple capability bridge for NWC wake, push, export, and NWA callbacks.
/// Protocol validation and application policy remain in the Rust actor.
@MainActor
@Observable
final class NwcAppManager {
    var wakeDebugEntries: [NwcWakeDebugEntry]
    var connectionExport: NwcConnectionExport?

    private let dispatch: (AppAction) -> Void
    private var lastWakeStatusLogged: String
    private var notificationObservers: [NSObjectProtocol] = []

    init(initialState: AppState, dispatch: @escaping (AppAction) -> Void) {
        self.dispatch = dispatch
        wakeDebugEntries = NwcWakeInbox.debugEntries()
        lastWakeStatusLogged = initialState.nwc.lastWakeStatus
    }

    func startObserving() {
        observePushNotificationRegistration()
        observeWakeInbox()
    }

    func restorePlatformState() {
        if let deviceToken = NwcPushPlatformContext.cachedDeviceToken {
            syncPushNotificationRegistration(status: "Registered", deviceToken: deviceToken)
        }
        NwcWakeInbox.removeLegacySnapshot()
        drainQueuedWakeRequests()
    }

    func reconcile(previousState: AppState, nextState: AppState) {
        NwcWakeInbox.remove(eventIds: Set(
            nextState.nwc.processedWakeRequests.map(\.eventIdHex)
        ))

        if previousState.setup != .ready,
           nextState.setup == .ready,
           !nextState.nwc.pendingWakeRequests.isEmpty
        {
            let count = nextState.nwc.pendingWakeRequests.count
            NwcWakeInbox.appendDebug(
                source: "Rust",
                message: "Wallet ready; retrying \(count) pending NWC wake request\(count == 1 ? "" : "s")"
            )
        }

        let status = nextState.nwc.lastWakeStatus
        if status != lastWakeStatusLogged {
            lastWakeStatusLogged = status
            NwcWakeInbox.appendDebug(source: "Rust", message: status)
        }
        refreshWakeDebugEntries()
    }

    func openCallback(url: String) {
        Task {
            let opened: Bool
            if let target = URL(string: url) {
                opened = await NwaCallbackOpener.open(target)
            } else {
                opened = false
            }
            dispatch(.completeNwaCallbackOpen(opened: opened))
        }
    }

    func presentConnectionExport(
        connectionId: String,
        name: String,
        uri: String,
        copyToClipboard: Bool,
        presentQr: Bool
    ) {
        if copyToClipboard {
            UIPasteboard.general.setItems(
                [[UIPasteboard.typeAutomatic: uri]],
                options: [
                    .localOnly: true,
                    .expirationDate: Date().addingTimeInterval(120),
                ]
            )
            Haptics.play(.impactLight)
        }
        if presentQr {
            connectionExport = NwcConnectionExport(id: connectionId, name: name, uri: uri)
        }
    }

    func handleOpenURL(_ url: URL) {
        let allowedSchemes = ["nostr+walletauth", "nostr+walletauth+rebelwallet"]
        guard let scheme = url.scheme?.lowercased(), allowedSchemes.contains(scheme) else { return }
        dispatch(.openNwaRequest(uri: url.absoluteString))
    }

    func drainQueuedWakeRequests() {
        let requests = NwcWakeInbox.pendingRequests()
        guard !requests.isEmpty else { return }

        NwcWakeInbox.appendDebug(
            source: "App",
            message: "Forwarded \(requests.count) durable nwc_wake request\(requests.count == 1 ? "" : "s") to Rust"
        )
        refreshWakeDebugEntries()
        dispatch(.processNwcWakeRequests(requests: requests.map {
            NwcWakeRequest(
                relayUrl: $0.payload.relayURL,
                eventIdHex: $0.payload.eventIDHex,
                walletServicePublicKeyHex: $0.payload.walletServicePublicKeyHex,
                embeddedEventJson: $0.payload.embeddedEventJSON,
                receivedAtSeconds: $0.receivedAtSeconds,
                settlementCheck: $0.settlementCheck
            )
        }))
    }

    func refreshWakeDebugEntries() {
        wakeDebugEntries = Array(NwcWakeInbox.debugEntries().reversed())
    }

    func clearWakeDebugEntries() {
        NwcWakeInbox.clearDebugEntries()
        refreshWakeDebugEntries()
    }

    private func observePushNotificationRegistration() {
        let observer = NotificationCenter.default.addObserver(
            forName: PushNotificationEvents.registrationDidChange,
            object: nil,
            queue: .main
        ) { [weak self] notification in
            let status = notification.userInfo?[PushNotificationEvents.statusKey] as? String
            let deviceToken = notification.userInfo?[PushNotificationEvents.deviceTokenKey] as? String

            Task { @MainActor [weak self] in
                self?.syncPushNotificationRegistration(
                    status: status ?? "Unknown",
                    deviceToken: deviceToken
                )
            }
        }
        notificationObservers.append(observer)
    }

    private func syncPushNotificationRegistration(status: String, deviceToken: String?) {
        let wakeServerURL = NwcPushPlatformContext.serverURL
        let installID = NwcPushPlatformContext.installId
        NwcWakeInbox.storeSettlementMonitorConfiguration(
            serverURL: wakeServerURL,
            installID: installID
        )
        dispatch(.setPushNotificationRegistration(
            apnsDeviceToken: deviceToken ?? NwcPushPlatformContext.cachedDeviceToken,
            registrationStatus: status,
            wakeServerUrl: wakeServerURL,
            appId: Bundle.main.bundleIdentifier ?? "com.rebelwallet.app",
            environment: NwcPushPlatformContext.apnsEnvironment,
            installId: installID
        ))
    }

    private func observeWakeInbox() {
        let queueObserver = NotificationCenter.default.addObserver(
            forName: NwcWakeInboxEvents.queueDidChange,
            object: nil,
            queue: .main
        ) { [weak self] _ in
            Task { @MainActor [weak self] in
                self?.refreshWakeDebugEntries()
                self?.drainQueuedWakeRequests()
            }
        }
        let debugObserver = NotificationCenter.default.addObserver(
            forName: NwcWakeInboxEvents.debugDidChange,
            object: nil,
            queue: .main
        ) { [weak self] _ in
            Task { @MainActor [weak self] in
                self?.refreshWakeDebugEntries()
            }
        }
        notificationObservers.append(contentsOf: [queueObserver, debugObserver])
    }
}
