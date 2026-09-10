import NwcMobileApple
import SwiftUI
import UIKit

struct NwcWakeStatusView: View {
    @Bindable var manager: AppManager
#if DEBUG
    @State private var copiedDeviceToken = false

    private var apnsDeviceToken: String {
        manager.state.pushNotifications.apnsDeviceToken ?? ""
    }
#endif

    private var pushRegistrationStatus: String {
        manager.state.pushNotifications.registrationStatus
    }

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 16) {
                SettingsCard(title: "NWC Wake Status") {
                    VStack(alignment: .leading, spacing: 10) {
                        NwcDebugValueRow(title: "Status", value: manager.state.nwc.lastWakeStatus)
                        NwcDebugValueRow(
                            title: "Pending",
                            value: "\(manager.state.nwc.pendingWakeRequests.count)"
                        )
                        NwcDebugValueRow(
                            title: "Processed",
                            value: "\(manager.state.nwc.processedWakeRequests.count)"
                        )
                    }
                    .padding(.horizontal, 14)
                    .padding(.vertical, 12)
                    .frame(maxWidth: .infinity, alignment: .leading)
                }

                SettingsCard(title: "Apple Push Notification") {
                    VStack(alignment: .leading, spacing: 10) {
                        HStack(spacing: 8) {
                            Text("Device token")
                                .font(.caption.bold())
                                .foregroundStyle(mutedText)
                            Spacer()
                            Text(pushRegistrationStatus)
                                .font(.caption.bold())
                                .foregroundStyle(pushRegistrationStatus == "Registered" ? rebelGreen : mutedText)
                        }

#if DEBUG
                        Button {
                            UIPasteboard.general.setItems(
                                [[UIPasteboard.typeAutomatic: apnsDeviceToken]],
                                options: [
                                    .localOnly: true,
                                    .expirationDate: Date().addingTimeInterval(120),
                                ]
                            )
                            copiedDeviceToken = true
                            DispatchQueue.main.asyncAfter(deadline: .now() + 1.5) {
                                copiedDeviceToken = false
                            }
                        } label: {
                            Label(
                                copiedDeviceToken ? "Copied" : "Copy token",
                                systemImage: copiedDeviceToken ? "checkmark" : "doc.on.doc"
                            )
                            .font(.caption.bold())
                        }
                        .buttonStyle(.bordered)
                        .disabled(apnsDeviceToken.isEmpty)
#endif
                    }
                    .padding(.horizontal, 14)
                    .padding(.vertical, 12)
                    .frame(maxWidth: .infinity, alignment: .leading)
                }
            }
            .padding(16)
        }
        .navigationTitle("Status")
        .background(pageBackground)
        .foregroundStyle(primaryText)
    }
}

struct NwcWakeLogsView: View {
    @Bindable var manager: AppManager

    var body: some View {
        List {
            ForEach(manager.nwc.wakeDebugEntries) { entry in
                NwcWakeLogRow(entry: entry)
                    .listRowBackground(surfaceBackground)
                    .listRowSeparatorTint(borderColor)
            }
        }
        .listStyle(.plain)
        .scrollContentBackground(.hidden)
        .background(pageBackground)
        .navigationTitle("Logs")
        .toolbar {
            ToolbarItemGroup(placement: .topBarTrailing) {
                Button {
                    manager.nwc.refreshWakeDebugEntries()
                } label: {
                    Image(systemName: "arrow.clockwise")
                }
                .accessibilityLabel("Refresh logs")
                .help("Refresh logs")

                Button(role: .destructive) {
                    manager.nwc.clearWakeDebugEntries()
                } label: {
                    Image(systemName: "trash")
                }
                .disabled(manager.nwc.wakeDebugEntries.isEmpty)
                .accessibilityLabel("Clear logs")
                .help("Clear logs")
            }
        }
        .overlay {
            if manager.nwc.wakeDebugEntries.isEmpty {
                ContentUnavailableView(
                    "No Logs",
                    systemImage: "list.bullet.rectangle",
                    description: Text("NWC Wake events will appear here.")
                )
            }
        }
        .refreshable {
            manager.nwc.refreshWakeDebugEntries()
        }
        .onAppear {
            manager.nwc.refreshWakeDebugEntries()
        }
    }
}

private struct NwcWakeLogRow: View {
    let entry: NwcWakeDebugEntry

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack(spacing: 8) {
                Text(entry.source)
                    .font(.caption.bold())
                    .foregroundStyle(rebelGreen)
                Spacer()
                Text(entry.timestampText)
                    .font(.caption2)
                    .foregroundStyle(mutedText)
            }

            Text(entry.message)
                .font(.system(.caption, design: .monospaced))
                .foregroundStyle(primaryText)
                .textSelection(.enabled)
                .lineLimit(nil)
                .fixedSize(horizontal: false, vertical: true)
        }
        .padding(.vertical, 6)
    }
}

private struct NwcDebugValueRow: View {
    let title: String
    let value: String

    var body: some View {
        VStack(alignment: .leading, spacing: 2) {
            Text(title)
                .font(.caption.bold())
                .foregroundStyle(mutedText)
            Text(value.isEmpty ? "None" : value)
                .font(.system(.caption, design: .monospaced))
                .foregroundStyle(primaryText)
                .textSelection(.enabled)
                .lineLimit(nil)
                .fixedSize(horizontal: false, vertical: true)
        }
    }
}
