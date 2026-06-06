import SwiftUI
import AVFoundation
import CoreImage.CIFilterBuiltins
import PhotosUI
import UIKit

private let rebelBlue = Color(red: 0.28, green: 0.45, blue: 0.82)
private let rebelGreen = Color(red: 0.11, green: 0.65, blue: 0.47)
private let rebelRed = Color(red: 0.96, green: 0.14, blue: 0.39)
private let pageBackground = Color(red: 0.05, green: 0.05, blue: 0.05)
private let surfaceBackground = Color(red: 0.12, green: 0.12, blue: 0.12)
private let raisedSurface = Color(red: 0.17, green: 0.17, blue: 0.17)
private let primaryText = Color.white
private let mutedText = Color(red: 0.64, green: 0.64, blue: 0.64)
private let borderColor = Color.white.opacity(0.10)

struct ContentView: View {
    @Bindable var manager: AppManager
    @State private var navPath: [Screen] = []
    @State private var selectedCapabilityPhoto: PhotosPickerItem?

    var body: some View {
        NavigationStack(path: $navPath) {
            root
                .navigationDestination(for: Screen.self) { screen in
                    screenView(for: screen)
                }
        }
        .tint(rebelRed)
        .foregroundStyle(primaryText)
        .background(pageBackground.ignoresSafeArea())
        .onChange(of: manager.state.router.screenStack) { _, new in
            navPath = new
        }
        .onChange(of: navPath) { old, new in
            guard new != manager.state.router.screenStack else { return }
            manager.dispatch(.updateScreenStack(stack: new))
        }
        .overlay(alignment: .bottom) {
            if let toast = manager.state.toast {
                ToastView(text: toast) {
                    manager.dispatch(.clearToast)
                }
            }
        }
        .sheet(isPresented: Binding(
            get: { manager.state.capabilityRequest?.kind == .qrScan },
            set: { if !$0 { manager.dispatch(.cancelCapabilityRequest) } }
        )) {
            QRScannerSheet { value in
                manager.dispatch(.completeQrScan(value: value))
            }
        }
        .photosPicker(isPresented: Binding(
            get: { manager.state.capabilityRequest?.kind == .photoPick },
            set: { if !$0 { manager.dispatch(.cancelCapabilityRequest) } }
        ), selection: $selectedCapabilityPhoto, matching: .images)
        .onChange(of: selectedCapabilityPhoto) { _, item in
            guard let item else { return }
            Task {
                let data = try? await item.loadTransferable(type: Data.self)
                manager.dispatch(.completePhotoPick(imageBase64: data?.base64EncodedString()))
                selectedCapabilityPhoto = nil
            }
        }
        .onChange(of: manager.state.capabilityRequest) { _, request in
            guard request?.kind == .clipboardRead else { return }
            manager.dispatch(.completeClipboardRead(value: UIPasteboard.general.string))
        }
    }

    @ViewBuilder
    private var root: some View {
        switch manager.state.setup {
        case .ready:
            MainWalletView(manager: manager)
        case .needsSetup, .error:
            SetupView(manager: manager)
        }
    }

    @ViewBuilder
    private func screenView(for screen: Screen) -> some View {
        switch screen {
        case .setup:
            SetupView(manager: manager)
        case .home:
            MainWalletView(manager: manager)
        case .send:
            SendView(manager: manager)
        case .receive:
            ReceiveView(manager: manager)
        case .profile:
            ProfileView(manager: manager)
        case .backup:
            BackupView(manager: manager)
        case .restore:
            RestoreWalletView(manager: manager)
        case .contactDetail(let contactId):
            ContactDetailView(manager: manager, contactId: contactId)
        }
    }
}

struct SetupView: View {
    @Bindable var manager: AppManager

    var body: some View {
        VStack(spacing: 22) {
            Spacer(minLength: 24)
            RebelMark(size: 88)
            VStack(spacing: 8) {
                Text("Rebel Wallet")
                    .font(.largeTitle.bold())
                Text("Ark and Lightning on Signet")
                    .font(.subheadline)
                    .foregroundStyle(mutedText)
            }

            VStack(spacing: 12) {
                Button {
                    manager.dispatch(.createWallet)
                } label: {
                    Label("Create wallet", systemImage: "plus.circle.fill")
                        .frame(maxWidth: .infinity)
                }
                .buttonStyle(PrimaryButtonStyle(color: rebelBlue))

                Button {
                    manager.dispatch(.pushScreen(screen: .restore))
                } label: {
                    Label("Restore wallet", systemImage: "arrow.down.circle.fill")
                        .frame(maxWidth: .infinity)
                }
                .buttonStyle(PrimaryButtonStyle(color: rebelGreen))
            }

            if case .error(let message) = manager.state.setup {
                Text(message)
                    .font(.footnote)
                    .foregroundStyle(rebelRed)
                    .multilineTextAlignment(.center)
            }

            if manager.state.busy {
                ProgressView()
            }
            Spacer()
        }
        .padding(22)
        .foregroundStyle(primaryText)
        .background(pageBackground.ignoresSafeArea())
    }
}

struct MainWalletView: View {
    @Bindable var manager: AppManager

    var body: some View {
        ZStack(alignment: .bottomTrailing) {
            Group {
                switch manager.state.router.selectedTab {
                case .home:
                    HomeView(manager: manager)
                case .activity:
                    ActivityView(manager: manager)
                case .contacts:
                    ContactsView(manager: manager)
                case .settings:
                    SettingsView(manager: manager)
                }
            }
            MutinyFab(manager: manager)
        }
        .background(pageBackground.ignoresSafeArea())
    }
}

struct HomeView: View {
    @Bindable var manager: AppManager

    var body: some View {
        ScrollView {
            VStack(spacing: 16) {
                WalletHeader(manager: manager)

                VStack(alignment: .leading, spacing: 0) {
                    if manager.state.activity.isEmpty {
                        MutinyEmptyHome()
                    } else {
                        ForEach(manager.state.activity, id: \.id) { item in
                            ActivityRow(item: item)
                            Divider().overlay(borderColor)
                        }
                    }
                }
                .padding(.bottom, 88)
            }
            .padding(.horizontal, 16)
            .padding(.top, 14)
        }
        .foregroundStyle(primaryText)
        .background(pageBackground)
    }
}

struct WalletHeader: View {
    @Bindable var manager: AppManager

    var body: some View {
        HStack(spacing: 14) {
            Button {
                manager.dispatch(.pushScreen(screen: .profile))
            } label: {
                ProfileAvatar(url: manager.state.nostr.picture, size: 48)
            }
            .buttonStyle(.plain)
            Spacer(minLength: 8)
            MutinyBalanceButton(wallet: manager.state.wallet)
                .frame(maxWidth: .infinity)
            Button {
                manager.dispatch(.selectTab(tab: .settings))
            } label: {
                MutinyCircle(size: 48) {
                    RebelMark(size: 28)
                }
            }
        }
    }
}

struct MutinyBalanceButton: View {
    let wallet: WalletState

    var body: some View {
        VStack(spacing: 2) {
            Text(wallet.balanceDisplay)
                .font(.system(size: 25, weight: .light, design: .default))
                .lineLimit(1)
                .minimumScaleFactor(0.7)
            Text(wallet.network)
                .font(.caption2)
                .foregroundStyle(mutedText)
        }
        .frame(minHeight: 48)
        .frame(maxWidth: .infinity)
        .padding(.horizontal, 14)
        .background(Color.black, in: RoundedRectangle(cornerRadius: 8))
        .overlay(alignment: .top) {
            RoundedRectangle(cornerRadius: 8)
                .stroke(Color.white.opacity(0.35), lineWidth: 1)
                .mask(alignment: .top) { Rectangle().frame(height: 1) }
        }
        .overlay(alignment: .bottom) {
            RoundedRectangle(cornerRadius: 8)
                .stroke(Color.white.opacity(0.08), lineWidth: 1)
                .mask(alignment: .bottom) { Rectangle().frame(height: 1) }
        }
    }
}

struct MutinyEmptyHome: View {
    var body: some View {
        VStack(spacing: 14) {
            Image(systemName: "bolt.circle")
                .font(.system(size: 42, weight: .light))
                .foregroundStyle(mutedText)
            Text("No payments yet")
                .font(.headline)
            Text("Use the plus button to send, receive, or scan.")
                .font(.subheadline)
                .foregroundStyle(mutedText)
                .multilineTextAlignment(.center)
        }
        .frame(maxWidth: .infinity)
        .padding(.vertical, 56)
    }
}

struct MutinyFab: View {
    @Bindable var manager: AppManager
    @State private var open = false

    var body: some View {
        VStack(alignment: .trailing, spacing: 14) {
            if open {
                VStack(alignment: .leading, spacing: 0) {
                    FabMenuButton(title: "Send", icon: "arrow.up.right") {
                        open = false
                        manager.dispatch(.pushScreen(screen: .send))
                    }
                    Divider().overlay(borderColor)
                    FabMenuButton(title: "Receive", icon: "arrow.down.left") {
                        open = false
                        manager.dispatch(.pushScreen(screen: .receive))
                    }
                    Divider().overlay(borderColor)
                    FabMenuButton(title: "Scan", icon: "qrcode.viewfinder") {
                        open = false
                        manager.dispatch(.requestQrScan)
                    }
                }
                .padding(.horizontal, 8)
                .fixedSize()
                .background(surfaceBackground.opacity(0.94), in: RoundedRectangle(cornerRadius: 12))
                .overlay(RoundedRectangle(cornerRadius: 12).stroke(borderColor))
            }
            Button {
                open.toggle()
            } label: {
                MutinyCircle(size: 64, color: rebelRed) {
                    Image(systemName: "plus")
                        .font(.system(size: 30, weight: .semibold))
                }
            }
        }
        .padding(.trailing, 24)
        .padding(.bottom, 26)
    }
}

struct FabMenuButton: View {
    let title: String
    let icon: String
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            HStack(spacing: 12) {
                Image(systemName: icon)
                    .frame(width: 24)
                Text(title)
                    .font(.body)
            }
            .foregroundStyle(primaryText)
            .frame(width: 132, alignment: .leading)
            .padding(.vertical, 12)
            .padding(.horizontal, 6)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
    }
}

struct BalancePanel: View {
    let wallet: WalletState

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            Text("Balance")
                .font(.subheadline)
                .foregroundStyle(mutedText)
            Text(wallet.balanceDisplay)
                .font(.system(size: 42, weight: .bold, design: .rounded))
            HStack {
                StatPill(title: "Claimable", value: wallet.pendingReceiveDisplay)
                StatPill(title: "Sending", value: wallet.pendingSendDisplay)
            }
            if let lastSync = wallet.lastSync {
                Text("Last sync \(lastSync)")
                    .font(.caption)
                    .foregroundStyle(mutedText)
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(18)
        .foregroundStyle(primaryText)
        .background(surfaceBackground, in: RoundedRectangle(cornerRadius: 8))
        .overlay(RoundedRectangle(cornerRadius: 8).stroke(borderColor))
    }
}

struct ReceiveView: View {
    @Bindable var manager: AppManager
    @State private var amountText = ""
    @State private var didInitializeAmount = false
    @FocusState private var amountFocused: Bool

    private var method: ReceiveMethod {
        manager.state.receive.method
    }

    private var showingResult: Bool {
        manager.state.receive.phase == .creating || manager.state.receive.phase == .showingRequest || manager.state.receive.phase == .success
    }

    private var receiveText: String? {
        switch method {
        case .lightning:
            return manager.state.receive.lightningInvoice
        case .ark:
            return manager.state.receive.arkAddress
        }
    }

    var body: some View {
        ScrollView {
            VStack(spacing: 22) {
                HStack(alignment: .center) {
                    VStack(alignment: .leading, spacing: 4) {
                        Text("Receive Bitcoin")
                            .font(.largeTitle.bold())
                    }
                    Spacer()
                    if showingResult {
                        Text("Checking")
                            .font(.caption.bold())
                            .foregroundStyle(primaryText)
                            .padding(.horizontal, 10)
                            .padding(.vertical, 6)
                            .background(raisedSurface, in: Capsule())
                        Button("Edit") {
                            manager.dispatch(.editReceiveRequest)
                        }
                        .buttonStyle(SecondaryButtonStyle())
                    }
                }
                .frame(maxWidth: .infinity, alignment: .leading)

                if showingResult {
                    ReceiveRequestPanel(
                        method: method,
                        text: receiveText,
                        amountText: manager.state.receive.amountDisplay,
                        statusText: manager.state.receive.lightningStatusDisplay,
                        paid: manager.state.receive.lightningPaid
                    )
                } else {
                    Spacer(minLength: 24)

                    VStack(spacing: 16) {
                        ReceiveAmountEditor(
                            amountText: $amountText,
                            amountFocused: $amountFocused,
                            onAmountChanged: { value in
                                manager.dispatch(.setReceiveAmount(amountSat: value))
                            }
                        )
                        ReceiveMethodPicker(method: method) { method in
                            manager.dispatch(.selectReceiveMethod(method: method))
                        }
                    }
                    .frame(maxWidth: 400)

                    Spacer(minLength: 24)

                    VStack(spacing: 12) {
                        TextField("What for?", text: Binding(
                            get: { manager.state.receive.memo },
                            set: { manager.dispatch(.setReceiveMemo(memo: $0)) }
                        ))
                        .padding(14)
                        .foregroundStyle(primaryText)
                        .background(surfaceBackground, in: RoundedRectangle(cornerRadius: 8))
                        .overlay(RoundedRectangle(cornerRadius: 8).stroke(borderColor))

                        if method == .lightning && manager.state.receive.amountSat == 0 {
                            HStack(spacing: 8) {
                                Image(systemName: "info.circle")
                                Text("Lightning invoices need an amount.")
                            }
                            .font(.caption)
                            .foregroundStyle(mutedText)
                            .frame(maxWidth: .infinity, alignment: .leading)
                        }

                        Button {
                            amountFocused = false
                            manager.dispatch(.beginReceiveRequest)
                        } label: {
                            Text("Continue")
                                .frame(maxWidth: .infinity)
                        }
                        .buttonStyle(PrimaryButtonStyle(color: rebelGreen))
                        .disabled(method == .lightning && manager.state.receive.amountSat == 0)
                    }
                }
            }
            .padding(16)
            .frame(maxWidth: .infinity)
        }
        .navigationTitle("Receive")
        .background(pageBackground)
        .foregroundStyle(primaryText)
        .onAppear {
            if !didInitializeAmount {
                amountText = ""
                manager.dispatch(.setReceiveAmount(amountSat: 0))
                didInitializeAmount = true
            }
            DispatchQueue.main.asyncAfter(deadline: .now() + 0.35) {
                if !showingResult {
                    amountFocused = true
                }
            }
        }
        .fullScreenCover(isPresented: Binding(
            get: { manager.state.receive.phase == .success },
            set: { if !$0 { manager.dispatch(.dismissPaymentSuccess) } }
        )) {
            PaymentSuccessScreen(
                title: "Payment Received",
                amountText: manager.state.receive.amountDisplay,
                detail: method == .lightning ? "Lightning receive claimed." : "Ark receive confirmed.",
                confirmText: "Nice"
            ) {
                returnHomeFromSuccess()
            }
        }
    }

    private func returnHomeFromSuccess() {
        amountText = ""
        manager.dispatch(.dismissPaymentSuccess)
        manager.dispatch(.setReceiveAmount(amountSat: 0))
        manager.dispatch(.selectTab(tab: .home))
        manager.dispatch(.updateScreenStack(stack: []))
    }
}

struct ReceiveAmountEditor: View {
    @Binding var amountText: String
    var amountFocused: FocusState<Bool>.Binding
    let onAmountChanged: (UInt64) -> Void

    var body: some View {
        VStack(spacing: 6) {
            HStack(alignment: .firstTextBaseline, spacing: 8) {
                TextField("", text: $amountText)
                .keyboardType(.numberPad)
                .focused(amountFocused)
                .multilineTextAlignment(.center)
                .font(.system(size: 58, weight: .light))
                .foregroundStyle(primaryText)
                .frame(minWidth: 90)
                .onChange(of: amountText) { _, newValue in
                    let filtered = newValue.filter(\.isNumber)
                    if filtered != newValue {
                        amountText = filtered
                        return
                    }
                    onAmountChanged(UInt64(filtered) ?? 0)
                }

                Text("sats")
                    .font(.title3.bold())
                    .foregroundStyle(mutedText)
                    .frame(width: 48, alignment: .leading)
            }
            .padding(.horizontal, 18)
            .padding(.vertical, 14)
            .background(Color.black, in: RoundedRectangle(cornerRadius: 8))
            .overlay(RoundedRectangle(cornerRadius: 8).stroke(Color.white.opacity(0.18)))

            Text("Amount")
                .font(.caption.bold())
                .foregroundStyle(mutedText)
        }
    }
}

struct ReceiveRequestPanel: View {
    let method: ReceiveMethod
    let text: String?
    let amountText: String
    let statusText: String
    let paid: Bool

    var body: some View {
        VStack(spacing: 16) {
            HStack(spacing: 8) {
                Image(systemName: method.icon)
                    .foregroundStyle(method == .lightning ? rebelBlue : rebelGreen)
                Text(method.title)
                    .font(.headline)
                Spacer()
                if amountText != "0 sats" {
                    Text(amountText)
                        .font(.caption.bold())
                        .foregroundStyle(mutedText)
                }
                Text(method == .lightning ? statusText : "Waiting")
                    .font(.caption.bold())
                    .foregroundStyle(paid ? rebelGreen : mutedText)
                    .padding(.horizontal, 8)
                    .padding(.vertical, 4)
                    .background(raisedSurface, in: Capsule())
            }

            if let text, !text.isEmpty {
                if paid {
                    HStack(spacing: 8) {
                        Image(systemName: "checkmark.circle.fill")
                        Text("Payment received")
                    }
                    .font(.headline)
                    .foregroundStyle(rebelGreen)
                    .frame(maxWidth: .infinity, alignment: .leading)
                }

                QRCodeView(text: text)
                    .frame(maxWidth: .infinity)

                Text(text)
                    .font(.caption.monospaced())
                    .lineLimit(4)
                    .textSelection(.enabled)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(12)
                    .foregroundStyle(primaryText)
                    .background(Color.black, in: RoundedRectangle(cornerRadius: 8))
                    .overlay(RoundedRectangle(cornerRadius: 8).stroke(borderColor))

                HStack(spacing: 10) {
                    Button {
                        UIPasteboard.general.string = text
                    } label: {
                        Label("Copy", systemImage: "doc.on.doc")
                            .frame(maxWidth: .infinity)
                    }
                    .buttonStyle(SecondaryButtonStyle())

                    ShareLink(item: text) {
                        Label("Share", systemImage: "square.and.arrow.up")
                            .frame(maxWidth: .infinity)
                    }
                    .buttonStyle(SecondaryButtonStyle())
                }
            } else {
                VStack(spacing: 10) {
                    ProgressView()
                        .tint(rebelGreen)
                    Text("Creating request...")
                        .font(.subheadline)
                        .foregroundStyle(mutedText)
                }
                .frame(maxWidth: .infinity)
                .padding(.vertical, 120)
            }
        }
        .padding(14)
        .background(surfaceBackground, in: RoundedRectangle(cornerRadius: 8))
        .overlay(RoundedRectangle(cornerRadius: 8).stroke(borderColor))
    }
}
extension ReceiveMethod: Identifiable {
    public var id: String {
        switch self {
        case .lightning: return "lightning"
        case .ark: return "ark"
        }
    }

    var title: String {
        switch self {
        case .lightning: return "Lightning"
        case .ark: return "Ark"
        }
    }

    var caption: String {
        switch self {
        case .lightning: return "Invoice for instant payments"
        case .ark: return "Bark address"
        }
    }

    var icon: String {
        switch self {
        case .lightning: return "bolt.fill"
        case .ark: return "link"
        }
    }
}

struct ReceiveMethodPicker: View {
    let method: ReceiveMethod
    let onSelect: (ReceiveMethod) -> Void

    var body: some View {
        HStack(spacing: 0) {
            ForEach([ReceiveMethod.lightning, ReceiveMethod.ark]) { option in
                Button {
                    onSelect(option)
                } label: {
                    HStack(spacing: 8) {
                        Image(systemName: option.icon)
                            .foregroundStyle(option == .lightning ? rebelBlue : rebelGreen)
                        Text(option.title)
                            .font(.subheadline.bold())
                    }
                    .foregroundStyle(primaryText)
                    .frame(maxWidth: .infinity)
                    .padding(.vertical, 12)
                    .background(method == option ? raisedSurface : Color.clear)
                }
                .buttonStyle(.plain)
            }
        }
        .clipShape(RoundedRectangle(cornerRadius: 8))
        .background(Color.black, in: RoundedRectangle(cornerRadius: 8))
        .overlay(RoundedRectangle(cornerRadius: 8).stroke(borderColor))
    }
}

struct SendView: View {
    @Bindable var manager: AppManager
    @State private var draftDestination = ""

    private var destination: String {
        manager.state.send.destination.trimmingCharacters(in: .whitespacesAndNewlines)
    }

    private var isSending: Bool {
        manager.state.send.phase == .sending
    }

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 18) {
                VStack(alignment: .leading, spacing: 4) {
                    Text("Send")
                        .font(.largeTitle.bold())
                    Text("Paste or scan an Ark address or Lightning invoice.")
                        .font(.subheadline)
                        .foregroundStyle(mutedText)
                }

                if destination.isEmpty {
                    VStack(spacing: 14) {
                        TextField("Lightning invoice or Ark address", text: $draftDestination, axis: .vertical)
                            .textInputAutocapitalization(.never)
                            .autocorrectionDisabled()
                            .lineLimit(3...6)
                            .padding(12)
                            .foregroundStyle(primaryText)
                            .background(surfaceBackground, in: RoundedRectangle(cornerRadius: 8))
                            .overlay(RoundedRectangle(cornerRadius: 8).stroke(borderColor))

                        HStack(spacing: 10) {
                            Button {
                                manager.dispatch(.requestClipboardRead)
                            } label: {
                                Label("Paste", systemImage: "doc.on.clipboard")
                                    .frame(maxWidth: .infinity)
                            }
                            .buttonStyle(SecondaryButtonStyle())

                            Button {
                                manager.dispatch(.requestQrScan)
                            } label: {
                                Label("Scan", systemImage: "qrcode.viewfinder")
                                    .frame(maxWidth: .infinity)
                            }
                            .buttonStyle(SecondaryButtonStyle())
                        }

                        Button {
                            manager.dispatch(.setSendDestination(destination: draftDestination))
                        } label: {
                            Text("Continue")
                                .frame(maxWidth: .infinity)
                        }
                        .buttonStyle(PrimaryButtonStyle(color: rebelBlue))
                        .disabled(draftDestination.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                    }
                    .padding(14)
                    .background(surfaceBackground, in: RoundedRectangle(cornerRadius: 12))
                    .overlay(RoundedRectangle(cornerRadius: 12).stroke(borderColor))
                } else {
                    SendDestinationSummary(destination: destination, kind: manager.state.send.destinationKind) {
                        draftDestination = ""
                        manager.dispatch(.resetSendDraft)
                    }

                    VStack(alignment: .leading, spacing: 12) {
                        Text("Amount")
                            .font(.headline)
                        TextField("Sats", value: Binding(
                            get: { manager.state.send.amountSat },
                            set: { manager.dispatch(.setSendAmount(amountSat: $0)) }
                        ), format: .number)
                        .keyboardType(.numberPad)
                        .padding(12)
                        .foregroundStyle(primaryText)
                        .background(surfaceBackground, in: RoundedRectangle(cornerRadius: 8))
                        .overlay(RoundedRectangle(cornerRadius: 8).stroke(borderColor))

                        Text(manager.state.send.destinationKind == .lightning ? "Leave amount at 0 for invoices that already include an amount." : "Ark sends require an amount.")
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

                    if let result = manager.state.send.lastResult {
                        SendResultPanel(result: result)
                    }

                    Button {
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
                            .foregroundStyle(rebelRed)
                    }
                }
            }
            .padding(16)
        }
        .navigationTitle("Send")
        .background(pageBackground)
        .foregroundStyle(primaryText)
        .onAppear {
            draftDestination = manager.state.send.destination
        }
        .onDisappear {
            draftDestination = ""
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
                returnHomeFromSuccess()
            }
        }
    }

    private func returnHomeFromSuccess() {
        draftDestination = ""
        manager.dispatch(.resetSendDraft)
        manager.dispatch(.selectTab(tab: .home))
        manager.dispatch(.updateScreenStack(stack: []))
    }
}

struct SendDestinationSummary: View {
    let destination: String
    let kind: SendDestinationKind
    let clear: () -> Void

    private var isLightning: Bool {
        kind == .lightning
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack(spacing: 12) {
                Image(systemName: isLightning ? "bolt.fill" : "link")
                    .foregroundStyle(isLightning ? rebelBlue : rebelGreen)
                    .frame(width: 32, height: 32)
                    .background(raisedSurface, in: RoundedRectangle(cornerRadius: 8))
                VStack(alignment: .leading, spacing: 3) {
                    Text(isLightning ? "Lightning invoice" : "Ark address")
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

struct ContactsView: View {
    @Bindable var manager: AppManager
    @State private var query = ""
    @State private var npub = ""
    @State private var name = ""
    @State private var lightningAddress = ""
    @State private var adding = false

    private var contacts: [Contact] {
        let trimmed = query.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        guard !trimmed.isEmpty else { return manager.state.nostr.contacts }
        return manager.state.nostr.contacts.filter { contact in
            contact.name.lowercased().contains(trimmed)
                || contact.npub.lowercased().contains(trimmed)
                || contact.lightningAddress.lowercased().contains(trimmed)
        }
    }

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 16) {
                HStack {
                    VStack(alignment: .leading, spacing: 4) {
                        Text("Social")
                            .font(.largeTitle.bold())
                        Text("Search contacts, send payments, and message over Nostr.")
                            .font(.subheadline)
                            .foregroundStyle(mutedText)
                    }
                    Spacer()
                    Button {
                        manager.dispatch(.refreshContactList)
                    } label: {
                        Image(systemName: "arrow.clockwise")
                            .frame(width: 36, height: 36)
                    }
                    .buttonStyle(.plain)
                    Button {
                        manager.dispatch(.publishContactList)
                    } label: {
                        Image(systemName: "paperplane")
                            .frame(width: 36, height: 36)
                    }
                    .buttonStyle(.plain)
                }

                HStack(spacing: 10) {
                    Image(systemName: "magnifyingglass")
                        .foregroundStyle(mutedText)
                    TextField("Search contacts, npub, or lightning address", text: $query)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                    if !query.isEmpty {
                        Button {
                            query = ""
                        } label: {
                            Image(systemName: "xmark.circle.fill")
                        }
                        .buttonStyle(.plain)
                    }
                }
                .padding(12)
                .background(surfaceBackground, in: RoundedRectangle(cornerRadius: 12))
                .overlay(RoundedRectangle(cornerRadius: 12).stroke(borderColor))

                Button {
                    adding.toggle()
                } label: {
                    HStack(spacing: 12) {
                        Image(systemName: adding ? "minus" : "plus")
                            .foregroundStyle(rebelRed)
                            .frame(width: 28)
                        Text(adding ? "Hide new contact" : "New contact")
                            .font(.headline)
                        Spacer()
                    }
                }
                .buttonStyle(.plain)
                .padding(14)
                .background(surfaceBackground, in: RoundedRectangle(cornerRadius: 12))
                .overlay(RoundedRectangle(cornerRadius: 12).stroke(borderColor))

                if adding {
                    VStack(spacing: 10) {
                        TextField("npub", text: $npub)
                            .textInputAutocapitalization(.never)
                            .autocorrectionDisabled()
                            .profileField()
                        TextField("Name", text: $name)
                            .profileField()
                        TextField("Lightning address", text: $lightningAddress)
                            .textInputAutocapitalization(.never)
                            .autocorrectionDisabled()
                            .profileField()
                        Button("Add and follow") {
                            manager.dispatch(.addContact(npub: npub, name: name.isEmpty ? npub : name, lightningAddress: lightningAddress, lnurl: "", picture: ""))
                            npub = ""
                            name = ""
                            lightningAddress = ""
                            adding = false
                        }
                        .buttonStyle(PrimaryButtonStyle(color: rebelBlue))
                        .disabled(npub.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                    }
                    .padding(14)
                    .background(surfaceBackground, in: RoundedRectangle(cornerRadius: 12))
                    .overlay(RoundedRectangle(cornerRadius: 12).stroke(borderColor))
                }

                VStack(spacing: 0) {
                    if contacts.isEmpty {
                        EmptyState(text: query.isEmpty ? "No Nostr contacts yet" : "No matching contacts")
                    } else {
                        ForEach(contacts, id: \.id) { contact in
                            Button {
                                manager.dispatch(.pushScreen(screen: .contactDetail(contactId: contact.id)))
                            } label: {
                                ContactRow(contact: contact)
                                    .padding(.vertical, 12)
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
            .padding(16)
        }
        .navigationTitle("Social")
        .background(pageBackground)
        .foregroundStyle(primaryText)
    }
}

struct SettingsView: View {
    @Bindable var manager: AppManager

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 18) {
                Button {
                    manager.dispatch(.selectTab(tab: .home))
                } label: {
                    Label("Back to wallet", systemImage: "chevron.left")
                        .foregroundStyle(mutedText)
                }
                .buttonStyle(.plain)

                Text("Settings")
                    .font(.largeTitle.bold())

                SettingsCard(title: "General") {
                    SettingsRow(title: "Backup", caption: "Show recovery phrase", accent: rebelGreen) {
                        manager.dispatch(.showSeed)
                        manager.dispatch(.pushScreen(screen: .backup))
                    }
                    SettingsDivider()
                    SettingsRow(title: "Restore", caption: "Replace this wallet from seed words", accent: rebelRed) {
                        manager.dispatch(.pushScreen(screen: .restore))
                    }
                    SettingsDivider()
                    SettingsRow(title: "Servers", caption: manager.state.wallet.serverAddress, disabled: true) {}
                }

                SettingsCard(title: "Appearance") {
                    SettingsRow(title: "Currency", caption: "Sats", disabled: true) {}
                    SettingsDivider()
                    SettingsRow(title: "Language", caption: "English", disabled: true) {}
                }

                SettingsCard(title: "Social") {
                    SettingsRow(title: "Nostr keys", caption: manager.state.nostr.npub ?? "No Nostr key") {
                        manager.dispatch(.pushScreen(screen: .profile))
                    }
                }

                Text("Secrets are stored in iOS Keychain. Wallet data uses local sqlite.")
                    .font(.caption)
                    .foregroundStyle(mutedText)
                    .frame(maxWidth: .infinity, alignment: .center)
                    .padding(.bottom, 24)
            }
            .padding(16)
        }
        .navigationTitle("Settings")
        .background(pageBackground)
        .foregroundStyle(primaryText)
    }
}

struct SettingsCard<Content: View>: View {
    let title: String
    @ViewBuilder let content: Content

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            Text(title)
                .font(.caption.bold())
                .foregroundStyle(mutedText)
                .padding(.horizontal, 14)
                .padding(.top, 12)
                .padding(.bottom, 6)
            content
        }
        .background(surfaceBackground, in: RoundedRectangle(cornerRadius: 12))
        .overlay(RoundedRectangle(cornerRadius: 12).stroke(borderColor))
    }
}

struct SettingsRow: View {
    let title: String
    let caption: String?
    var accent: Color?
    var disabled = false
    let action: () -> Void

    init(title: String, caption: String? = nil, accent: Color? = nil, disabled: Bool = false, action: @escaping () -> Void) {
        self.title = title
        self.caption = caption
        self.accent = accent
        self.disabled = disabled
        self.action = action
    }

    var body: some View {
        Button(action: action) {
            HStack(spacing: 12) {
                VStack(alignment: .leading, spacing: 4) {
                    Text(title)
                        .font(.body)
                        .foregroundStyle(accent ?? primaryText)
                    if let caption, !caption.isEmpty {
                        Text(caption)
                            .font(.caption)
                            .foregroundStyle(mutedText)
                            .lineLimit(2)
                    }
                }
                Spacer()
                Image(systemName: "chevron.right")
                    .foregroundStyle(mutedText)
            }
            .padding(.horizontal, 14)
            .padding(.vertical, 12)
            .opacity(disabled ? 0.45 : 1)
        }
        .buttonStyle(.plain)
        .disabled(disabled)
    }
}

struct SettingsDivider: View {
    var body: some View {
        Divider()
            .overlay(borderColor)
            .padding(.leading, 14)
    }
}

struct BackupView: View {
    @Bindable var manager: AppManager
    @State private var revealed = false
    @State private var copied = false
    @State private var checkedSecure = false
    @State private var checkedResponsibility = false
    @State private var checkedPrivate = false

    private var words: [String] {
        (manager.state.recoveryPhrase ?? "")
            .split(separator: " ")
            .map(String.init)
    }

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 18) {
                VStack(alignment: .leading, spacing: 10) {
                    Text("Backup")
                        .font(.largeTitle.bold())
                    Text("Your recovery phrase controls your funds. Write these words down and keep them offline.")
                        .font(.body)
                        .foregroundStyle(mutedText)
                    Text("Anyone with these words can restore and spend from this wallet.")
                        .font(.body)
                        .foregroundStyle(mutedText)
                }

                SeedWordsPanel(words: words, revealed: $revealed, copied: $copied)

                if revealed {
                    VStack(alignment: .leading, spacing: 12) {
                        BackupCheckBox(checked: $checkedSecure, text: "I wrote the words down.")
                        BackupCheckBox(checked: $checkedResponsibility, text: "I understand Rebel cannot recover them.")
                        BackupCheckBox(checked: $checkedPrivate, text: "I will not share them with anyone.")
                    }
                    .padding(14)
                    .background(surfaceBackground, in: RoundedRectangle(cornerRadius: 12))
                    .overlay(RoundedRectangle(cornerRadius: 12).stroke(borderColor))
                }

                Button {
                    manager.dispatch(.popScreen)
                } label: {
                    Label("Done", systemImage: "checkmark")
                        .frame(maxWidth: .infinity)
                }
                .buttonStyle(PrimaryButtonStyle(color: rebelBlue))
                .disabled(!(revealed && checkedSecure && checkedResponsibility && checkedPrivate))
            }
            .padding(16)
        }
        .navigationTitle("Backup")
        .scrollContentBackground(.hidden)
        .background(pageBackground)
        .foregroundStyle(primaryText)
        .onAppear {
            if manager.state.recoveryPhrase == nil {
                manager.dispatch(.showSeed)
            }
        }
    }
}

struct RestoreWalletView: View {
    @Bindable var manager: AppManager
    @State private var phrase = ""
    @State private var confirmingReplace = false

    private var normalizedPhrase: String {
        phrase
            .split(whereSeparator: { $0.isWhitespace })
            .joined(separator: " ")
    }

    private var wordCount: Int {
        normalizedPhrase.isEmpty ? 0 : normalizedPhrase.split(separator: " ").count
    }

    private var replacingCurrentWallet: Bool {
        if case .ready = manager.state.setup {
            return true
        }
        return false
    }

    private var canRestore: Bool {
        wordCount >= 12 && !manager.state.busy
    }

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 18) {
                VStack(alignment: .leading, spacing: 10) {
                    Text("Restore")
                        .font(.largeTitle.bold())
                    Text(replacingCurrentWallet ? "Restore from seed words and replace the wallet currently on this device." : "Restore your wallet from seed words.")
                        .font(.body)
                        .foregroundStyle(mutedText)
                    if replacingCurrentWallet {
                        Text("This clears local Bark wallet data before opening the restored wallet. Your Nostr profile and contacts stay on this device.")
                            .font(.body)
                            .foregroundStyle(rebelRed)
                    }
                }

                VStack(alignment: .leading, spacing: 10) {
                    Text("Recovery phrase")
                        .font(.headline)
                    SecureMultilineTextView(text: $phrase)
                        .frame(minHeight: 150)
                        .padding(10)
                        .background(surfaceBackground, in: RoundedRectangle(cornerRadius: 8))
                        .overlay(RoundedRectangle(cornerRadius: 8).stroke(borderColor))
                    Text("\(wordCount) words")
                        .font(.caption)
                        .foregroundStyle(wordCount >= 12 ? rebelGreen : mutedText)
                }

                Button {
                    if replacingCurrentWallet {
                        confirmingReplace = true
                    } else {
                        manager.dispatch(.restoreWallet(mnemonic: normalizedPhrase))
                    }
                } label: {
                    HStack {
                        if manager.state.busy {
                            ProgressView()
                                .tint(.white)
                        }
                        Label(replacingCurrentWallet ? "Replace wallet" : "Restore wallet", systemImage: "arrow.down.circle.fill")
                            .frame(maxWidth: .infinity)
                    }
                }
                .buttonStyle(PrimaryButtonStyle(color: replacingCurrentWallet ? rebelRed : rebelGreen))
                .disabled(!canRestore)

                if case .error(let message) = manager.state.setup {
                    Text(message)
                        .font(.footnote)
                        .foregroundStyle(rebelRed)
                        .multilineTextAlignment(.leading)
                }
            }
            .padding(16)
        }
        .navigationTitle("Restore")
        .scrollContentBackground(.hidden)
        .background(pageBackground)
        .foregroundStyle(primaryText)
        .alert("Replace wallet?", isPresented: $confirmingReplace) {
            Button("Replace", role: .destructive) {
                manager.dispatch(.replaceWallet(mnemonic: normalizedPhrase))
            }
            Button("Cancel", role: .cancel) {}
        } message: {
            Text("This removes the local wallet database on this device and restores from the seed words you entered.")
        }
    }
}

struct SecureMultilineTextView: UIViewRepresentable {
    @Binding var text: String

    func makeUIView(context: Context) -> UITextView {
        let textView = UITextView()
        textView.delegate = context.coordinator
        textView.backgroundColor = .clear
        textView.textColor = UIColor(primaryText)
        textView.tintColor = UIColor(rebelRed)
        textView.font = UIFont.monospacedSystemFont(ofSize: UIFont.preferredFont(forTextStyle: .body).pointSize, weight: .regular)
        textView.adjustsFontForContentSizeCategory = true
        textView.autocapitalizationType = .none
        textView.autocorrectionType = .no
        textView.spellCheckingType = .no
        textView.smartDashesType = .no
        textView.smartQuotesType = .no
        textView.smartInsertDeleteType = .no
        textView.keyboardType = .asciiCapable
        textView.textContentType = .password
        textView.isSecureTextEntry = true
        textView.returnKeyType = .done
        textView.textContainerInset = .zero
        textView.textContainer.lineFragmentPadding = 0
        DispatchQueue.main.async {
            textView.becomeFirstResponder()
        }
        return textView
    }

    func updateUIView(_ textView: UITextView, context: Context) {
        if textView.text != text {
            textView.text = text
        }
    }

    func makeCoordinator() -> Coordinator {
        Coordinator(text: $text)
    }

    final class Coordinator: NSObject, UITextViewDelegate {
        @Binding var text: String

        init(text: Binding<String>) {
            self._text = text
        }

        func textViewDidChange(_ textView: UITextView) {
            text = textView.text
        }
    }
}

struct SeedWordsPanel: View {
    let words: [String]
    @Binding var revealed: Bool
    @Binding var copied: Bool

    var body: some View {
        VStack(spacing: 16) {
            Button {
                revealed.toggle()
            } label: {
                Text(revealed ? "Hide seed words" : "Reveal seed words")
                    .font(.system(.body, design: .monospaced).weight(.semibold))
                    .frame(maxWidth: .infinity)
                    .padding(.vertical, 4)
            }

            if revealed {
                LazyVGrid(columns: [GridItem(.flexible()), GridItem(.flexible())], alignment: .leading, spacing: 10) {
                    ForEach(Array(words.enumerated()), id: \.offset) { index, word in
                        HStack(spacing: 8) {
                            Text("\(index + 1).")
                                .foregroundStyle(primaryText.opacity(0.65))
                                .frame(width: 28, alignment: .trailing)
                            Text(word)
                                .font(.system(.body, design: .monospaced).weight(.medium))
                            Spacer()
                        }
                    }
                }

                Button {
                    UIPasteboard.general.string = words.joined(separator: " ")
                    copied = true
                    DispatchQueue.main.asyncAfter(deadline: .now() + 1.2) {
                        copied = false
                    }
                } label: {
                    Label(copied ? "Copied" : "Copy", systemImage: "doc.on.doc")
                        .padding(.horizontal, 12)
                        .padding(.vertical, 8)
                        .background(Color.white.opacity(0.10), in: RoundedRectangle(cornerRadius: 8))
                }
            }
        }
        .padding(16)
        .background(rebelRed, in: RoundedRectangle(cornerRadius: 12))
        .foregroundStyle(primaryText)
    }
}

struct BackupCheckBox: View {
    @Binding var checked: Bool
    let text: String

    var body: some View {
        Button {
            checked.toggle()
        } label: {
            HStack(spacing: 12) {
                Image(systemName: checked ? "checkmark.square.fill" : "square")
                    .font(.title3)
                    .foregroundStyle(checked ? rebelRed : mutedText)
                Text(text)
                    .foregroundStyle(primaryText)
                Spacer()
            }
        }
    }
}

struct ProfileView: View {
    @Bindable var manager: AppManager
    @State private var mode: ProfileMode = .summary

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 18) {
                switch mode {
                case .summary:
                    ProfileSummaryPanel(manager: manager, mode: $mode)
                case .edit:
                    EditProfilePanel(manager: manager) {
                        mode = .summary
                    }
                case .keys:
                    NostrKeysPanel(manager: manager) {
                        mode = .summary
                    }
                }
            }
            .padding(16)
        }
        .navigationTitle(mode.title)
        .background(pageBackground)
        .foregroundStyle(primaryText)
    }
}

enum ProfileMode {
    case summary
    case edit
    case keys

    var title: String {
        switch self {
        case .summary: return "Profile"
        case .edit: return "Edit Profile"
        case .keys: return "Nostr Keys"
        }
    }
}

struct ProfileSummaryPanel: View {
    @Bindable var manager: AppManager
    @Binding var mode: ProfileMode

    var body: some View {
        VStack(spacing: 18) {
            VStack(spacing: 12) {
                ProfileAvatar(url: manager.state.nostr.picture, size: 128)
                Text(manager.state.nostr.name.isEmpty ? "Rebel" : manager.state.nostr.name)
                    .font(.largeTitle.bold())
                    .multilineTextAlignment(.center)
                if !manager.state.nostr.lud16.isEmpty {
                    Text(manager.state.nostr.lud16)
                        .font(.subheadline)
                        .foregroundStyle(rebelGreen)
                }
                if !manager.state.nostr.about.isEmpty {
                    Text(manager.state.nostr.about)
                        .font(.body)
                        .foregroundStyle(mutedText)
                        .multilineTextAlignment(.center)
                }
            }
            .frame(maxWidth: .infinity)
            .padding(.vertical, 10)

            VStack(spacing: 10) {
                ProfileActionRow(icon: "pencil", title: "Edit Profile", color: rebelRed) {
                    mode = .edit
                }
                ProfileActionRow(icon: "key.fill", title: "Nostr Keys", color: rebelBlue) {
                    mode = .keys
                }
            }

            BalancePanel(wallet: manager.state.wallet)
        }
    }
}

struct EditProfilePanel: View {
    @Bindable var manager: AppManager
    let done: () -> Void
    @State private var name = ""
    @State private var about = ""
    @State private var picture = ""
    @State private var lud16 = ""
    @State private var nip05 = ""

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            Button(action: done) {
                Label("Profile", systemImage: "chevron.left")
            }
            .buttonStyle(.plain)
            .foregroundStyle(mutedText)

            VStack(spacing: 14) {
                Button {
                    manager.dispatch(.requestPhotoPick)
                } label: {
                    ZStack(alignment: .bottomTrailing) {
                        ProfileAvatar(url: picture, size: 128)
                        Image(systemName: "pencil")
                            .font(.headline)
                            .padding(10)
                            .background(rebelRed, in: Circle())
                    }
                }
                .buttonStyle(.plain)

                TextField("Name", text: $name)
                    .profileField()
                TextField("About", text: $about, axis: .vertical)
                    .lineLimit(3...6)
                    .profileField()
                TextField("Picture URL", text: $picture)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                    .profileField()
                TextField("Lightning address", text: $lud16)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                    .profileField()
                TextField("NIP-05", text: $nip05)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                    .profileField()

                Button {
                    manager.dispatch(.editNostrProfile(name: name, about: about, picture: picture, lud16: lud16, nip05: nip05))
                    manager.dispatch(.publishNostrProfile)
                    done()
                } label: {
                    Text("Save")
                        .frame(maxWidth: .infinity)
                }
                .buttonStyle(PrimaryButtonStyle(color: rebelBlue))
            }
            .padding(14)
            .background(surfaceBackground, in: RoundedRectangle(cornerRadius: 12))
            .overlay(RoundedRectangle(cornerRadius: 12).stroke(borderColor))
        }
        .onAppear {
            name = manager.state.nostr.name
            about = manager.state.nostr.about
            picture = manager.state.nostr.picture
            lud16 = manager.state.nostr.lud16
            nip05 = manager.state.nostr.nip05
        }
        .onChange(of: manager.state.nostr.picture) { _, newValue in
            picture = newValue
        }
    }
}

struct NostrKeysPanel: View {
    @Bindable var manager: AppManager
    let done: () -> Void
    @State private var secret = ""
    @State private var confirmDelete = false

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            Button(action: done) {
                Label("Profile", systemImage: "chevron.left")
            }
            .buttonStyle(.plain)
            .foregroundStyle(mutedText)

            VStack(spacing: 14) {
                if let npub = manager.state.nostr.npub {
                    QRCodeView(text: npub)
                        .frame(maxWidth: .infinity)
                    KeyValueBlock(title: "Public Key", value: npub, hidden: false)
                } else {
                    EmptyState(text: "No Nostr key")
                }

                SecureField("Nostr private key (starts with nsec)", text: $secret)
                    .textContentType(.password)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                    .keyboardType(.asciiCapable)
                    .profileField()

                Button {
                    manager.dispatch(.importNostrSecret(nsecOrHex: secret))
                    secret = ""
                } label: {
                    Label("Import", systemImage: "square.and.arrow.down")
                        .frame(maxWidth: .infinity)
                }
                .buttonStyle(PrimaryButtonStyle(color: rebelBlue))
                .disabled(secret.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)

                HStack(spacing: 10) {
                    Button {
                        manager.dispatch(.generateNostrKey)
                    } label: {
                        Label("Generate", systemImage: "plus")
                            .frame(maxWidth: .infinity)
                    }
                    .buttonStyle(SecondaryButtonStyle())

                    Button {
                        manager.dispatch(.exportNostrSecret)
                    } label: {
                        Label("Export", systemImage: "square.and.arrow.up")
                            .frame(maxWidth: .infinity)
                    }
                    .buttonStyle(SecondaryButtonStyle())
                    .disabled(manager.state.nostr.npub == nil)
                }

                Button(role: .destructive) {
                    confirmDelete = true
                } label: {
                    Label("Delete published profile", systemImage: "person.crop.circle.badge.xmark")
                        .frame(maxWidth: .infinity)
                }
                .buttonStyle(SecondaryButtonStyle())
                .disabled(manager.state.nostr.npub == nil)

                Button(role: .destructive) {
                    manager.dispatch(.clearNostrKey)
                } label: {
                    Label("Unlink key", systemImage: "trash")
                        .frame(maxWidth: .infinity)
                }
                .buttonStyle(SecondaryButtonStyle())
                .disabled(manager.state.nostr.npub == nil)
            }
            .padding(14)
            .background(surfaceBackground, in: RoundedRectangle(cornerRadius: 12))
            .overlay(RoundedRectangle(cornerRadius: 12).stroke(borderColor))
        }
        .confirmationDialog("Delete published Nostr profile?", isPresented: $confirmDelete, titleVisibility: .visible) {
            Button("Delete published profile", role: .destructive) {
                manager.dispatch(.deleteNostrProfile)
            }
            Button("Cancel", role: .cancel) {}
        } message: {
            Text("This publishes a deletion event to configured relays.")
        }
    }
}

struct ProfileAvatar: View {
    let url: String
    let size: CGFloat

    var body: some View {
        ZStack {
            Circle()
                .fill(raisedSurface)
            if let parsed = URL(string: url), !url.isEmpty {
                AsyncImage(url: parsed) { image in
                    image
                        .resizable()
                        .scaledToFill()
                } placeholder: {
                    ProgressView()
                }
            } else {
                Text("R")
                    .font(.system(size: size * 0.42, weight: .bold))
                    .foregroundStyle(primaryText)
            }
        }
        .frame(width: size, height: size)
        .clipShape(Circle())
        .overlay(Circle().stroke(Color.white.opacity(0.20)))
    }
}

struct ProfileActionRow: View {
    let icon: String
    let title: String
    let color: Color
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            HStack(spacing: 12) {
                Image(systemName: icon)
                    .foregroundStyle(color)
                    .frame(width: 28)
                Text(title)
                    .font(.headline)
                Spacer()
                Image(systemName: "chevron.right")
                    .foregroundStyle(mutedText)
            }
            .padding(14)
            .background(surfaceBackground, in: RoundedRectangle(cornerRadius: 12))
            .overlay(RoundedRectangle(cornerRadius: 12).stroke(borderColor))
        }
        .buttonStyle(.plain)
    }
}

struct KeyValueBlock: View {
    let title: String
    let value: String
    let hidden: Bool

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text(title)
                .font(.caption)
                .foregroundStyle(mutedText)
            Text(hidden ? String(repeating: "*", count: min(value.count, 32)) : value)
                .font(.caption.monospaced())
                .textSelection(.enabled)
                .padding(10)
                .frame(maxWidth: .infinity, alignment: .leading)
                .background(raisedSurface, in: RoundedRectangle(cornerRadius: 8))
        }
    }
}

extension View {
    func profileField() -> some View {
        self
            .padding(12)
            .foregroundStyle(primaryText)
            .background(raisedSurface, in: RoundedRectangle(cornerRadius: 8))
            .overlay(RoundedRectangle(cornerRadius: 8).stroke(borderColor))
    }
}

struct ContactDetailView: View {
    @Bindable var manager: AppManager
    let contactId: String
    @State private var message = ""

    var contact: Contact? {
        manager.state.nostr.contacts.first { $0.id == contactId }
    }

    var messages: [NostrMessage] {
        manager.state.directMessages.filter { $0.contactId == contactId }
    }

    var body: some View {
        VStack(spacing: 0) {
            if let contact {
                ContactChatHeader(manager: manager, contact: contact)
                    .padding(.horizontal, 16)
                    .padding(.top, 12)
                    .padding(.bottom, 10)
                    .background(pageBackground.opacity(0.92))

                ScrollView {
                    VStack(spacing: 14) {
                        if messages.isEmpty {
                            Button {
                                manager.dispatch(.pushScreen(screen: .receive))
                            } label: {
                                HStack(spacing: 14) {
                                    Image(systemName: "message.badge")
                                        .foregroundStyle(rebelRed)
                                    Text("Send a message or request a payment to start this chat.")
                                        .font(.subheadline)
                                        .multilineTextAlignment(.leading)
                                    Spacer()
                                }
                                .padding(14)
                                .background(surfaceBackground, in: RoundedRectangle(cornerRadius: 12))
                            }
                            .buttonStyle(.plain)
                        } else {
                            ForEach(messages, id: \.id) { msg in
                                DirectMessageRow(message: msg)
                            }
                        }
                    }
                    .padding(16)
                }

                HStack(spacing: 10) {
                    Button {
                        if !contact.lightningAddress.isEmpty {
                            manager.dispatch(.setSendDestination(destination: contact.lightningAddress))
                            manager.dispatch(.pushScreen(screen: .send))
                        }
                    } label: {
                        Image(systemName: "plus")
                            .font(.title3)
                            .foregroundStyle(rebelRed)
                            .frame(width: 36, height: 36)
                    }
                    .disabled(contact.lightningAddress.isEmpty)

                    TextField("Message", text: $message, axis: .vertical)
                        .lineLimit(1...4)
                        .padding(12)
                        .background(surfaceBackground, in: RoundedRectangle(cornerRadius: 8))
                        .overlay(RoundedRectangle(cornerRadius: 8).stroke(borderColor))

                    Button {
                        manager.dispatch(.sendDirectMessage(contactId: contact.id, message: message))
                        message = ""
                    } label: {
                        Text("Send")
                    }
                    .buttonStyle(PrimaryButtonStyle(color: rebelBlue))
                    .disabled(message.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                }
                .padding(12)
                .background(pageBackground.opacity(0.94))
            } else {
                EmptyState(text: "Contact not found")
            }
        }
        .navigationTitle(contact?.name ?? "Contact")
        .background(pageBackground)
        .foregroundStyle(primaryText)
        .onAppear {
            manager.dispatch(.loadDirectMessages(contactId: contactId))
        }
    }
}

struct ContactChatHeader: View {
    @Bindable var manager: AppManager
    let contact: Contact

    var body: some View {
        VStack(spacing: 12) {
            HStack(spacing: 12) {
                ContactRow(contact: contact)
                Button {
                    manager.dispatch(.loadDirectMessages(contactId: contact.id))
                } label: {
                    Image(systemName: "arrow.clockwise")
                        .frame(width: 36, height: 36)
                }
                .buttonStyle(.plain)
            }

            HStack(spacing: 16) {
                Button {
                    if !contact.lightningAddress.isEmpty {
                        manager.dispatch(.setSendDestination(destination: contact.lightningAddress))
                        manager.dispatch(.pushScreen(screen: .send))
                    }
                } label: {
                    Label("Send", systemImage: "arrow.up.right")
                }
                .foregroundStyle(rebelGreen)
                .disabled(contact.lightningAddress.isEmpty)

                Button {
                    manager.dispatch(.pushScreen(screen: .receive))
                } label: {
                    Label("Request", systemImage: "arrow.down.left")
                }
                .foregroundStyle(rebelBlue)

                Button {
                    if contact.followed {
                        manager.dispatch(.unfollowContact(contactId: contact.id))
                    } else {
                        manager.dispatch(.followContact(contactId: contact.id))
                    }
                } label: {
                    Label(contact.followed ? "Unfollow" : "Follow", systemImage: contact.followed ? "xmark" : "checkmark")
                }
                .foregroundStyle(contact.followed ? rebelRed : primaryText)

                Spacer()

                Button(role: .destructive) {
                    manager.dispatch(.deleteContact(contactId: contact.id))
                    manager.dispatch(.popScreen)
                } label: {
                    Image(systemName: "trash")
                }
            }
            .font(.subheadline.bold())
        }
    }
}

struct RebelMark: View {
    let size: CGFloat

    var body: some View {
        ZStack {
            RoundedRectangle(cornerRadius: 8)
                .fill(rebelRed)
            Text("R")
                .font(.system(size: size * 0.55, weight: .black, design: .rounded))
                .foregroundStyle(.white)
        }
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

struct PrimaryButtonStyle: ButtonStyle {
    let color: Color

    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .font(.headline)
            .foregroundStyle(.white)
            .padding(.vertical, 14)
            .padding(.horizontal, 16)
            .background(color.opacity(configuration.isPressed ? 0.82 : 1), in: RoundedRectangle(cornerRadius: 8))
    }
}

struct StatPill: View {
    let title: String
    let value: String

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            Text(title)
                .font(.caption)
                .foregroundStyle(mutedText)
            Text(value)
                .font(.subheadline.bold())
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
        item.counterpartyKnown && !item.counterpartyPicture.isEmpty
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
                    ProfileAvatar(url: item.counterpartyPicture, size: 48)
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
                    .foregroundStyle(item.counterpartyKnown || primaryName == "You" ? primaryText : mutedText)
                    Text(" \(verb) ")
                        .font(.subheadline.weight(.light))
                        .foregroundStyle(primaryText)
                    Text(secondaryName)
                        .font(.subheadline.bold())
                    .foregroundStyle(item.counterpartyKnown || secondaryName == "you" ? primaryText : mutedText)
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

    var body: some View {
        HStack(spacing: 12) {
            Circle()
                .fill(rebelBlue.opacity(0.28))
                .frame(width: 42, height: 42)
                .overlay(Text(String(contact.name.prefix(1))).font(.headline).foregroundStyle(primaryText))
            VStack(alignment: .leading) {
                Text(contact.name)
                    .font(.subheadline.bold())
                Text(contact.lightningAddress.isEmpty ? (contact.followed ? "Following" : "Not following") : contact.lightningAddress)
                    .font(.caption)
                    .foregroundStyle(mutedText)
            }
            Spacer()
        }
    }
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

struct QRCodeView: View {
    let text: String
    private let context = CIContext()
    private let filter = CIFilter.qrCodeGenerator()

    var body: some View {
        if let image = makeImage() {
            Image(uiImage: image)
                .interpolation(.none)
                .resizable()
                .scaledToFit()
                .frame(width: 220, height: 220)
                .padding(12)
                .background(.white, in: RoundedRectangle(cornerRadius: 8))
        }
    }

    private func makeImage() -> UIImage? {
        filter.message = Data(text.utf8)
        guard let output = filter.outputImage else { return nil }
        let scaled = output.transformed(by: CGAffineTransform(scaleX: 10, y: 10))
        guard let cgImage = context.createCGImage(scaled, from: scaled.extent) else { return nil }
        return UIImage(cgImage: cgImage)
    }
}

struct QRScannerSheet: View {
    let onScan: (String) -> Void

    var body: some View {
        NavigationStack {
            QRScannerView(onScan: onScan)
                .ignoresSafeArea(edges: .bottom)
                .navigationTitle("Scan payment")
                .navigationBarTitleDisplayMode(.inline)
        }
    }
}

struct QRScannerView: UIViewControllerRepresentable {
    let onScan: (String) -> Void

    func makeUIViewController(context: Context) -> QRScannerViewController {
        let controller = QRScannerViewController()
        controller.onScan = onScan
        return controller
    }

    func updateUIViewController(_ uiViewController: QRScannerViewController, context: Context) {}
}

final class QRScannerViewController: UIViewController, AVCaptureMetadataOutputObjectsDelegate {
    var onScan: ((String) -> Void)?
    private let session = AVCaptureSession()
    private var previewLayer: AVCaptureVideoPreviewLayer?
    private var didScan = false

    override func viewDidLoad() {
        super.viewDidLoad()
        view.backgroundColor = .black
        configureSession()
    }

    override func viewDidLayoutSubviews() {
        super.viewDidLayoutSubviews()
        previewLayer?.frame = view.bounds
    }

    override func viewWillAppear(_ animated: Bool) {
        super.viewWillAppear(animated)
        didScan = false
        if !session.isRunning {
            DispatchQueue.global(qos: .userInitiated).async { [session] in
                session.startRunning()
            }
        }
    }

    override func viewWillDisappear(_ animated: Bool) {
        super.viewWillDisappear(animated)
        if session.isRunning {
            DispatchQueue.global(qos: .userInitiated).async { [session] in
                session.stopRunning()
            }
        }
    }

    private func configureSession() {
        guard let device = AVCaptureDevice.default(for: .video),
              let input = try? AVCaptureDeviceInput(device: device),
              session.canAddInput(input) else {
            showUnavailable()
            return
        }
        session.addInput(input)

        let output = AVCaptureMetadataOutput()
        guard session.canAddOutput(output) else {
            showUnavailable()
            return
        }
        session.addOutput(output)
        output.setMetadataObjectsDelegate(self, queue: .main)
        output.metadataObjectTypes = [.qr]

        let layer = AVCaptureVideoPreviewLayer(session: session)
        layer.videoGravity = .resizeAspectFill
        view.layer.insertSublayer(layer, at: 0)
        previewLayer = layer
    }

    private func showUnavailable() {
        let label = UILabel()
        label.text = "Camera unavailable"
        label.textColor = .white
        label.textAlignment = .center
        label.translatesAutoresizingMaskIntoConstraints = false
        view.addSubview(label)
        NSLayoutConstraint.activate([
            label.centerXAnchor.constraint(equalTo: view.centerXAnchor),
            label.centerYAnchor.constraint(equalTo: view.centerYAnchor)
        ])
    }

    func metadataOutput(
        _ output: AVCaptureMetadataOutput,
        didOutput metadataObjects: [AVMetadataObject],
        from connection: AVCaptureConnection
    ) {
        guard !didScan,
              let readable = metadataObjects.first as? AVMetadataMachineReadableCodeObject,
              let value = readable.stringValue else {
            return
        }
        didScan = true
        onScan?(value)
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
