import BackgroundTasks
import Foundation

/// Schedules a periodic background app refresh so wallet maintenance runs even
/// when the user does not open the app. Delegated refreshes only need the wallet
/// to come online briefly to hand the next refresh to the Ark server and to
/// collect the VTXOs from rounds that already completed.
enum BackgroundMaintenance {
    static let taskIdentifier = "com.rebelwallet.app.maintenance"

    /// iOS decides when the task actually runs. Asking for an hour matches the
    /// server's round interval, so there is little to do before then anyway.
    private static let earliestInterval: TimeInterval = 60 * 60

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

    static func schedule() {
        let request = BGAppRefreshTaskRequest(identifier: taskIdentifier)
        request.earliestBeginDate = Date(timeIntervalSinceNow: earliestInterval)
        do {
            try BGTaskScheduler.shared.submit(request)
        } catch {
            // Background refresh is disabled or unavailable; foreground maintenance still runs.
        }
    }

    private static func handle(_ task: BGAppRefreshTask, manager: AppManager) {
        // Always queue the next run first so a failure here does not end the cycle.
        schedule()

        let work = Task { @MainActor in
            let succeeded = await manager.runBackgroundMaintenance()
            task.setTaskCompleted(success: succeeded)
        }
        task.expirationHandler = {
            work.cancel()
        }
    }
}
