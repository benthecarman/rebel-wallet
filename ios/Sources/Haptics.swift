import UIKit

@MainActor
enum Haptics {
    static func play(_ feedback: HapticFeedback) {
        switch feedback {
        case .selection:
            UISelectionFeedbackGenerator().selectionChanged()
        case .impactLight:
            UIImpactFeedbackGenerator(style: .light).impactOccurred()
        case .impactMedium:
            UIImpactFeedbackGenerator(style: .medium).impactOccurred()
        case .notificationSuccess:
            UINotificationFeedbackGenerator().notificationOccurred(.success)
        case .notificationWarning:
            UINotificationFeedbackGenerator().notificationOccurred(.warning)
        case .notificationError:
            UINotificationFeedbackGenerator().notificationOccurred(.error)
        }
    }
}
