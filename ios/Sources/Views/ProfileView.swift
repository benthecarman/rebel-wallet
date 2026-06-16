import SwiftUI
import UIKit

struct ProfileView: View {
    @Bindable var manager: AppManager
    var close: (() -> Void)? = nil
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
        .toolbar {
            if let close {
                ToolbarItem(placement: .topBarLeading) {
                    Button(action: close) {
                        Label("Back", systemImage: "chevron.left")
                    }
                    .tint(mutedText)
                }
            }
        }
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
    @Environment(\.walletAccent) private var walletAccent

    var body: some View {
        VStack(spacing: 18) {
            VStack(spacing: 12) {
                ProfileAvatar(url: manager.state.nostr.pictureDisplayUrl, size: 128, imageNormalizer: manager.rust)
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
                if manager.state.nostr.deleted {
                    DeletedProfileNotice()
                } else {
                    ProfileActionRow(icon: "pencil", title: "Edit Profile", color: walletAccent) {
                        mode = .edit
                    }
                }
                ProfileActionRow(icon: "key.fill", title: "Nostr Keys", color: rebelBlue) {
                    mode = .keys
                }
            }

            LightningAddressPanel(manager: manager)
            BalancePanel(wallet: manager.state.wallet)
        }
    }
}

struct LightningAddressPanel: View {
    @Bindable var manager: AppManager
    @Environment(\.walletAccent) private var walletAccent

    private var claimedAddress: String? {
        manager.state.lightningAddress.address
    }

    private var domain: String {
        claimedAddress?
            .split(separator: "@")
            .last
            .map(String.init) ?? "arkzap.me"
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            HStack(spacing: 10) {
                Image(systemName: "bolt.badge.checkmark")
                    .foregroundStyle(rebelGreen)
                    .frame(width: 28)
                VStack(alignment: .leading, spacing: 3) {
                    Text("Lightning Address")
                        .font(.headline)
                    Text(domain)
                        .font(.caption)
                        .foregroundStyle(mutedText)
                        .lineLimit(1)
                }
                Spacer()
            }

            if let claimedAddress {
                Text(truncateLightningAddress(claimedAddress))
                    .font(.caption.monospaced())
                    .textSelection(.enabled)
                    .padding(10)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .background(raisedSurface, in: RoundedRectangle(cornerRadius: 8))

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
            }
        }
        .padding(14)
        .background(surfaceBackground, in: RoundedRectangle(cornerRadius: 12))
        .overlay(RoundedRectangle(cornerRadius: 12).stroke(borderColor))
    }
}

private func truncateLightningAddress(_ value: String) -> String {
    let maxLength = 34
    guard value.count > maxLength else { return value }
    let prefixCount = 14
    let suffixCount = maxLength - prefixCount - 3
    return "\(value.prefix(prefixCount))...\(value.suffix(suffixCount))"
}

struct EditProfilePanel: View {
    @Bindable var manager: AppManager
    let done: () -> Void
    @State private var name = ""
    @State private var about = ""
    @State private var picture = ""
    @State private var lightningAddress = ""
    @State private var nip05 = ""
    @State private var confirmDelete = false
    @Environment(\.walletAccent) private var walletAccent

    private var arkLightningAddress: String? {
        manager.state.lightningAddress.address
    }

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
                        ProfileAvatar(
                            url: picture,
                            size: 128,
                            imageNormalizer: manager.rust,
                            allowRemotePreview: true
                        )
                        Image(systemName: "pencil")
                            .font(.headline)
                            .padding(10)
                            .background(walletAccent, in: Circle())
                    }
                }
                .buttonStyle(.plain)

                TextField("Name", text: $name)
                    .profileField()
                TextField("About", text: $about, axis: .vertical)
                    .lineLimit(3...6)
                    .profileField()

                HStack(spacing: 10) {
                    TextField("Lightning Address", text: $lightningAddress)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                        .keyboardType(.emailAddress)
                        .profileField()

                    Button {
                        if let arkLightningAddress {
                            lightningAddress = arkLightningAddress
                            manager.requestHaptic(.selection)
                        }
                    } label: {
                        Label("Use Arkzap", systemImage: "bolt.badge.checkmark")
                            .labelStyle(.iconOnly)
                            .frame(width: 44, height: 44)
                            .foregroundStyle(primaryText)
                            .background(raisedSurface, in: RoundedRectangle(cornerRadius: 8))
                            .overlay(RoundedRectangle(cornerRadius: 8).stroke(borderColor))
                    }
                    .buttonStyle(.plain)
                    .disabled(arkLightningAddress == nil)
                    .accessibilityLabel("Use Arkzap Lightning address")
                }

                TextField("NIP-05", text: $nip05)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                    .profileField()

                Button {
                    manager.dispatch(.editNostrProfile(name: name, about: about, picture: picture, lud16: lightningAddress, nip05: nip05))
                    manager.dispatch(.publishNostrProfile)
                    done()
                } label: {
                    Text("Save")
                        .frame(maxWidth: .infinity)
                }
                .buttonStyle(PrimaryButtonStyle(color: rebelBlue))

                Button(role: .destructive) {
                    confirmDelete = true
                } label: {
                    Label("Delete Profile", systemImage: "person.crop.circle.badge.xmark")
                        .frame(maxWidth: .infinity)
                }
                .buttonStyle(SecondaryButtonStyle())
                .disabled(manager.state.nostr.npub == nil)
            }
            .padding(14)
            .background(surfaceBackground, in: RoundedRectangle(cornerRadius: 12))
            .overlay(RoundedRectangle(cornerRadius: 12).stroke(borderColor))
        }
        .confirmationDialog("Delete profile?", isPresented: $confirmDelete, titleVisibility: .visible) {
            Button("Delete profile", role: .destructive) {
                manager.dispatch(.deleteNostrProfile)
                done()
            }
            Button("Cancel", role: .cancel) {}
        } message: {
            Text("This publishes a deleted profile to Nostr relays and permanently disables profile editing in this app.")
        }
        .onAppear {
            name = manager.state.nostr.name
            about = manager.state.nostr.about
            picture = manager.state.nostr.picture
            lightningAddress = manager.state.nostr.lud16
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
                    Label("Import override", systemImage: "square.and.arrow.down")
                        .frame(maxWidth: .infinity)
                }
                .buttonStyle(PrimaryButtonStyle(color: rebelBlue))
                .disabled(secret.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)

                Button {
                    manager.dispatch(.exportNostrSecret)
                } label: {
                    Label("Export", systemImage: "square.and.arrow.up")
                        .frame(maxWidth: .infinity)
                }
                .buttonStyle(SecondaryButtonStyle())
                .disabled(manager.state.nostr.npub == nil)

                Button(role: .destructive) {
                    presentClearProfileCacheConfirmation()
                } label: {
                    Label("Clear profile cache", systemImage: "xmark.bin")
                        .frame(maxWidth: .infinity)
                }
                .buttonStyle(SecondaryButtonStyle())

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
    }

    private func presentClearProfileCacheConfirmation() {
        guard let presenter = UIApplication.shared.rebelTopViewController() else {
            return
        }

        let sheet = UIAlertController(
            title: "Clear cached Nostr profiles?",
            message: nil,
            preferredStyle: .actionSheet
        )
        sheet.addAction(UIAlertAction(title: "Clear Profile Cache", style: .destructive) { _ in
            manager.dispatch(.clearNostrProfileCache)
        })
        sheet.addAction(UIAlertAction(title: "Cancel", style: .cancel))

        if UIDevice.current.userInterfaceIdiom == .pad, let popover = sheet.popoverPresentationController {
            popover.sourceView = presenter.view
            popover.sourceRect = CGRect(
                x: presenter.view.bounds.midX,
                y: presenter.view.bounds.maxY,
                width: 1,
                height: 1
            )
            popover.permittedArrowDirections = []
        }

        presenter.present(sheet, animated: true)
    }
}

private extension UIApplication {
    func rebelTopViewController() -> UIViewController? {
        connectedScenes
            .compactMap { $0 as? UIWindowScene }
            .flatMap(\.windows)
            .first { $0.isKeyWindow }?
            .rootViewController?
            .rebelTopPresentedViewController()
    }
}

private extension UIViewController {
    func rebelTopPresentedViewController() -> UIViewController {
        if let presentedViewController {
            return presentedViewController.rebelTopPresentedViewController()
        }
        if let navigationController = self as? UINavigationController {
            return navigationController.visibleViewController?.rebelTopPresentedViewController() ?? navigationController
        }
        if let tabBarController = self as? UITabBarController {
            return tabBarController.selectedViewController?.rebelTopPresentedViewController() ?? tabBarController
        }
        return self
    }
}

struct DeletedProfileNotice: View {
    var body: some View {
        HStack(spacing: 12) {
            Image(systemName: "person.crop.circle.badge.xmark")
                .foregroundStyle(mutedText)
                .frame(width: 28)
            Text("Profile deleted")
                .font(.headline)
            Spacer()
        }
        .padding(14)
        .background(surfaceBackground, in: RoundedRectangle(cornerRadius: 12))
        .overlay(RoundedRectangle(cornerRadius: 12).stroke(borderColor))
    }
}

struct ProfileAvatar: View {
    let url: String
    let size: CGFloat
    var initial: String = "R"
    var imageNormalizer: FfiApp? = nil
    var allowRemotePreview = false

    @StateObject private var loader = ProfileImageLoader()

    var body: some View {
        ZStack {
            Circle()
                .fill(raisedSurface)
            if let image = loader.image {
                Image(uiImage: image)
                    .resizable()
                    .scaledToFill()
            } else if loader.isLoading {
                ProgressView()
            } else {
                Text(initial.isEmpty ? "R" : initial)
                    .font(.system(size: size * 0.42, weight: .bold))
                    .foregroundStyle(primaryText)
            }
        }
        .frame(width: size, height: size)
        .clipShape(Circle())
        .overlay(Circle().stroke(Color.white.opacity(0.20)))
        .task(id: url) {
            if let parsed = URL(string: url), !url.isEmpty {
                loader.load(parsed, normalizer: imageNormalizer, allowRemote: allowRemotePreview)
            } else {
                loader.reset()
            }
        }
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
