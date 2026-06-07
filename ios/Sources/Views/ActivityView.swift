import SwiftUI

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
                                ActivityRow(item: item)
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
        .sheet(isPresented: Binding(
            get: { selectedActivityId != nil },
            set: { if !$0 { selectedActivityId = nil } }
        )) {
            if let selectedActivity {
                ActivityDetailSheet(item: selectedActivity)
                    .presentationDetents([.medium])
                    .presentationDragIndicator(.visible)
            }
        }
    }
}

struct ActivityDetailSheet: View {
    let item: ActivityItem

    var body: some View {
        VStack(alignment: .leading, spacing: 18) {
            HStack(spacing: 12) {
                Image(systemName: item.iconKind == .sent ? "arrow.up.right" : "arrow.down.left")
                    .font(.title2)
                    .foregroundStyle(item.iconKind == .sent ? rebelBlue : rebelGreen)
                    .frame(width: 44, height: 44)
                    .background(raisedSurface, in: RoundedRectangle(cornerRadius: 8))
                VStack(alignment: .leading, spacing: 3) {
                    Text(item.title)
                        .font(.title2.bold())
                    Text(item.status)
                        .font(.caption.bold())
                        .foregroundStyle(mutedText)
                }
            }

            VStack(spacing: 0) {
                DetailLine(title: "Amount", value: item.signedAmountDisplay)
                if !item.subtitle.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                    SettingsDivider()
                    DetailLine(title: "Description", value: item.subtitle)
                }
                SettingsDivider()
                DetailLine(title: "Time", value: item.timestamp)
                SettingsDivider()
                DetailLine(title: "ID", value: item.id)
            }
            .background(surfaceBackground, in: RoundedRectangle(cornerRadius: 12))
            .overlay(RoundedRectangle(cornerRadius: 12).stroke(borderColor))

            Spacer()
        }
        .padding(18)
        .background(pageBackground)
        .foregroundStyle(primaryText)
    }
}

struct DetailLine: View {
    let title: String
    let value: String

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            Text(title)
                .font(.caption)
                .foregroundStyle(mutedText)
            Text(value)
                .font(.subheadline)
                .textSelection(.enabled)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(14)
    }
}
