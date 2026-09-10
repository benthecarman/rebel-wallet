import SwiftUI
import UIKit

struct NwcConnectionsView: View {
    @Bindable var manager: AppManager
    @Environment(\.walletAccent) private var walletAccent

    private var connections: [NwcConnection] {
        manager.state.nwc.connections
    }

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 18) {
                createLink
                connectionsSection
            }
            .padding(16)
        }
        .navigationTitle("NWC")
        .toolbar {
            ToolbarItem(placement: .topBarTrailing) {
                Menu {
#if DEBUG
                    Button {
                        manager.dispatch(.pushScreen(screen: .nwcWakeLogs))
                    } label: {
                        Label("Logs", systemImage: "list.bullet.rectangle")
                    }
#endif

                    Button {
                        manager.dispatch(.pushScreen(screen: .nwcWakeStatus))
                    } label: {
                        Label("Status", systemImage: "waveform.path.ecg")
                    }
                } label: {
                    Image(systemName: "ellipsis")
                        .rotationEffect(.degrees(90))
                        .offset(y: 2)
                        .frame(width: 32, height: 32)
                }
                .accessibilityLabel("NWC options")
            }
        }
        .background(pageBackground)
        .foregroundStyle(primaryText)
    }

    private var createLink: some View {
        NavigationLink {
            NwcCreateConnectionView(manager: manager)
        } label: {
            NwcCreateHeroButton(connectionCount: connections.count)
        }
        .buttonStyle(.plain)
        .accessibilityLabel("Create NWC")
    }

    private var connectionsSection: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack {
                Text("Connections")
                    .font(.caption.bold())
                    .foregroundStyle(mutedText)
                Spacer()
                Text("\(connections.count)")
                    .font(.caption.bold())
                    .foregroundStyle(connections.isEmpty ? mutedText : rebelGreen)
            }
            .padding(.horizontal, 4)

            if connections.isEmpty {
                HStack(spacing: 12) {
                    Image("NwcIcon")
                        .resizable()
                        .scaledToFit()
                        .frame(width: 42, height: 42)

                    VStack(alignment: .leading, spacing: 4) {
                        Text("No NWC connections")
                            .font(.headline)
                        Text("Create one to authorize a Nostr Wallet Connect client.")
                            .font(.caption)
                            .foregroundStyle(mutedText)
                    }
                }
                .padding(16)
                .frame(maxWidth: .infinity, alignment: .leading)
                .background(surfaceBackground, in: RoundedRectangle(cornerRadius: 8))
                .overlay(RoundedRectangle(cornerRadius: 8).stroke(borderColor))
            } else {
                VStack(spacing: 0) {
                    ForEach(Array(connections.enumerated()), id: \.element.id) { index, connection in
                        Button {
                            manager.dispatch(.pushScreen(screen: .nwcConnectionDetail(
                                connectionId: connection.id
                            )))
                        } label: {
                            NwcConnectionRow(connection: connection)
                        }
                        .buttonStyle(.plain)
                        .accessibilityLabel("Open \(connection.name) NWC connection")

                        if index < connections.count - 1 {
                            Divider()
                                .overlay(borderColor)
                                .padding(.leading, 64)
                        }
                    }
                }
                .background(surfaceBackground, in: RoundedRectangle(cornerRadius: 8))
                .overlay(RoundedRectangle(cornerRadius: 8).stroke(borderColor))
            }
        }
    }
}

private struct NwcCreateConnectionView: View {
    @Bindable var manager: AppManager
    @Environment(\.dismiss) private var dismiss
    @Environment(\.walletAccent) private var walletAccent
    @FocusState private var focusedField: NwcCreateField?
    @State private var name = ""
    @State private var relay = "wss://relay.getalby.com\nwss://relay2.getalby.com"
    @State private var budgetText = "10,000"
    @State private var budgetInterval: NwcBudgetInterval = .daily
    @State private var permissionPreset: NwcPermissionPreset = .fullAccess
    @State private var selectedPermissions = Set<NwcPermission>(NwcPermissionPreset.fullAccess.permissions)
    @State private var customRelayDraft: NwcCustomRelayDraft?
    @State private var awaitingCreatedExport = false
    @State private var presentedCreatedExport = false

    private var parsedBudget: UInt64? {
        let cleaned = budgetText.filter(\.isNumber)
        if cleaned.isEmpty {
            return 0
        }
        return UInt64(cleaned)
    }

    private var canCreate: Bool {
        parsedBudget != nil
            && nwcRelayInputIsValid(value: relay)
    }

    private var permissionsForCreate: [NwcPermission] {
        switch permissionPreset {
        case .fullAccess, .readOnly:
            return permissionPreset.permissions
        case .custom:
            return selectedPermissions.sortedForDisplay
        }
    }

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 18) {
                VStack(alignment: .leading, spacing: 14) {
                    NwcConnectionVisualization()
                        .frame(maxWidth: .infinity)

                    TextField("Name", text: $name)
                        .textInputAutocapitalization(.words)
                        .focused($focusedField, equals: .name)
                        .profileField()

                    VStack(alignment: .leading, spacing: 10) {
                        Text("Budget")
                            .font(.caption.bold())
                            .foregroundStyle(mutedText)

                        HStack(alignment: .firstTextBaseline, spacing: 8) {
                            TextField("", text: $budgetText)
                                .keyboardType(.numberPad)
                                .focused($focusedField, equals: .budget)
                                .multilineTextAlignment(.center)
                                .font(.system(size: 34, weight: .light))
                                .foregroundStyle(primaryText)
                                .frame(minWidth: 90)
                                .onChange(of: budgetText) { _, newValue in
                                    let formatted = formatBudgetInput(newValue)
                                    if formatted != newValue {
                                        budgetText = formatted
                                    }
                                }

                            Text("sats")
                                .font(.subheadline.bold())
                                .foregroundStyle(mutedText)
                                .frame(width: 42, alignment: .leading)
                        }
                        .padding(.horizontal, 18)
                        .padding(.vertical, 12)
                        .background(Color.black, in: RoundedRectangle(cornerRadius: 8))
                        .overlay(RoundedRectangle(cornerRadius: 8).stroke(Color.white.opacity(0.18)))

                        HStack(spacing: 10) {
                            ForEach([5000, 10000, 500_000], id: \.self) { amount in
                                Button {
                                    budgetText = formatBudgetInput("\(amount)")
                                    manager.requestHaptic(.selection)
                                } label: {
                                    Text(compactSats(amount))
                                        .font(.caption.bold())
                                        .frame(maxWidth: .infinity)
                                }
                                .buttonStyle(SecondaryButtonStyle())
                            }
                        }
                    }

                    VStack(alignment: .leading, spacing: 10) {
                        Text("Interval")
                            .font(.caption.bold())
                            .foregroundStyle(mutedText)

                        Picker("Interval", selection: $budgetInterval) {
                            ForEach(NwcBudgetInterval.createOptions, id: \.self) { interval in
                                Text(interval.title)
                                    .tag(interval)
                            }
                        }
                        .pickerStyle(.segmented)
                    }

                    permissionsSection

                    VStack(alignment: .leading, spacing: 10) {
                        Text("Relay")
                            .font(.caption.bold())
                            .foregroundStyle(mutedText)

                        NwcRelayPresetMenu(
                            selection: NwcRelayPreset.matching(relay),
                            relay: relay,
                            select: selectRelayPreset,
                            editCustom: editCustomRelay
                        )
                    }

                    Button {
                        createConnection()
                    } label: {
                        Label("Connect", systemImage: "link.badge.plus")
                            .frame(maxWidth: .infinity)
                    }
                    .buttonStyle(PrimaryButtonStyle(color: walletAccent))
                    .disabled(!canCreate)
                }
                .padding(14)
                .background(surfaceBackground, in: RoundedRectangle(cornerRadius: 8))
                .overlay(RoundedRectangle(cornerRadius: 8).stroke(borderColor))
            }
            .padding(16)
        }
        .contentShape(Rectangle())
        .onTapGesture {
            focusedField = nil
        }
        .scrollDismissesKeyboard(.interactively)
        .navigationTitle("Create NWC")
        .background(pageBackground)
        .foregroundStyle(primaryText)
        .onAppear {
            relay = manager.state.nwc.defaultRelay
        }
        .onChange(of: manager.nwc.connectionExport?.id) { _, exportId in
            guard awaitingCreatedExport else { return }
            if exportId != nil {
                presentedCreatedExport = true
            } else if presentedCreatedExport {
                awaitingCreatedExport = false
                presentedCreatedExport = false
                dismiss()
            }
        }
        .sheet(item: $customRelayDraft) { draft in
            NwcCustomRelaySheet(relay: $relay, initialRelay: draft.url)
                .presentationDetents([.height(260), .medium])
        }
    }

    private var permissionsSection: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text("Permissions")
                .font(.caption.bold())
                .foregroundStyle(mutedText)

            NwcPermissionPresetMenu(
                selection: permissionPreset,
                select: selectPermissionPreset
            )

            if permissionPreset == .custom {
                HStack {
                    Spacer()

                    Button("All") {
                        selectedPermissions = Set(NwcPermission.createOptions)
                        manager.requestHaptic(.selection)
                    }
                    .font(.caption.bold())
                    .foregroundStyle(rebelGreen)

                    Button("None") {
                        selectedPermissions.removeAll()
                        manager.requestHaptic(.selection)
                    }
                    .font(.caption.bold())
                    .foregroundStyle(mutedText)
                }

                VStack(spacing: 10) {
                    ForEach(NwcPermission.createOptions, id: \.self) { permission in
                        NwcPermissionToggleRow(
                            icon: permission.icon,
                            title: permission.title,
                            caption: permission.methodName,
                            color: permission.color(walletAccent: walletAccent),
                            isOn: Binding(
                                get: { selectedPermissions.contains(permission) },
                                set: { enabled in
                                    if enabled {
                                        selectedPermissions.insert(permission)
                                    } else {
                                        selectedPermissions.remove(permission)
                                    }
                                }
                            )
                        )
                    }
                }
            }
        }
    }

    private func createConnection() {
        guard let parsedBudget else { return }
        awaitingCreatedExport = true
        manager.dispatch(.createNwcConnection(
            name: name,
            relay: relay,
            budgetSat: parsedBudget,
            budgetInterval: budgetInterval,
            permissions: permissionsForCreate
        ))
    }

    private func selectPermissionPreset(_ preset: NwcPermissionPreset) {
        permissionPreset = preset
        if preset != .custom {
            selectedPermissions = Set(preset.permissions)
        }
        manager.requestHaptic(.selection)
    }

    private func selectRelayPreset(_ preset: NwcRelayPreset?) {
        if let preset {
            relay = encodeNwcRelayURLs(preset.urls)
        }
        focusedField = nil
        manager.requestHaptic(.selection)
    }

    private func editCustomRelay() {
        focusedField = nil
        customRelayDraft = NwcCustomRelayDraft(url: relay)
        manager.requestHaptic(.selection)
    }

    private func formatBudgetInput(_ value: String) -> String {
        let digits = value.filter(\.isNumber)
        guard !digits.isEmpty else {
            return ""
        }

        var output = ""
        let reversedDigits = Array(digits.reversed())
        for (index, character) in reversedDigits.enumerated() {
            if index > 0 && index.isMultiple(of: 3) {
                output.append(",")
            }
            output.append(character)
        }
        return String(output.reversed())
    }
}

private enum NwcCreateField: Hashable {
    case name
    case budget
}

struct NwcCustomRelayDraft: Identifiable {
    let id = UUID()
    let url: String
}

struct NwcRelayPreset: Identifiable, Hashable {
    let id: String
    let title: String
    let caption: String
    let urls: [String]
    let icon: String

    static let all: [NwcRelayPreset] = [
        NwcRelayPreset(
            id: "alby",
            title: "Alby NWC",
            caption: "Dedicated NWC relay with fallback",
            urls: [
                "wss://relay.getalby.com",
                "wss://relay2.getalby.com",
            ],
            icon: "bolt.fill"
        ),
        NwcRelayPreset(
            id: "primal",
            title: "Primal",
            caption: "Public Nostr relay",
            urls: ["wss://relay.primal.net"],
            icon: "antenna.radiowaves.left.and.right"
        ),
        NwcRelayPreset(
            id: "noslol",
            title: "nos.lol",
            caption: "Public Nostr relay",
            urls: ["wss://nos.lol"],
            icon: "antenna.radiowaves.left.and.right"
        ),
        NwcRelayPreset(
            id: "nostrband",
            title: "Nostr.band",
            caption: "Public Nostr relay",
            urls: ["wss://relay.nostr.band"],
            icon: "antenna.radiowaves.left.and.right"
        ),
    ]

    static func matching(_ url: String) -> NwcRelayPreset? {
        let normalized = nwcRelayURLs(url)
        return all.first { $0.urls == normalized }
    }
}

struct NwcRelayPresetMenu: View {
    let selection: NwcRelayPreset?
    let relay: String
    let select: (NwcRelayPreset?) -> Void
    let editCustom: () -> Void

    var body: some View {
        Menu {
            ForEach(NwcRelayPreset.all) { preset in
                Button {
                    select(preset)
                } label: {
                    Label(preset.title, systemImage: preset.icon)
                }
            }

            Button {
                editCustom()
            } label: {
                Label("Custom", systemImage: "link")
            }
        } label: {
            NwcRelayPresetLabel(selection: selection, relay: relay)
        }
        .buttonStyle(.plain)
        .accessibilityLabel("NWC relay preset")
    }
}

struct NwcCustomRelaySheet: View {
    @Binding var relay: String
    @Environment(\.dismiss) private var dismiss
    @FocusState private var focused: Bool
    @State private var primaryRelay: String
    @State private var fallbackRelay: String

    private var cleanedRelays: [String] {
        nwcRelayURLs([primaryRelay, fallbackRelay].joined(separator: "\n"))
    }

    private var canSave: Bool {
        nwcRelayInputIsValid(value: cleanedRelays.joined(separator: "\n"))
    }

    init(relay: Binding<String>, initialRelay: String) {
        _relay = relay
        let relays = nwcRelayURLs(initialRelay)
        _primaryRelay = State(initialValue: relays.first ?? "")
        _fallbackRelay = State(initialValue: relays.dropFirst().first ?? "")
    }

    var body: some View {
        NavigationStack {
            VStack(alignment: .leading, spacing: 16) {
                Text("Custom Relay")
                    .font(.headline)
                    .foregroundStyle(primaryText)

                TextField("wss://relay.example.com", text: $primaryRelay)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                    .keyboardType(.URL)
                    .focused($focused)
                    .profileField()

                TextField("Optional fallback relay", text: $fallbackRelay)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                    .keyboardType(.URL)
                    .profileField()

                Text("Use websocket relay URLs that support NWC events.")
                    .font(.caption)
                    .foregroundStyle(mutedText)

                Spacer(minLength: 0)
            }
            .padding(16)
            .background(pageBackground)
            .foregroundStyle(primaryText)
            .navigationTitle("Relay")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel") {
                        dismiss()
                    }
                }

                ToolbarItem(placement: .confirmationAction) {
                    Button("Save") {
                        relay = encodeNwcRelayURLs(cleanedRelays)
                        dismiss()
                    }
                    .disabled(!canSave)
                }
            }
            .onAppear {
                focused = true
            }
        }
    }
}

private struct NwcRelayPresetLabel: View {
    let selection: NwcRelayPreset?
    let relay: String

    private var title: String {
        selection?.title ?? "Custom"
    }

    private var caption: String {
        if let selection {
            return selection.urls.joined(separator: " + ")
        }

        let relays = nwcRelayURLs(relay)
        return relays.isEmpty ? "Enter relay URL" : relays.joined(separator: " + ")
    }

    private var icon: String {
        selection?.icon ?? "link"
    }

    var body: some View {
        HStack(spacing: 12) {
            Image(systemName: icon)
                .font(.system(size: 19, weight: .medium))
                .foregroundStyle(mutedText)
                .frame(width: 28, height: 28)

            VStack(alignment: .leading, spacing: 4) {
                Text(title)
                    .font(.subheadline.bold())
                    .foregroundStyle(primaryText)

                Text(caption)
                    .font(.caption)
                    .foregroundStyle(mutedText)
                    .lineLimit(1)
                    .truncationMode(.middle)
            }

            Spacer()

            Image(systemName: "chevron.up.chevron.down")
                .font(.caption.weight(.semibold))
                .foregroundStyle(mutedText)
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 14)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(raisedSurface, in: RoundedRectangle(cornerRadius: 8))
        .overlay(RoundedRectangle(cornerRadius: 8).stroke(borderColor))
        .contentShape(RoundedRectangle(cornerRadius: 8))
    }
}

private struct NwcCreateHeroButton: View {
    let connectionCount: Int
    @Environment(\.walletAccent) private var walletAccent

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            NwcConnectionVisualization()

            Label("Create NWC", systemImage: "plus.circle.fill")
                .font(.headline)
                .frame(maxWidth: .infinity)
                .padding(.vertical, 12)
                .foregroundStyle(primaryText)
                .background(walletAccent, in: RoundedRectangle(cornerRadius: 8))
        }
        .padding(16)
        .background(surfaceBackground, in: RoundedRectangle(cornerRadius: 8))
        .overlay(RoundedRectangle(cornerRadius: 8).stroke(borderColor))
    }
}

struct NwcConnectionVisualization: View {
    var externalAppIconURL: String? = nil

    var body: some View {
        HStack(spacing: 0) {
            VStack(spacing: 10) {
                RebelMark(size: 68)
                Text("Rebel Wallet")
                    .font(.caption.bold())
                    .foregroundStyle(primaryText)
                    .lineLimit(1)
                    .minimumScaleFactor(0.8)
            }
            .frame(maxWidth: .infinity)

            ZStack {
                Rectangle()
                    .fill(
                        LinearGradient(
                            colors: [.clear, mutedText.opacity(0.65), .clear],
                            startPoint: .leading,
                            endPoint: .trailing
                        )
                    )
                    .frame(height: 4)

                Image(systemName: "cable.connector.horizontal")
                    .font(.system(size: 34, weight: .medium))
                    .foregroundStyle(mutedText)
                    .padding(.horizontal, 8)
                    .background(surfaceBackground, in: Capsule())
            }
            .frame(maxWidth: .infinity)

            VStack(spacing: 10) {
                NwaExternalAppIcon(url: externalAppIconURL, size: 68)

                Text("External App")
                    .font(.caption.bold())
                    .foregroundStyle(primaryText)
                    .lineLimit(1)
                    .minimumScaleFactor(0.8)
            }
            .frame(maxWidth: .infinity)
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 18)
        .frame(minHeight: 138)
    }
}

private struct NwaExternalAppIcon: View {
    let url: String?
    let size: CGFloat

    var body: some View {
        RemoteAppIcon(url: url, size: size) {
            RoundedRectangle(cornerRadius: 16)
                .fill(Color.white.opacity(0.92))
                .overlay {
                Image(systemName: "square.grid.2x2")
                    .font(.system(size: 38, weight: .semibold))
                    .foregroundStyle(Color.black.opacity(0.55))
                }
        }
        .overlay(RoundedRectangle(cornerRadius: 16).stroke(Color.white.opacity(0.20)))
    }
}

private struct NwcSavedConnectionIcon: View {
    let url: String?
    let size: CGFloat

    var body: some View {
        RemoteAppIcon(url: url, size: size) {
            Image("NwcIcon")
                .resizable()
                .scaledToFit()
        }
    }
}

private struct RemoteAppIcon<Placeholder: View>: View {
    let url: String?
    let size: CGFloat
    @ViewBuilder let placeholder: Placeholder
    @StateObject private var loader = ProfileImageLoader()

    var body: some View {
        ZStack {
            placeholder

            if let image = loader.image {
                Image(uiImage: image)
                    .resizable()
                    .scaledToFit()
            }
        }
        .frame(width: size, height: size)
        .clipShape(RoundedRectangle(cornerRadius: size * 0.235))
        .task(id: url) {
            guard let url, let parsed = URL(string: url) else {
                loader.reset()
                return
            }
            loader.load(parsed)
        }
    }
}

enum NwcPermissionPreset: String, CaseIterable, Identifiable {
    case fullAccess
    case readOnly
    case custom

    var id: String {
        rawValue
    }

    var title: String {
        switch self {
        case .fullAccess: return "Full Access"
        case .readOnly: return "Read Only"
        case .custom: return "Custom"
        }
    }

    var caption: String {
        switch self {
        case .fullAccess: return "Send and receive payments"
        case .readOnly: return "Receive payments and view history"
        case .custom: return "Define specific permissions"
        }
    }

    var icon: String {
        switch self {
        case .fullAccess: return "arrow.up.arrow.down"
        case .readOnly: return "arrow.down"
        case .custom: return "square.and.pencil"
        }
    }

    var color: Color {
        switch self {
        case .fullAccess: return rebelGreen
        case .readOnly: return rebelBlue
        case .custom: return mutedText
        }
    }

    var permissions: [NwcPermission] {
        switch self {
        case .fullAccess:
            return NwcPermission.createOptions
        case .readOnly:
            return [
                .getInfo,
                .getBalance,
                .makeInvoice,
                .lookupInvoice,
                .listTransactions,
            ]
        case .custom:
            return []
        }
    }

    static func matching(_ permissions: [NwcPermission]) -> NwcPermissionPreset {
        let selected = Set(permissions)
        if selected == Set(NwcPermissionPreset.fullAccess.permissions) {
            return .fullAccess
        }
        if selected == Set(NwcPermissionPreset.readOnly.permissions) {
            return .readOnly
        }
        return .custom
    }
}

struct NwcPermissionPresetMenu: View {
    let selection: NwcPermissionPreset
    let select: (NwcPermissionPreset) -> Void

    var body: some View {
        Menu {
            ForEach(NwcPermissionPreset.allCases) { preset in
                Button {
                    select(preset)
                } label: {
                    Label(preset.title, systemImage: preset.icon)
                }
            }
        } label: {
            NwcPermissionPresetLabel(preset: selection)
        }
        .buttonStyle(.plain)
        .accessibilityLabel("NWC permission preset")
    }
}

private struct NwcPermissionPresetLabel: View {
    let preset: NwcPermissionPreset

    var body: some View {
        HStack(spacing: 12) {
            Image(systemName: preset.icon)
                .font(.system(size: 19, weight: .medium))
                .foregroundStyle(mutedText)
                .frame(width: 28, height: 28)

            VStack(alignment: .leading, spacing: 4) {
                Text(preset.title)
                    .font(.subheadline.bold())
                    .foregroundStyle(primaryText)

                Text(preset.caption)
                    .font(.caption)
                    .foregroundStyle(mutedText)
            }

            Spacer()

            Image(systemName: "chevron.up.chevron.down")
                .font(.caption.weight(.semibold))
                .foregroundStyle(mutedText)
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 14)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(raisedSurface, in: RoundedRectangle(cornerRadius: 8))
        .overlay(RoundedRectangle(cornerRadius: 8).stroke(borderColor))
        .contentShape(RoundedRectangle(cornerRadius: 8))
    }
}

struct NwcPermissionToggleRow: View {
    let icon: String
    let title: String
    let caption: String
    let color: Color
    @Binding var isOn: Bool

    var body: some View {
        Toggle(isOn: $isOn) {
            HStack(spacing: 12) {
                ZStack {
                    RoundedRectangle(cornerRadius: 8)
                        .fill(color.opacity(0.18))
                    Image(systemName: icon)
                        .font(.system(size: 15, weight: .semibold))
                        .foregroundStyle(color)
                }
                .frame(width: 36, height: 36)

                VStack(alignment: .leading, spacing: 3) {
                    Text(title)
                        .font(.subheadline.bold())
                    Text(caption)
                        .font(.caption)
                        .foregroundStyle(mutedText)
                }
            }
        }
        .tint(color)
        .padding(12)
        .background(raisedSurface, in: RoundedRectangle(cornerRadius: 8))
        .overlay(RoundedRectangle(cornerRadius: 8).stroke(borderColor))
    }
}

private struct NwcConnectionRow: View {
    let connection: NwcConnection

    private var policySummary: String {
        let permissionSummary = connection.permissionPresetForDisplay?.title
            ?? "\(connection.enabledPermissionsForDisplay.count) permissions"
        return [connection.budgetDisplay, connection.budgetIntervalDisplay, permissionSummary]
            .joined(separator: "  ·  ")
    }

    var body: some View {
        HStack(spacing: 12) {
            NwcSavedConnectionIcon(url: connection.iconDisplayUrl, size: 38)

            VStack(alignment: .leading, spacing: 3) {
                Text(connection.name)
                    .font(.subheadline.bold())
                    .foregroundStyle(primaryText)
                    .lineLimit(1)
                Text(policySummary)
                    .font(.caption)
                    .foregroundStyle(mutedText)
                    .lineLimit(1)
                    .minimumScaleFactor(0.8)
            }

            Spacer(minLength: 8)

            Image(systemName: "chevron.right")
                .font(.caption.bold())
                .foregroundStyle(mutedText)
        }
        .padding(.horizontal, 12)
        .frame(minHeight: 62)
        .contentShape(Rectangle())
    }
}

struct NwcConnectionDetailView: View {
    @Bindable var manager: AppManager
    let connectionId: String
    @State private var copied = false
    @State private var showDeleteConfirmation = false

    private var connection: NwcConnection? {
        manager.state.nwc.connections.first { $0.id == connectionId }
    }

    var body: some View {
        ScrollView {
            if let connection {
                NwcConnectionDetailsCard(
                    connection: connection,
                    canExport: connection.walletManagedSecret,
                    copied: copied,
                    showQRCode: {
                        manager.dispatch(.requestNwcConnectionExport(
                            id: connection.id,
                            copyToClipboard: false
                        ))
                    },
                    copy: { copy(connection) },
                    delete: { showDeleteConfirmation = true }
                )
                .padding(16)
            } else {
                ContentUnavailableView(
                    "Connection Not Found",
                    systemImage: "link.badge.plus",
                    description: Text("This NWC connection may have been removed.")
                )
                .padding(24)
            }
        }
        .navigationTitle(connection?.name ?? "NWC Connection")
        .background(pageBackground)
        .foregroundStyle(primaryText)
        .alert("Delete NWC connection?", isPresented: $showDeleteConfirmation) {
            Button("Delete", role: .destructive) {
                manager.dispatch(.deleteNwcConnection(id: connectionId))
                manager.dispatch(.popScreen)
            }
            Button("Cancel", role: .cancel) {}
        }
    }

    private func copy(_ connection: NwcConnection) {
        manager.dispatch(.requestNwcConnectionExport(
            id: connection.id,
            copyToClipboard: true
        ))
        copied = true
        DispatchQueue.main.asyncAfter(deadline: .now() + 1.5) {
            copied = false
        }
    }
}

private struct NwcConnectionDetailsCard: View {
    let connection: NwcConnection
    let canExport: Bool
    let copied: Bool
    let showQRCode: () -> Void
    let copy: () -> Void
    let delete: () -> Void

    private var lastUsedText: String {
        guard let lastUsedAt = connection.lastUsedAt else {
            return "Never used"
        }
        return "Last used \(shortDate(lastUsedAt))"
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            HStack(alignment: .top, spacing: 12) {
                NwcSavedConnectionIcon(url: connection.iconDisplayUrl, size: 42)

                VStack(alignment: .leading, spacing: 4) {
                    Text(connection.name)
                        .font(.headline)
                    Text(nwcRelayURLs(connection.relay).joined(separator: " + "))
                        .font(.caption)
                        .foregroundStyle(mutedText)
                        .lineLimit(1)
                        .truncationMode(.middle)
                }

                Spacer()

                Button(action: delete) {
                    Image(systemName: "trash")
                        .font(.headline)
                        .foregroundStyle(rebelRed)
                        .frame(width: 36, height: 36)
                }
                .buttonStyle(.plain)
                .accessibilityLabel("Delete NWC string")
            }

            HStack(spacing: 8) {
                NwcPolicyPill(text: connection.budgetDisplay, color: primaryText)
                NwcPolicyPill(text: connection.budgetIntervalDisplay, color: primaryText)
                if let permissionPreset = connection.permissionPresetForDisplay {
                    NwcPolicyPill(text: permissionPreset.title, color: permissionPreset.color)
                }
            }

            if connection.permissionPresetForDisplay == nil {
                LazyVGrid(columns: [GridItem(.adaptive(minimum: 108), spacing: 8)], alignment: .leading, spacing: 8) {
                    ForEach(connection.enabledPermissionsForDisplay, id: \.self) { permission in
                        NwcPolicyPill(text: permission.shortTitle, color: permission.color(walletAccent: rebelBlue))
                    }
                }
            }

            VStack(alignment: .leading, spacing: 6) {
                Text(canExport ? "Wallet-managed connection string" : "Client-managed connection")
                    .font(.system(.caption, design: .monospaced))
                    .foregroundStyle(canExport ? primaryText : mutedText)
                    .textSelection(.enabled)
                    .lineLimit(3)
                    .fixedSize(horizontal: false, vertical: true)
            }
            .padding(12)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(raisedSurface, in: RoundedRectangle(cornerRadius: 8))
            .overlay(RoundedRectangle(cornerRadius: 8).stroke(borderColor))

            if canExport {
                HStack(spacing: 10) {
                    Button(action: showQRCode) {
                        Image(systemName: "qrcode")
                            .frame(width: 44)
                    }
                    .buttonStyle(SecondaryButtonStyle())
                    .accessibilityLabel("Show NWC QR code")

                    Button(action: copy) {
                        Label(copied ? "Copied" : "Copy", systemImage: copied ? "checkmark" : "doc.on.doc")
                            .frame(maxWidth: .infinity)
                            .lineLimit(1)
                    }
                    .buttonStyle(SecondaryButtonStyle())
                }
            }

            HStack(spacing: 8) {
                Text("Created \(shortDate(connection.createdAt))")
                Text(lastUsedText)
            }
            .font(.caption2)
            .foregroundStyle(mutedText)
        }
        .padding(16)
        .background(surfaceBackground, in: RoundedRectangle(cornerRadius: 8))
        .overlay(RoundedRectangle(cornerRadius: 8).stroke(borderColor))
    }
}

struct NwcConnectionExport: Identifiable {
    let id: String
    let name: String
    let uri: String
}

struct NwcConnectionQRCodeSheet: View {
    let connection: NwcConnectionExport
    @Environment(\.dismiss) private var dismiss
    @Environment(\.walletAccent) private var walletAccent
    @State private var copied: Bool

    init(connection: NwcConnectionExport, initiallyCopied: Bool = false) {
        self.connection = connection
        _copied = State(initialValue: initiallyCopied)
    }

    private var displayName: String {
        let trimmed = connection.name.trimmingCharacters(in: .whitespacesAndNewlines)
        return trimmed.isEmpty ? "NWC Connection" : trimmed
    }

    var body: some View {
        NavigationStack {
            ScrollView {
                VStack(spacing: 18) {
                    VStack(spacing: 6) {
                        Text(displayName)
                            .font(.title2.bold())
                            .multilineTextAlignment(.center)
                        Text("Scan this code from the app you want to connect.")
                            .font(.subheadline)
                            .foregroundStyle(mutedText)
                            .multilineTextAlignment(.center)
                    }

                    QRCodeView(text: connection.uri)
                        .frame(maxWidth: .infinity)

                    Text(truncateMiddle(connection.uri, maxLength: 120, prefixCount: 42))
                        .font(.caption.monospaced())
                        .foregroundStyle(primaryText)
                        .textSelection(.enabled)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .padding(12)
                        .background(raisedSurface, in: RoundedRectangle(cornerRadius: 8))
                        .overlay(RoundedRectangle(cornerRadius: 8).stroke(borderColor))

                    HStack(spacing: 10) {
                        Button(action: copyConnection) {
                            Label(copied ? "Copied" : "Copy", systemImage: copied ? "checkmark" : "doc.on.doc")
                                .frame(maxWidth: .infinity)
                        }
                        .buttonStyle(PrimaryButtonStyle(color: walletAccent))

                        ShareLink(item: connection.uri) {
                            Label("Share", systemImage: "square.and.arrow.up")
                                .frame(maxWidth: .infinity)
                        }
                        .buttonStyle(SecondaryButtonStyle())
                    }

                    Label(
                        "This QR code contains wallet access. Only share it with an app you trust.",
                        systemImage: "lock.shield"
                    )
                    .font(.caption)
                    .foregroundStyle(mutedText)
                    .frame(maxWidth: .infinity, alignment: .leading)
                }
                .padding(20)
            }
            .background(pageBackground)
            .foregroundStyle(primaryText)
            .navigationTitle("NWC Connection")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .confirmationAction) {
                    Button("Done") {
                        dismiss()
                    }
                }
            }
            .task {
                guard copied else { return }
                try? await Task.sleep(nanoseconds: 1_500_000_000)
                copied = false
            }
        }
    }

    private func copyConnection() {
        UIPasteboard.general.setItems(
            [[UIPasteboard.typeAutomatic: connection.uri]],
            options: [
                .localOnly: true,
                .expirationDate: Date().addingTimeInterval(120),
            ]
        )
        copied = true
        UIImpactFeedbackGenerator(style: .light).impactOccurred()
        DispatchQueue.main.asyncAfter(deadline: .now() + 1.5) {
            copied = false
        }
    }
}

struct NwcPolicyPill: View {
    let text: String
    let color: Color

    var body: some View {
        Text(text)
            .font(.caption.bold())
            .lineLimit(1)
            .minimumScaleFactor(0.8)
            .padding(.horizontal, 10)
            .padding(.vertical, 6)
            .foregroundStyle(color)
            .background(color.opacity(0.14), in: Capsule())
    }
}

private extension NwcConnection {
    var permissionPresetForDisplay: NwcPermissionPreset? {
        let enabled = Set(enabledPermissionsForDisplay)
        if enabled == Set(NwcPermissionPreset.fullAccess.permissions) {
            return .fullAccess
        }
        if enabled == Set(NwcPermissionPreset.readOnly.permissions) {
            return .readOnly
        }
        return nil
    }

    var enabledPermissionsForDisplay: [NwcPermission] {
        permissions.sortedForDisplay
    }
}

extension Set where Element == NwcPermission {
    var sortedForDisplay: [NwcPermission] {
        NwcPermission.createOptions.filter { contains($0) }
    }
}

extension Array where Element == NwcPermission {
    var sortedForDisplay: [NwcPermission] {
        NwcPermission.createOptions.filter { contains($0) }
    }
}

extension NwcPermission {
    static var createOptions: [NwcPermission] {
        [
            .getInfo,
            .getBalance,
            .payInvoice,
            .makeInvoice,
            .lookupInvoice,
            .listTransactions,
        ]
    }

    var title: String {
        switch self {
        case .getInfo: return "Get info"
        case .getBalance: return "Get balance"
        case .payInvoice: return "Pay invoices"
        case .makeInvoice: return "Make invoices"
        case .lookupInvoice: return "Lookup invoices"
        case .listTransactions: return "List transactions"
        }
    }

    var shortTitle: String {
        switch self {
        case .getInfo: return "Info"
        case .getBalance: return "Balance"
        case .payInvoice: return "Pay"
        case .makeInvoice: return "Invoice"
        case .lookupInvoice: return "Lookup"
        case .listTransactions: return "History"
        }
    }

    var methodName: String {
        switch self {
        case .getInfo: return "get_info"
        case .getBalance: return "get_balance"
        case .payInvoice: return "pay_invoice"
        case .makeInvoice: return "make_invoice"
        case .lookupInvoice: return "lookup_invoice"
        case .listTransactions: return "list_transactions"
        }
    }

    var icon: String {
        switch self {
        case .getInfo: return "info.circle"
        case .getBalance: return "eye"
        case .payInvoice: return "bolt.fill"
        case .makeInvoice: return "plus.app"
        case .lookupInvoice: return "magnifyingglass"
        case .listTransactions: return "list.bullet.rectangle"
        }
    }

    func color(walletAccent: Color) -> Color {
        switch self {
        case .getInfo: return rebelBlue
        case .getBalance: return rebelGreen
        case .payInvoice: return walletAccent
        case .makeInvoice: return Color.cyan
        case .lookupInvoice: return Color.orange
        case .listTransactions: return Color.indigo
        }
    }
}

extension NwcBudgetInterval {
    static var createOptions: [NwcBudgetInterval] {
        [.never, .hourly, .daily, .weekly, .monthly, .yearly]
    }

    var title: String {
        switch self {
        case .never: return "Never"
        case .hourly: return "Hourly"
        case .daily: return "Daily"
        case .weekly: return "Weekly"
        case .monthly: return "Monthly"
        case .yearly: return "Yearly"
        }
    }
}

func compactSats(_ amount: Int) -> String {
    if amount >= 1000 {
        return "\(amount / 1000)k"
    }
    return "\(amount)"
}

func nwcRelayURLs(_ value: String) -> [String] {
    var seen = Set<String>()
    return value
        .components(separatedBy: CharacterSet.whitespacesAndNewlines.union(CharacterSet(charactersIn: ",")))
        .map { $0.trimmingCharacters(in: .whitespacesAndNewlines) }
        .filter { !$0.isEmpty }
        .filter { relay in
            if seen.contains(relay) {
                return false
            }
            seen.insert(relay)
            return true
        }
        .prefix(2)
        .map { String($0) }
}

func encodeNwcRelayURLs(_ relays: [String]) -> String {
    nwcRelayURLs(relays.joined(separator: "\n")).joined(separator: "\n")
}

private func shortDate(_ unix: UInt64) -> String {
    let date = Date(timeIntervalSince1970: TimeInterval(unix))
    return date.formatted(date: .abbreviated, time: .shortened)
}
