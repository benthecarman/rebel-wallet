import SwiftUI
import UIKit

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
        manager.state.receive.receiveRequest
    }

    var body: some View {
        ScrollView {
            VStack(spacing: 22) {
                if showingResult {
                    HStack(alignment: .center) {
                        Spacer()
                        Button("Edit") {
                            manager.dispatch(.editReceiveRequest)
                        }
                        .buttonStyle(SecondaryButtonStyle())
                    }
                    .frame(maxWidth: .infinity, alignment: .trailing)
                }

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
        manager.dispatch(.selectTab(tab: .home))
        manager.dispatch(.updateScreenStack(stack: []))
        manager.dispatch(.dismissPaymentSuccess)
        manager.dispatch(.setReceiveAmount(amountSat: 0))
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
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .frame(maxWidth: .infinity)
            }
        }
        .clipShape(RoundedRectangle(cornerRadius: 8))
        .background(Color.black, in: RoundedRectangle(cornerRadius: 8))
        .overlay(RoundedRectangle(cornerRadius: 8).stroke(borderColor))
    }
}
