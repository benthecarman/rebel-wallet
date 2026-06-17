import SwiftUI
import UIKit

struct LightningAddressView: View {
    @Bindable var manager: AppManager
    @Environment(\.walletAccent) private var walletAccent

    private var lightning: LightningAddressState {
        manager.state.lightningAddress
    }

    private var claimedAddress: String? {
        lightning.address
    }

    private var addressKind: String {
        lightning.customAddress == nil ? "Arkzap" : "Custom"
    }

    private var domain: String {
        claimedAddress?
            .split(separator: "@")
            .last
            .map(String.init) ?? "arkzap.me"
    }

    private var customName: Binding<String> {
        Binding(
            get: { lightning.customName },
            set: { manager.dispatch(.setLightningAddressName(name: $0)) }
        )
    }

    private var registrationBusy: Bool {
        lightning.registrationPhase == .registering || lightning.registrationPhase == .verifying
    }

    private var registerButtonTitle: String {
        switch lightning.registrationPhase {
        case .registering: return "Registering"
        case .verifying: return "Checking"
        case .awaitingPayment: return "Retry"
        case .active: return "Register"
        case .idle: return "Register"
        }
    }

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 18) {
                currentAddressSection
                customAddressSection

                if lightning.registrationPhase == .awaitingPayment,
                   let invoice = lightning.registrationInvoice,
                   !invoice.isEmpty {
                    LightningRegistrationInvoicePanel(invoice: invoice, manager: manager)
                }
            }
            .padding(16)
        }
        .navigationTitle("Lightning Address")
        .background(pageBackground)
        .foregroundStyle(primaryText)
    }

    private var currentAddressSection: some View {
        VStack(alignment: .leading, spacing: 14) {
            LightningAddressSectionHeader(
                icon: "bolt.badge.checkmark",
                title: "Current",
                caption: domain,
                color: rebelGreen
            )

            if let claimedAddress {
                VStack(alignment: .leading, spacing: 8) {
                    Text(addressKind)
                        .font(.caption.bold())
                        .foregroundStyle(mutedText)
                    Text(claimedAddress)
                        .font(.system(.body, design: .monospaced))
                        .textSelection(.enabled)
                        .lineLimit(3)
                        .fixedSize(horizontal: false, vertical: true)
                }
                .padding(14)
                .frame(maxWidth: .infinity, alignment: .leading)
                .background(raisedSurface, in: RoundedRectangle(cornerRadius: 8))
                .overlay(RoundedRectangle(cornerRadius: 8).stroke(borderColor))

                HStack(spacing: 10) {
                    Button {
                        UIPasteboard.general.string = claimedAddress
                        manager.requestHaptic(.impactLight)
                    } label: {
                        Label("Copy", systemImage: "doc.on.doc")
                            .frame(maxWidth: .infinity)
                    }
                    .buttonStyle(SecondaryButtonStyle())

                    ShareLink(item: claimedAddress) {
                        Label("Share", systemImage: "square.and.arrow.up")
                            .frame(maxWidth: .infinity)
                    }
                    .buttonStyle(SecondaryButtonStyle())
                }
            } else {
                HStack(spacing: 10) {
                    ProgressView()
                    Text("Preparing Arkzap address")
                        .font(.caption)
                        .foregroundStyle(mutedText)
                }
                .padding(.vertical, 4)
            }
        }
        .padding(16)
        .background(surfaceBackground, in: RoundedRectangle(cornerRadius: 8))
        .overlay(RoundedRectangle(cornerRadius: 8).stroke(borderColor))
    }

    private var customAddressSection: some View {
        VStack(alignment: .leading, spacing: 14) {
            LightningAddressSectionHeader(
                icon: "checkmark.seal.fill",
                title: "Custom",
                caption: lightning.registrationStatusText,
                color: walletAccent
            )

            ViewThatFits(in: .horizontal) {
                HStack(spacing: 10) {
                    customNameField
                    registerButton
                }

                VStack(spacing: 10) {
                    customNameField
                    registerButton
                }
            }

            registrationStatusRow

            if let nameError = lightning.registrationNameError, !lightning.customName.isEmpty {
                Text(nameError)
                    .font(.caption)
                    .foregroundStyle(rebelRed)
            }

            if let registrationError = lightning.registrationError {
                Text(registrationError)
                    .font(.caption)
                    .foregroundStyle(rebelRed)
            }
        }
        .padding(16)
        .background(surfaceBackground, in: RoundedRectangle(cornerRadius: 8))
        .overlay(RoundedRectangle(cornerRadius: 8).stroke(borderColor))
    }

    private var customNameField: some View {
        TextField("custom name", text: customName)
            .textInputAutocapitalization(.never)
            .autocorrectionDisabled()
            .keyboardType(.asciiCapable)
            .profileField()
    }

    private var registerButton: some View {
        Button {
            manager.dispatch(.registerLightningAddress)
        } label: {
            HStack(spacing: 7) {
                if registrationBusy {
                    ProgressView()
                        .controlSize(.small)
                        .tint(.white)
                } else {
                    Image(systemName: "checkmark.seal.fill")
                }
                Text(registerButtonTitle)
            }
            .frame(minWidth: 118)
        }
        .buttonStyle(PrimaryButtonStyle(color: walletAccent))
        .disabled(!lightning.registrationCanSubmit)
    }

    private var registrationStatusRow: some View {
        HStack(spacing: 8) {
            Text(lightning.registrationStatusText)
                .font(.caption.bold())
                .foregroundStyle(lightning.registrationPhase == .active ? rebelGreen : mutedText)
            if lightning.registrationAmountSat > 0 {
                Text(lightning.registrationAmountDisplay)
                    .font(.caption)
                    .foregroundStyle(mutedText)
            }
            if let address = lightning.registrationAddress, lightning.registrationPhase != .active {
                Text(truncateLightningAddress(address))
                    .font(.caption.monospaced())
                    .foregroundStyle(mutedText)
                    .lineLimit(1)
            }
            Spacer()
            if lightning.registrationCanCheckStatus {
                Button {
                    manager.dispatch(.verifyLightningAddressRegistration)
                } label: {
                    Image(systemName: "arrow.clockwise")
                        .frame(width: 32, height: 32)
                }
                .buttonStyle(.plain)
                .foregroundStyle(primaryText)
                .accessibilityLabel("Check registration status")
            }
        }
    }
}

private struct LightningAddressSectionHeader: View {
    let icon: String
    let title: String
    let caption: String
    let color: Color

    var body: some View {
        HStack(spacing: 10) {
            Image(systemName: icon)
                .foregroundStyle(color)
                .frame(width: 28)
            VStack(alignment: .leading, spacing: 3) {
                Text(title)
                    .font(.headline)
                Text(caption)
                    .font(.caption)
                    .foregroundStyle(mutedText)
                    .lineLimit(1)
            }
            Spacer()
        }
    }
}

private struct LightningRegistrationInvoicePanel: View {
    let invoice: String
    @Bindable var manager: AppManager

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            LightningAddressSectionHeader(
                icon: "qrcode",
                title: "Payment",
                caption: "Pending registration",
                color: rebelGreen
            )

            QRCodeView(text: invoice)
                .frame(maxWidth: .infinity)

            Text(invoice)
                .font(.caption.monospaced())
                .lineLimit(5)
                .textSelection(.enabled)
                .padding(10)
                .frame(maxWidth: .infinity, alignment: .leading)
                .background(Color.black, in: RoundedRectangle(cornerRadius: 8))
                .overlay(RoundedRectangle(cornerRadius: 8).stroke(borderColor))

            HStack(spacing: 10) {
                Button {
                    UIPasteboard.general.string = invoice
                    manager.requestHaptic(.impactLight)
                } label: {
                    Label("Copy", systemImage: "doc.on.doc")
                        .frame(maxWidth: .infinity)
                }
                .buttonStyle(SecondaryButtonStyle())

                ShareLink(item: invoice) {
                    Label("Share", systemImage: "square.and.arrow.up")
                        .frame(maxWidth: .infinity)
                }
                .buttonStyle(SecondaryButtonStyle())
            }
        }
        .padding(16)
        .background(surfaceBackground, in: RoundedRectangle(cornerRadius: 8))
        .overlay(RoundedRectangle(cornerRadius: 8).stroke(borderColor))
    }
}

private func truncateLightningAddress(_ value: String) -> String {
    let maxLength = 34
    guard value.count > maxLength else { return value }
    let prefixCount = 14
    let suffixCount = maxLength - prefixCount - 3
    return "\(value.prefix(prefixCount))...\(value.suffix(suffixCount))"
}
