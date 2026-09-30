import BackgroundTasks
import Foundation

/// Schedules a periodic background app refresh so wallet maintenance runs even
/// when the user does not open the app. Delegated refreshes only need the wallet
/// to come online briefly to hand the next refresh to the Ark server and to
/// collect the VTXOs from rounds that already completed.
enum BackgroundMaintenance {
    static let taskIdentifier = "com.rebelwallet.app.maintenance"

    /// iOS decides when the task actually runs; these only bound our request.
    /// The floor matches the server's round interval, so there is little to do
    /// sooner. The cap keeps the wallet syncing a few times a day even when no
    /// refresh is due, so incoming funds and finished rounds still get collected.
    private static let minimumInterval: TimeInterval = 60 * 60
    private static let maximumInterval: TimeInterval = 6 * 60 * 60
    /// Wake this long before a VTXO enters the refresh window so the intent is
    /// submitted with time to spare for the round and its confirmations.
    private static let leadTime: TimeInterval = 2 * 60 * 60
    private static let secondsPerBlock: TimeInterval = 10 * 60

    /// Must be called before the app finishes launching.
    static func register(manager: AppManager) {
        BGTaskScheduler.shared.register(forTaskWithIdentifier: taskIdentifier, using: nil) { task in
            guard let task = task as? BGAppRefreshTask else {
                task.setTaskCompleted(success: false)
                return
            }
            handle(task, manager: manager)
        }
    }

    /// Picks the earliest run from how many blocks remain until the next VTXO
    /// becomes due, per the Rust core's last sync. Unknown means the floor.
    static func earliestInterval(nextRefreshDueBlocks: UInt32?) -> TimeInterval {
        guard let blocks = nextRefreshDueBlocks else { return minimumInterval }
        let untilDue = TimeInterval(blocks) * secondsPerBlock - leadTime
        return min(max(untilDue, minimumInterval), maximumInterval)
    }

    static func schedule(nextRefreshDueBlocks: UInt32?) {
        let request = BGAppRefreshTaskRequest(identifier: taskIdentifier)
        request.earliestBeginDate = Date(
            timeIntervalSinceNow: earliestInterval(nextRefreshDueBlocks: nextRefreshDueBlocks)
        )
        do {
            try BGTaskScheduler.shared.submit(request)
        } catch {
            // Background refresh is disabled or unavailable; foreground maintenance still runs.
        }
    }

    private static func handle(_ task: BGAppRefreshTask, manager: AppManager) {
        let work = Task { @MainActor in
            // Queue the next run before doing anything so a failure here does not
            // end the cycle, then requeue afterwards with what maintenance learned.
            schedule(nextRefreshDueBlocks: manager.state.wallet.nextRefreshDueBlocks)
            let succeeded = await manager.runBackgroundMaintenance()
            schedule(nextRefreshDueBlocks: manager.state.wallet.nextRefreshDueBlocks)
            task.setTaskCompleted(success: succeeded)
        }
        task.expirationHandler = {
            work.cancel()
        }
    }
}
