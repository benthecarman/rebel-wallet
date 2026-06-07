import SwiftUI
import UIKit

struct RebelMark: View {
    let size: CGFloat

    var body: some View {
        Image("RebelMark")
            .renderingMode(.original)
            .resizable()
            .scaledToFit()
            .clipShape(RoundedRectangle(cornerRadius: size * 0.16))
        .frame(width: size, height: size)
    }
}

struct MutinyCircle<Content: View>: View {
    let size: CGFloat
    var color: Color = raisedSurface
    @ViewBuilder let content: Content

    var body: some View {
        ZStack {
            Circle()
                .fill(color)
            content
                .foregroundStyle(primaryText)
        }
        .frame(width: size, height: size)
        .shadow(color: .black.opacity(0.25), radius: 4, y: 2)
    }
}

struct NavAction: View {
    let title: String
    let icon: String
    let color: Color
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            VStack(spacing: 10) {
                Image(systemName: icon)
                    .font(.title)
                Text(title)
                    .font(.headline)
            }
            .frame(maxWidth: .infinity)
            .padding(.vertical, 18)
        }
        .buttonStyle(PrimaryButtonStyle(color: color))
    }
}

struct StatPill: View {
    let title: String
    let value: String
    var caption: String?

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            Text(title)
                .font(.caption)
                .foregroundStyle(mutedText)
            Text(value)
                .font(.subheadline.bold())
            if let caption {
                Text(caption)
                    .font(.caption2)
                    .foregroundStyle(mutedText)
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(10)
        .foregroundStyle(primaryText)
        .background(raisedSurface, in: RoundedRectangle(cornerRadius: 8))
    }
}

struct ActivityRow: View {
    let item: ActivityItem

    private var inbound: Bool {
        item.iconKind == .received
    }

    private var primaryName: String {
        item.displayPrimaryName
    }

    private var secondaryName: String {
        item.displaySecondaryName
    }

    private var counterpartyHasPicture: Bool {
        if let counterparty = item.counterparty {
            return !counterparty.picture.isEmpty
        }
        return false
    }

    private var counterpartyKnown: Bool {
        item.counterparty != nil
    }

    private var verb: String {
        item.displayVerb
    }

    private var methodIcon: String {
        item.methodIcon
    }

    private var methodColor: Color {
        inbound ? rebelGreen : rebelBlue
    }

    var body: some View {
        HStack(alignment: .center, spacing: 12) {
            Group {
                if counterpartyHasPicture {
                    ProfileAvatar(url: item.counterparty?.picture ?? "", size: 48)
                } else {
                    ZStack {
                        Circle()
                            .fill(inbound ? rebelGreen.opacity(0.18) : raisedSurface)
                        Image(systemName: methodIcon)
                            .font(.system(size: 17, weight: .semibold))
                            .foregroundStyle(methodColor)
                    }
                }
            }
            .frame(width: 48, height: 48)

            VStack(alignment: .leading, spacing: 7) {
                HStack(spacing: 0) {
                    Text(primaryName)
                        .font(.subheadline.bold())
                    .foregroundStyle(counterpartyKnown || primaryName == "You" ? primaryText : mutedText)
                    Text(" \(verb) ")
                        .font(.subheadline.weight(.light))
                        .foregroundStyle(primaryText)
                    Text(secondaryName)
                        .font(.subheadline.bold())
                    .foregroundStyle(counterpartyKnown || secondaryName == "you" ? primaryText : mutedText)
                }
                .lineLimit(1)

                HStack(spacing: 6) {
                    HStack(spacing: 4) {
                        Image(systemName: "bolt.fill")
                            .font(.system(size: 10, weight: .bold))
                        Text(item.amountDisplay)
                            .font(.caption.bold())
                    }
                    .foregroundStyle(primaryText)
                    .padding(.horizontal, 9)
                    .padding(.vertical, 5)
                    .background(inbound ? rebelGreen.opacity(0.38) : raisedSurface, in: Capsule())

                    if let messageText = item.messageText {
                        Text(messageText)
                            .font(.caption)
                            .foregroundStyle(primaryText)
                            .lineLimit(1)
                            .padding(.horizontal, 9)
                            .padding(.vertical, 5)
                            .background(raisedSurface, in: Capsule())
                    }
                }

                HStack(spacing: 5) {
                    Image(systemName: "eye.slash")
                        .font(.system(size: 10, weight: .medium))
                    Text(item.timestamp)
                        .font(.caption2)
                }
                .foregroundStyle(mutedText)
            }

            Spacer()
        }
        .padding(.vertical, 12)
        .padding(.horizontal, 2)
        .contentShape(Rectangle())
    }
}

struct ContactRow: View {
    let contact: Contact

    private var displayName: String {
        let trimmed = contact.name.trimmingCharacters(in: .whitespacesAndNewlines)
        return trimmed.isEmpty ? contact.name : trimmed
    }

    private var subtitle: String {
        if contact.lightningAddress.isEmpty {
            return contact.followed ? "Following" : "Not following"
        }
        return truncateMiddle(contact.lightningAddress.trimmingCharacters(in: .whitespacesAndNewlines), maxLength: 34)
    }

    var body: some View {
        HStack(spacing: 12) {
            ProfileAvatar(
                url: contact.picture,
                size: 42,
                initial: String(displayName.prefix(1)).uppercased()
            )
            VStack(alignment: .leading) {
                Text(displayName)
                    .font(.subheadline.bold())
                Text(subtitle)
                    .font(.caption)
                    .foregroundStyle(mutedText)
            }
            Spacer()
        }
    }
}

private func truncateMiddle(_ value: String, maxLength: Int) -> String {
    guard value.count > maxLength, maxLength > 3 else { return value }
    let edgeCount = (maxLength - 3) / 2
    let prefix = value.prefix(edgeCount)
    let suffix = value.suffix(edgeCount)
    return "\(prefix)...\(suffix)"
}

struct DirectMessageRow: View {
    let message: NostrMessage

    var body: some View {
        HStack {
            if !message.inbound { Spacer(minLength: 48) }
            VStack(alignment: message.inbound ? .leading : .trailing, spacing: 4) {
                Text(message.body)
                    .font(.subheadline)
                    .foregroundStyle(primaryText)
                    .padding(10)
                    .background(message.inbound ? raisedSurface : rebelBlue.opacity(0.45), in: RoundedRectangle(cornerRadius: 8))
                Text(message.timestamp)
                    .font(.caption2)
                    .foregroundStyle(mutedText)
            }
            if message.inbound { Spacer(minLength: 48) }
        }
    }
}

struct ReceiveStringBox: View {
    let text: String?
    let placeholder: String

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            if let text, !text.isEmpty {
                QRCodeView(text: text)
                    .frame(maxWidth: .infinity)
                Text(text)
                    .font(.caption.monospaced())
                    .textSelection(.enabled)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(10)
                    .foregroundStyle(primaryText)
                    .background(raisedSurface, in: RoundedRectangle(cornerRadius: 8))
                HStack {
                    Button {
                        UIPasteboard.general.string = text
                    } label: {
                        Label("Copy", systemImage: "doc.on.doc")
                    }
                    ShareLink(item: text) {
                        Label("Share", systemImage: "square.and.arrow.up")
                    }
                }
                .buttonStyle(.bordered)
            } else {
                Text(placeholder)
                    .font(.caption)
                    .foregroundStyle(mutedText)
            }
        }
    }
}

struct EmptyState: View {
    let text: String

    var body: some View {
        Text(text)
            .font(.subheadline)
            .foregroundStyle(mutedText)
            .frame(maxWidth: .infinity, alignment: .center)
            .padding(.vertical, 24)
    }
}

struct ToastView: View {
    let text: String
    let dismiss: () -> Void

    var body: some View {
        HStack(spacing: 10) {
            Text(text)
                .font(.footnote)
                .lineLimit(4)
            Button(action: dismiss) {
                Image(systemName: "xmark")
            }
        }
        .padding(12)
        .background(.black.opacity(0.86), in: RoundedRectangle(cornerRadius: 8))
        .foregroundStyle(.white)
        .padding()
    }
}
