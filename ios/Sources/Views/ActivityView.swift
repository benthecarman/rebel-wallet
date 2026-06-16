import SwiftUI
import UIKit

struct ActivityView: View {
    @Bindable var manager: AppManager
    @State private var selectedActivityId: String?

    private var selectedActivity: ActivityItem? {
        manager.state.activity.first { $0.id == selectedActivityId }
    }

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 16) {
                Text("Activity")
                    .font(.largeTitle.bold())
                VStack(spacing: 0) {
                    if manager.state.activity.isEmpty {
                        EmptyState(text: "No wallet activity recorded")
                    } else {
                        ForEach(manager.state.activity, id: \.id) { item in
                            Button {
                                selectedActivityId = item.id
                            } label: {
                                ActivityRow(item: item, imageNormalizer: manager.rust)
                            }
                            .buttonStyle(.plain)
                        }
                    }
                }
            }
            .padding(16)
        }
        .navigationTitle("Activity")
        .background(pageBackground)
        .foregroundStyle(primaryText)
        .activityPreviewSheet(item: selectedActivity, selectedActivityId: $selectedActivityId, manager: manager)
    }
}

extension View {
    func activityPreviewSheet(item: ActivityItem?, selectedActivityId: Binding<String?>, manager: AppManager) -> some View {
        self.sheet(isPresented: Binding(
            get: { selectedActivityId.wrappedValue != nil },
            set: { if !$0 { selectedActivityId.wrappedValue = nil } }
        )) {
            if let item {
                ActivityPreviewSheet(item: item, manager: manager)
                    .presentationDetents([.fraction(0.82), .large])
                    .presentationDragIndicator(.visible)
            }
        }
    }
}

struct ActivityPreviewSheet: View {
    let item: ActivityItem
    @Bindable var manager: AppManager

    private var inbound: Bool {
        item.iconKind == .received
    }

    private var accent: Color {
        inbound ? rebelGreen : rebelBlue
    }

    private var directionTitle: String {
        inbound ? "Received" : "Sent"
    }

    private var cleanedSubtitle: String {
        item.subtitle.trimmingCharacters(in: .whitespacesAndNewlines)
    }

    private var counterpartyDisplay: String {
        if let counterparty = item.counterparty, !counterparty.name.isEmpty {
            return counterparty.name
        }
        return "Unknown"
    }

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 14) {
                VStack(spacing: 10) {
                    Image(systemName: inbound ? "arrow.down.left" : "arrow.up.right")
                        .font(.system(size: 21, weight: .bold))
                        .foregroundStyle(accent)
                        .frame(width: 46, height: 46)
                        .background(accent.opacity(0.16), in: Circle())

                    VStack(spacing: 4) {
                        Text(directionTitle)
                            .font(.subheadline.weight(.medium))
                            .foregroundStyle(mutedText)
                        Text(item.signedAmountDisplay)
                            .font(.system(size: 30, weight: .bold))
                            .lineLimit(1)
                            .minimumScaleFactor(0.7)
                            .foregroundStyle(inbound ? rebelGreen : primaryText)
                    }

                    Text(item.status)
                        .font(.caption.bold())
                        .foregroundStyle(primaryText)
                        .padding(.horizontal, 10)
                        .padding(.vertical, 6)
                        .background(raisedSurface, in: Capsule())
                }
                .frame(maxWidth: .infinity)
                .padding(.vertical, 2)

                VStack(spacing: 0) {
                    ActivityPreviewLine(title: "Counterparty", value: counterpartyDisplay)
                    SettingsDivider()
                    ActivityPreviewLine(title: "Method", value: item.methodDisplay)
                    if !cleanedSubtitle.isEmpty {
                        SettingsDivider()
                        ActivityPreviewLine(title: "Fee", value: cleanedSubtitle)
                    }
                    SettingsDivider()
                    ActivityPreviewLine(title: "Date", value: item.timestamp)
                    if let arkAddress = item.arkAddress, !arkAddress.isEmpty {
                        SettingsDivider()
                        ActivityPreviewLine(title: "Ark Address", value: arkAddress, canCopy: true, manager: manager)
                    }
                    if let invoice = item.lightningInvoice, !invoice.isEmpty {
                        SettingsDivider()
                        ActivityPreviewLine(title: "Invoice", value: invoice, canCopy: true, manager: manager)
                    }
                    if let offer = item.lightningOffer, !offer.isEmpty {
                        SettingsDivider()
                        ActivityPreviewLine(title: "Offer", value: offer, canCopy: true, manager: manager)
                    }
                    if let paymentHash = item.lightningPaymentHash, !paymentHash.isEmpty {
                        SettingsDivider()
                        ActivityPreviewLine(title: "Payment Hash", value: paymentHash, canCopy: true, manager: manager)
                    }
                    if let preimage = item.lightningPaymentPreimage, !preimage.isEmpty {
                        SettingsDivider()
                        ActivityPreviewLine(title: "Preimage", value: preimage, canCopy: true, manager: manager)
                    }
                }
                .background(surfaceBackground, in: RoundedRectangle(cornerRadius: 12))
                .overlay(RoundedRectangle(cornerRadius: 12).stroke(borderColor))
            }
            .padding(16)
        }
        .background(pageBackground)
        .foregroundStyle(primaryText)
    }
}

struct ActivityPreviewLine: View {
    let title: String
    let value: String
    var canCopy: Bool = false
    var manager: AppManager? = nil

    private var displayValue: String {
        canCopy ? truncateMiddle(value, maxLength: 34) : value
    }

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: 12) {
            Text(title)
                .font(.caption)
                .foregroundStyle(mutedText)
                .frame(width: 86, alignment: .leading)

            Text(displayValue)
                .font(.subheadline)
                .monospaced(canCopy)
                .lineLimit(3)
                .textSelection(.enabled)

            if canCopy {
                Button {
                    UIPasteboard.general.string = value
                    manager?.requestHaptic(.impactLight)
                } label: {
                    Image(systemName: "doc.on.doc")
                        .font(.system(size: 13, weight: .semibold))
                }
                .buttonStyle(.plain)
                .foregroundStyle(mutedText)
                .accessibilityLabel("Copy activity ID")
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(.horizontal, 12)
        .padding(.vertical, 11)
    }
}

private func truncateMiddle(_ value: String, maxLength: Int) -> String {
    guard value.count > maxLength, maxLength > 3 else { return value }
    let edgeCount = (maxLength - 3) / 2
    return "\(value.prefix(edgeCount))...\(value.suffix(edgeCount))"
}
