import CoreMotion
import Foundation
import Observation
import SwiftUI

@MainActor
@Observable
final class WalletEasterEgg {
    var isDellMode = false

    @ObservationIgnored private let motionManager = CMMotionManager()
    @ObservationIgnored private var lastShakeTime: TimeInterval = 0

    var accentColor: Color {
        isDellMode ? dellBlue : rebelRed
    }

    func start() {
        guard motionManager.isAccelerometerAvailable, !motionManager.isAccelerometerActive else {
            return
        }

        motionManager.accelerometerUpdateInterval = 1.0 / 24.0
        motionManager.startAccelerometerUpdates(to: .main) { [weak self] data, _ in
            guard let self, let acceleration = data?.acceleration else { return }

            let magnitude = sqrt(
                acceleration.x * acceleration.x +
                acceleration.y * acceleration.y +
                acceleration.z * acceleration.z
            )
            let now = Date.timeIntervalSinceReferenceDate

            guard magnitude > 2.7, now - self.lastShakeTime > 1.1 else {
                return
            }

            self.lastShakeTime = now
            self.isDellMode.toggle()
        }
    }

    func stop() {
        motionManager.stopAccelerometerUpdates()
    }
}
