import SwiftUI

struct ContactDetailView: View {
    @Bindable var manager: AppManager
    let contactId: String
    @State private var message = ""
    @Environment(\.walletAccent) private var walletAccent

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
                                        .foregroundStyle(walletAccent)
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
                            .foregroundStyle(walletAccent)
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
    @Environment(\.walletAccent) private var walletAccent

    var body: some View {
        VStack(spacing: 12) {
            HStack(spacing: 12) {
                ContactRow(contact: contact, imageNormalizer: manager.rust)
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
                .foregroundStyle(contact.followed ? walletAccent : primaryText)

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
