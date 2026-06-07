import SwiftUI

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
                Text("Paste or scan an Ark address or Lightning invoice.")
                    .font(.subheadline)
                    .foregroundStyle(mutedText)

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
