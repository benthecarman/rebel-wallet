import SwiftUI

struct SecondaryButtonStyle: ButtonStyle {
    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .font(.headline)
            .foregroundStyle(primaryText)
            .padding(.vertical, 12)
            .padding(.horizontal, 14)
            .background(raisedSurface.opacity(configuration.isPressed ? 0.75 : 1), in: RoundedRectangle(cornerRadius: 8))
            .overlay(RoundedRectangle(cornerRadius: 8).stroke(borderColor))
    }
}

struct PrimaryButtonStyle: ButtonStyle {
    @Environment(\.isEnabled) private var isEnabled

    let color: Color

    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .font(.headline)
            .foregroundStyle(isEnabled ? .white : mutedText)
            .padding(.vertical, 14)
            .padding(.horizontal, 16)
            .background(
                (isEnabled ? color : raisedSurface).opacity(configuration.isPressed ? 0.82 : 1),
                in: RoundedRectangle(cornerRadius: 8)
            )
    }
}
