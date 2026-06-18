import SwiftUI
import UIKit

struct SendView: View {
    @Bindable var manager: AppManager
    @Environment(\.walletAccent) private var walletAccent
    @State private var amountText = ""
    @FocusState private var amountFieldFocused: Bool

    private var destination: String {
        manager.state.send.destination.trimmingCharacters(in: .whitespacesAndNewlines)
    }

    private var isSending: Bool {
        manager.state.send.phase == .sending
    }

    var body: some View {
        ScrollView {
            ZStack(alignment: .topLeading) {
                Color.clear
                    .contentShape(Rectangle())
                    .onTapGesture {
                        dismissKeyboard()
                    }

                VStack(alignment: .leading, spacing: 18) {
                    Text("Paste or scan an Ark address, Lightning invoice, offer, or Lightning address.")
                        .font(.subheadline)
                        .foregroundStyle(mutedText)

                    if destination.isEmpty {
                        SendSearchPanel(manager: manager, dismissKeyboard: dismissKeyboard)
                    } else {
                        SendDestinationSummary(destination: destination, kind: manager.state.send.destinationKind) {
                            dismissKeyboard()
                            manager.dispatch(.resetSendDraft)
                        }

                        VStack(alignment: .leading, spacing: 12) {
                            Text("Amount")
                                .font(.headline)
                            TextField("Sats", text: Binding(
                                get: { amountText },
                                set: { setAmountText($0) }
                            ))
                            .keyboardType(.numberPad)
                            .focused($amountFieldFocused)
                            .padding(12)
                            .foregroundStyle(primaryText)
                            .background(surfaceBackground, in: RoundedRectangle(cornerRadius: 8))
                            .overlay(RoundedRectangle(cornerRadius: 8).stroke(borderColor))
                            .disabled(manager.state.send.amountLocked)

                            HStack(spacing: 4) {
                                Text("Available:")
                                Text(manager.state.wallet.balanceDisplay)
                                    .fontWeight(.semibold)
                                    .monospacedDigit()
                                if let fiatBalance = manager.state.wallet.balanceFiatDisplay {
                                    Text("(\(fiatBalance))")
                                        .monospacedDigit()
                                }
                            }
                            .font(.caption)
                            .foregroundStyle(mutedText)
                        }

                        VStack(alignment: .leading, spacing: 12) {
                            Text("Note")
                                .font(.headline)
                            TextField("What for?", text: Binding(
                                get: { manager.state.send.memo },
                                set: { manager.dispatch(.setSendMemo(memo: $0)) }
                            ))
                            .padding(12)
                            .foregroundStyle(primaryText)
                            .background(surfaceBackground, in: RoundedRectangle(cornerRadius: 8))
                            .overlay(RoundedRectangle(cornerRadius: 8).stroke(borderColor))
                        }

                        SendFeeSummary(send: manager.state.send)

                        if manager.state.send.zapAvailable {
                            Toggle(isOn: Binding(
                                get: { manager.state.send.zapEnabled },
                                set: { manager.dispatch(.setSendZapEnabled(enabled: $0)) }
                            )) {
                                Label("Zap", systemImage: "bolt.fill")
                                    .font(.headline)
                            }
                            .toggleStyle(.switch)
                            .padding(14)
                            .foregroundStyle(primaryText)
                            .background(surfaceBackground, in: RoundedRectangle(cornerRadius: 8))
                            .overlay(RoundedRectangle(cornerRadius: 8).stroke(borderColor))
                        }

                        if let result = manager.state.send.lastResult {
                            SendResultPanel(result: result)
                        }

                        Button {
                            dismissKeyboard()
                            manager.dispatch(.payDestination)
                        } label: {
                            HStack(spacing: 8) {
                                if isSending {
                                    ProgressView()
                                        .tint(.white)
                                } else {
                                    Image(systemName: "paperplane.fill")
                                }
                                Text(isSending ? "Sending..." : "Confirm send")
                            }
                                .frame(maxWidth: .infinity)
                        }
                        .buttonStyle(PrimaryButtonStyle(color: rebelBlue))
                        .disabled(!manager.state.send.canSubmit)

                        if let errorText = manager.state.send.errorText {
                            Text(errorText)
                                .font(.caption)
                                .foregroundStyle(walletAccent)
                        }
                    }
                }
                .padding(16)
            }
            .frame(maxWidth: .infinity, minHeight: 1, alignment: .topLeading)
        }
        .navigationTitle("Send")
        .background(pageBackground)
        .foregroundStyle(primaryText)
        .onAppear {
            syncAmountTextFromState()
        }
        .onChange(of: manager.state.send.amountSat) { _, _ in
            if !amountFieldFocused || manager.state.send.amountLocked {
                syncAmountTextFromState()
            }
        }
        .onChange(of: destination) { _, _ in
            if !amountFieldFocused || manager.state.send.amountLocked {
                syncAmountTextFromState()
            }
        }
        .onChange(of: manager.state.send.amountLocked) { _, locked in
            if locked {
                amountFieldFocused = false
                syncAmountTextFromState()
            }
        }
        .onChange(of: amountFieldFocused) { _, focused in
            if !focused {
                syncAmountTextFromState()
            }
        }
        .fullScreenCover(isPresented: Binding(
            get: { manager.state.send.phase == .success },
            set: { if !$0 { manager.dispatch(.dismissPaymentSuccess) } }
        )) {
            PaymentSuccessScreen(
                title: "Payment Sent",
                amountText: manager.state.send.successAmountDisplay,
                detail: manager.state.send.lastResult ?? "",
                confirmText: "Nice"
            ) {
                dismissKeyboard()
                returnHomeFromSuccess()
            }
        }
    }

    private func returnHomeFromSuccess() {
        manager.dispatch(.resetSendDraft)
        manager.dispatch(.selectTab(tab: .home))
        manager.dispatch(.updateScreenStack(stack: []))
    }

    private func setAmountText(_ value: String) {
        guard !manager.state.send.amountLocked else {
            syncAmountTextFromState()
            return
        }
        let digits = value.filter(\.isNumber)
        amountText = digits
        manager.dispatch(.setSendAmount(amountSat: UInt64(digits) ?? 0))
    }

    private func syncAmountTextFromState() {
        if manager.state.send.amountLocked {
            amountText = manager.state.send.amountDisplay
        } else {
            amountText = manager.state.send.amountSat == 0 ? "" : String(manager.state.send.amountSat)
        }
    }

    private func dismissKeyboard() {
        amountFieldFocused = false
        UIApplication.shared.sendAction(#selector(UIResponder.resignFirstResponder), to: nil, from: nil, for: nil)
    }
}

struct SendFeeSummary: View {
    let send: SendState

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            FeeSummaryRow(
                label: "Estimated fee",
                value: send.feeEstimateDisplay ?? (send.estimatingFee ? "Estimating..." : "-"),
                fiatValue: send.feeEstimateFiatDisplay,
                isUpdating: send.estimatingFee
            )

            FeeSummaryRow(
                label: "Total",
                value: send.totalCostDisplay ?? totalPlaceholder,
                fiatValue: send.totalCostFiatDisplay
            )

            if let error = send.feeEstimateError {
                Text("Fee estimate unavailable")
                    .font(.subheadline.weight(.semibold))
                    .padding(.top, 2)
                Text(error)
                    .font(.caption)
                    .foregroundStyle(mutedText)
                    .lineLimit(2)
            }
        }
        .padding(14)
        .frame(maxWidth: .infinity, alignment: .leading)
        .foregroundStyle(primaryText)
        .background(surfaceBackground, in: RoundedRectangle(cornerRadius: 8))
        .overlay(RoundedRectangle(cornerRadius: 8).stroke(borderColor))
    }

    private var totalPlaceholder: String {
        send.amountSat > 0 ? send.amountDisplay : "-"
    }
}

struct FeeSummaryRow: View {
    let label: String
    let value: String
    let fiatValue: String?
    var isUpdating = false

    var body: some View {
        HStack {
            HStack(spacing: 6) {
                Text(label)
                    .foregroundStyle(mutedText)
                if isUpdating {
                    ProgressView()
                        .controlSize(.mini)
                        .tint(rebelRed)
                }
            }
            Spacer()
            VStack(alignment: .trailing, spacing: 2) {
                Text(value)
                    .fontWeight(.semibold)
                    .monospacedDigit()
                Text(fiatValue ?? " ")
                    .font(.caption)
                    .foregroundStyle(mutedText)
                    .monospacedDigit()
                    .opacity(fiatValue == nil ? 0 : 1)
            }
        }
        .font(.subheadline)
    }
}

struct SendSearchPanel: View {
    @Bindable var manager: AppManager
    let dismissKeyboard: () -> Void
    @State private var requestedProfilePictureIds = Set<String>()
    @State private var pendingVisibleProfilePictureIds = Set<String>()
    @State private var visibleProfilePicturePrefetchTask: Task<Void, Never>?
    @State private var backgroundProfilePicturePrefetchTask: Task<Void, Never>?

    private let initialProfilePicturePrefetchCount = 20
    private let backgroundProfilePicturePrefetchDelayNanoseconds: UInt64 = 500_000_000
    private let visibleProfilePicturePrefetchDelayNanoseconds: UInt64 = 150_000_000

    private var searchBinding: Binding<String> {
        Binding(
            get: { manager.state.send.searchQuery },
            set: { manager.dispatch(.setSendSearchQuery(query: $0)) }
        )
    }

    private var searchResultIds: [String] {
        manager.state.send.searchResults.map(\.id)
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            HStack(spacing: 10) {
                Image(systemName: "magnifyingglass")
                    .foregroundStyle(mutedText)
                TextField("Search contacts, paste invoice, or enter address", text: searchBinding, axis: .vertical)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                    .lineLimit(1...4)
                    .submitLabel(.go)
                    .onSubmit {
                        if manager.state.send.canContinueSearch {
                            manager.dispatch(.continueSendSearch)
                        }
                    }
                if manager.state.send.searchQuery.isEmpty {
                    Button {
                        dismissKeyboard()
                        manager.dispatch(.requestClipboardRead)
                    } label: {
                        Image(systemName: "doc.on.clipboard")
                            .frame(width: 30, height: 30)
                    }
                    .buttonStyle(.plain)
                } else {
                    Button {
                        dismissKeyboard()
                        manager.dispatch(.setSendSearchQuery(query: ""))
                    } label: {
                        Image(systemName: "xmark.circle.fill")
                            .frame(width: 30, height: 30)
                    }
                    .buttonStyle(.plain)
                }
            }
            .padding(12)
            .foregroundStyle(primaryText)
            .background(surfaceBackground, in: RoundedRectangle(cornerRadius: 12))
            .overlay(RoundedRectangle(cornerRadius: 12).stroke(borderColor))

            HStack(spacing: 10) {
                Button {
                    dismissKeyboard()
                    manager.dispatch(.requestQrScan)
                } label: {
                    Label("Scan", systemImage: "qrcode.viewfinder")
                        .frame(maxWidth: .infinity)
                }
                .buttonStyle(SecondaryButtonStyle())

                Button {
                    dismissKeyboard()
                    manager.dispatch(.continueSendSearch)
                } label: {
                    Label("Continue", systemImage: "arrow.right")
                        .frame(maxWidth: .infinity)
                }
                .buttonStyle(PrimaryButtonStyle(color: rebelBlue))
                .disabled(!manager.state.send.canContinueSearch)
            }

            VStack(alignment: .leading, spacing: 10) {
                Text("Contacts")
                    .font(.headline)

                LazyVStack(spacing: 0) {
                    if manager.state.send.searchResults.isEmpty {
                        EmptyState(text: manager.state.send.searchQuery.isEmpty ? "No sendable contacts yet" : "No matching contacts")
                    } else {
                        ForEach(manager.state.send.searchResults, id: \.id) { contact in
                            Button {
                                dismissKeyboard()
                                manager.dispatch(.selectSendContact(contactId: contact.id))
                            } label: {
                                ContactRow(contact: contact, imageNormalizer: manager.rust)
                                    .padding(.vertical, 12)
                                    .onAppear {
                                        scheduleVisibleProfilePicturePrefetch(contact.id)
                                    }
                            }
                            .buttonStyle(.plain)
                            Divider().overlay(borderColor)
                        }
                    }
                }
                .padding(.horizontal, 12)
                .background(surfaceBackground, in: RoundedRectangle(cornerRadius: 12))
                .overlay(RoundedRectangle(cornerRadius: 12).stroke(borderColor))
            }
        }
        .onAppear {
            scheduleSearchResultProfilePicturePrefetch(searchResultIds)
        }
        .onChange(of: searchResultIds) { _, ids in
            scheduleSearchResultProfilePicturePrefetch(ids)
        }
        .onDisappear {
            visibleProfilePicturePrefetchTask?.cancel()
            visibleProfilePicturePrefetchTask = nil
            backgroundProfilePicturePrefetchTask?.cancel()
            backgroundProfilePicturePrefetchTask = nil
            pendingVisibleProfilePictureIds.removeAll()
            requestedProfilePictureIds.removeAll()
        }
    }

    private func scheduleSearchResultProfilePicturePrefetch(_ ids: [String]) {
        backgroundProfilePicturePrefetchTask?.cancel()
        guard !ids.isEmpty else { return }

        let initialIds = ids
            .prefix(initialProfilePicturePrefetchCount)
            .filter { requestedProfilePictureIds.insert($0).inserted }
        if !initialIds.isEmpty {
            manager.dispatch(.prefetchProfilePictures(contactIds: Array(initialIds)))
        }

        let remainingIds = ids
            .dropFirst(initialProfilePicturePrefetchCount)
            .filter { !requestedProfilePictureIds.contains($0) }
        guard !remainingIds.isEmpty else { return }

        backgroundProfilePicturePrefetchTask = Task { @MainActor in
            try? await Task.sleep(nanoseconds: backgroundProfilePicturePrefetchDelayNanoseconds)
            guard !Task.isCancelled else { return }
            let idsToPrefetch = remainingIds.filter { requestedProfilePictureIds.insert($0).inserted }
            guard !idsToPrefetch.isEmpty else { return }
            manager.dispatch(.prefetchProfilePictures(contactIds: Array(idsToPrefetch)))
        }
    }

    private func scheduleVisibleProfilePicturePrefetch(_ contactId: String) {
        guard !requestedProfilePictureIds.contains(contactId) else { return }
        pendingVisibleProfilePictureIds.insert(contactId)
        visibleProfilePicturePrefetchTask?.cancel()
        visibleProfilePicturePrefetchTask = Task { @MainActor in
            try? await Task.sleep(nanoseconds: visibleProfilePicturePrefetchDelayNanoseconds)
            guard !Task.isCancelled else { return }
            let ids = pendingVisibleProfilePictureIds.filter { requestedProfilePictureIds.insert($0).inserted }
            pendingVisibleProfilePictureIds.removeAll()
            guard !ids.isEmpty else { return }
            manager.dispatch(.prefetchProfilePictures(contactIds: Array(ids)))
        }
    }
}

struct SendDestinationSummary: View {
    let destination: String
    let kind: SendDestinationKind
    let clear: () -> Void

    private var presentation: (icon: String, color: Color, title: String) {
        switch kind {
        case .lightning:
            return ("bolt.fill", rebelBlue, "Lightning")
        case .onChain:
            return ("bitcoinsign.circle.fill", rebelRed, "On-chain address")
        case .ark, .unknown:
            return ("link", rebelGreen, "Ark address")
        }
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack(spacing: 12) {
                Image(systemName: presentation.icon)
                    .foregroundStyle(presentation.color)
                    .frame(width: 32, height: 32)
                    .background(raisedSurface, in: RoundedRectangle(cornerRadius: 8))
                VStack(alignment: .leading, spacing: 3) {
                    Text(presentation.title)
                        .font(.headline)
                    Text(destination)
                        .font(.caption.monospaced())
                        .foregroundStyle(mutedText)
                        .lineLimit(3)
                        .textSelection(.enabled)
                }
                Spacer()
                Button(action: clear) {
                    Image(systemName: "xmark")
                        .frame(width: 32, height: 32)
                }
                .buttonStyle(.plain)
            }
        }
        .padding(14)
        .background(surfaceBackground, in: RoundedRectangle(cornerRadius: 12))
        .overlay(RoundedRectangle(cornerRadius: 12).stroke(borderColor))
    }
}

struct SendResultPanel: View {
    let result: String

    var body: some View {
        HStack(spacing: 12) {
            Image(systemName: "checkmark.circle.fill")
                .font(.title2)
                .foregroundStyle(rebelGreen)
            Text(result)
                .font(.subheadline)
            Spacer()
        }
        .padding(14)
        .background(rebelGreen.opacity(0.14), in: RoundedRectangle(cornerRadius: 12))
        .overlay(RoundedRectangle(cornerRadius: 12).stroke(rebelGreen.opacity(0.35)))
    }
}

struct PaymentSuccessScreen: View {
    let title: String
    let amountText: String?
    let detail: String
    let confirmText: String
    let onConfirm: () -> Void

    var body: some View {
        ZStack {
            pageBackground.ignoresSafeArea()

            VStack(spacing: 28) {
                Spacer(minLength: 24)

                Image("MegaCheck")
                    .resizable()
                    .scaledToFit()
                    .frame(maxWidth: 240)
                    .accessibilityLabel("Success")

                VStack(spacing: 10) {
                    Text(title)
                        .font(.largeTitle.bold())
                        .multilineTextAlignment(.center)

                    if let amountText, amountText != "0 sats" {
                        Text(amountText)
                            .font(.title2.bold())
                            .foregroundStyle(rebelGreen)
                    }

                    Text(detail)
                        .font(.subheadline)
                        .foregroundStyle(mutedText)
                        .multilineTextAlignment(.center)
                        .frame(maxWidth: 320)
                }

                Spacer(minLength: 24)

                Button(action: onConfirm) {
                    Text(confirmText)
                        .frame(maxWidth: .infinity)
                }
                .buttonStyle(PrimaryButtonStyle(color: raisedSurface))
                .frame(maxWidth: 300)
            }
            .padding(24)
            .foregroundStyle(primaryText)
        }
    }
}
