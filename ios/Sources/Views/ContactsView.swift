import SwiftUI

struct ContactsView: View {
    @Bindable var manager: AppManager
    @State private var query = ""
    @State private var npub = ""
    @State private var name = ""
    @State private var lightningAddress = ""
    @State private var adding = false
    @Environment(\.walletAccent) private var walletAccent

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
                    manager.requestHaptic(.selection)
                    adding.toggle()
                } label: {
                    HStack(spacing: 12) {
                        Image(systemName: adding ? "minus" : "plus")
                            .foregroundStyle(walletAccent)
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
                            manager.dispatch(.addContact(npub: npub, name: name, lightningAddress: lightningAddress, lnurl: "", picture: ""))
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
                                ContactRow(contact: contact, imageNormalizer: manager.rust)
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
