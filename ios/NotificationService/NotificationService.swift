import Foundation
import NwcMobileApple
import UserNotifications

// Leave two seconds below the approximate 30-second NSE ceiling so Rust can
// publish the terminal NWC response and Swift can deliver notification content.
private let nwcExtensionExecutionMilliseconds: UInt64 = 28_000
private let nwcNotificationCopy = NwcNotificationCopy(
    processingTitle: "Nostr Wallet Connect",
    processingBody: "Processing request",
    completedTitle: "Nostr Wallet Connect",
    completedBody: "Request completed",
    getInfoBody: "Getting Info",
    getBalanceBody: "Getting Balance",
    payInvoiceBody: "Paying Invoice",
    makeInvoiceBody: "Creating Invoice",
    lookupInvoiceBody: "Fetching Invoice",
    listTransactionsBody: "Fetching Transactions",
    openApplicationTitle: "Nostr Wallet Connect",
    openApplicationBody: "Open Rebel Wallet to continue"
)
private let nwcNotificationPresenter = NwcNotificationPresenter(copy: nwcNotificationCopy)

final class NotificationService: UNNotificationServiceExtension {
    private var adapter: NwcNotificationServiceAdapter?

    override func didReceive(
        _ request: UNNotificationRequest,
        withContentHandler contentHandler: @escaping (UNNotificationContent) -> Void
    ) {
        NwcWakeInbox.removeLegacySnapshot()
        guard let wake = NwcQueuedWakeRequest(validatedUserInfo: request.content.userInfo) else {
            NwcWakeInbox.appendDebug(
                source: "NSE",
                message: NwcQueuedWakeRequest.parseFailureMessage
            )
            contentHandler(request.content)
            return
        }
        guard let dataDirectory = NwcWakeInbox.extensionDataDirectoryPath() else {
            NwcWakeInbox.appendDebug(source: "NSE", message: "Shared storage is unavailable")
            contentHandler(nwcNotificationPresenter.content(
                applying: .openApplication,
                to: request.content,
                userInfo: wake.payload.normalizedUserInfo
            ))
            return
        }
        let settlementMonitor = NwcWakeInbox.settlementMonitorConfiguration()
        let isSettlementCheck = wake.settlementCheck

        let engine = NwcExtensionEngine(
            dataDir: dataDirectory,
            secretStore: KeychainSecretStore(),
            wakeServerUrl: settlementMonitor?.serverURL,
            installId: settlementMonitor?.installID ?? ""
        )
        let executor = RebelNwcWakeExecutor(
            engine: engine,
            settlementCheck: isSettlementCheck
        )
        let adapter = NwcNotificationServiceAdapter(
            executor: executor,
            cancellationFactory: { RebelNwcWakeCancellation() },
            executionMilliseconds: nwcExtensionExecutionMilliseconds,
            copy: nwcNotificationCopy
        )
        self.adapter = adapter

        NwcWakeInbox.appendDebug(
            source: "NSE",
            message: isSettlementCheck
                ? "Started targeted NWC invoice settlement check"
                : "Started bounded NWC wake processing"
        )
        adapter.didReceive(
            payload: wake.payload,
            content: request.content,
            contentHandler: { content in
                guard isSettlementCheck else {
                    contentHandler(content)
                    return
                }
                contentHandler(executor.settlementContent(applyingTo: content))
            }
        )
    }

    override func serviceExtensionTimeWillExpire() {
        adapter?.timeWillExpire()
    }

}

private final class RebelNwcWakeCancellation: NwcWakeCancellation, @unchecked Sendable {
    let rust = MobileCancellation()

    func cancel() {
        rust.cancel()
    }
}

private final class RebelNwcWakeExecutor: NwcWakeExecutor, @unchecked Sendable {
    private let engine: NwcExtensionEngine
    private let settlementCheck: Bool
    private let settlementStatusLock = NSLock()
    private var settlementStatus: NwcSettlementNotificationStatus = .notTracked

    init(engine: NwcExtensionEngine, settlementCheck: Bool) {
        self.engine = engine
        self.settlementCheck = settlementCheck
    }

    func execute(
        payload: NwcWakePayload,
        executionMilliseconds: UInt64,
        cancellation: any NwcWakeCancellation
    ) async -> NwcWakePresentationHint {
        guard let cancellation = cancellation as? RebelNwcWakeCancellation else {
            enqueue(payload)
            return .openApplication
        }

        let execution = await engine.executeWake(
            request: MobileWakeEnvelope(
                relayUrl: payload.relayURL,
                eventIdHex: payload.eventIDHex,
                walletServicePublicKeyHex: payload.walletServicePublicKeyHex,
                embeddedEventJson: payload.embeddedEventJSON,
                receivedAtSeconds: UInt64(Date().timeIntervalSince1970),
                settlementCheck: settlementCheck
            ),
            executionMilliseconds: executionMilliseconds,
            cancellation: cancellation.rust
        )
        let result = execution.disposition
        settlementStatusLock.withLock {
            settlementStatus = execution.settlementNotificationStatus
        }
        if settlementCheck {
            NwcWakeInbox.appendDebug(
                source: "NSE",
                message: "Targeted NWC settlement status: \(execution.settlementNotificationStatus)"
            )
        }
        if !execution.diagnosticCodes.isEmpty {
            NwcWakeInbox.appendDebug(
                source: "NSE",
                message: "NWC diagnostics: \(execution.diagnosticCodes.joined(separator: ", "))"
            )
        }

        let notification: MobileNotificationHint
        switch result {
        case .completed(let hint), .alreadyProcessed(let hint), .rejected(_, let hint):
            notification = hint
        case .queuedForApplication(_, let hint), .retryAfter(_, _, let hint):
            notification = hint
            enqueue(payload)
        }
        NwcWakeInbox.appendDebug(
            source: "NSE",
            message: "Finished bounded NWC wake processing: \(result)"
        )

        switch notification {
        case .processing:
            return .processing
        case .completed:
            return .completed
        case .request(let method):
            switch method {
            case .getInfo:
                return .request(.getInfo)
            case .getBalance:
                return .request(.getBalance)
            case .payInvoice:
                return .request(.payInvoice)
            case .makeInvoice:
                return .request(.makeInvoice)
            case .lookupInvoice:
                return .request(.lookupInvoice)
            case .listTransactions:
                return .request(.listTransactions)
            }
        case .openApplication:
            return .openApplication
        }
    }

    func settlementContent(applyingTo content: UNNotificationContent) -> UNNotificationContent {
        let status = settlementStatusLock.withLock { settlementStatus }
        guard let updated = content.mutableCopy() as? UNMutableNotificationContent else {
            return content
        }
        switch status {
        case .delivered:
            updated.title = "Payment received"
            updated.body = "Invoice settled. The connected NWC app was updated."
        case .pending:
            updated.title = "Incoming payment"
            updated.body = "Checking whether the NWC invoice has settled."
            updated.sound = nil
            updated.interruptionLevel = .passive
        case .notTracked:
            updated.title = "Nostr Wallet Connect"
            updated.body = "Checking incoming payment status."
            updated.sound = nil
            updated.interruptionLevel = .passive
        }
        return updated
    }

    private func enqueue(_ payload: NwcWakePayload) {
        NwcWakeInbox.enqueue(NwcQueuedWakeRequest(
            payload: payload,
            receivedAtSeconds: UInt64(Date().timeIntervalSince1970),
            settlementCheck: settlementCheck
        ))
    }
}
