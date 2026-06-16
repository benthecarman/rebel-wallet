import SwiftUI
import UIKit

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
                    Text("Your recovery phrase controls your funds. Write these words down and keep them offline.")
                        .font(.body)
                        .foregroundStyle(mutedText)
                    Text("Anyone with these words can restore and spend from this wallet.")
                        .font(.body)
                        .foregroundStyle(mutedText)
                }

                SeedWordsPanel(words: words, revealed: $revealed, copied: $copied) { feedback in
                    manager.requestHaptic(feedback)
                }

                if revealed {
                    VStack(alignment: .leading, spacing: 12) {
                        BackupCheckBox(checked: $checkedSecure, text: "I wrote the words down.") {
                            manager.requestHaptic(.selection)
                        }
                        BackupCheckBox(checked: $checkedResponsibility, text: "I understand Rebel cannot recover them.") {
                            manager.requestHaptic(.selection)
                        }
                        BackupCheckBox(checked: $checkedPrivate, text: "I will not share them with anyone.") {
                            manager.requestHaptic(.selection)
                        }
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
    @Environment(\.walletAccent) private var walletAccent

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
        wordCount >= 12 && !manager.state.busy.openingWallet
    }

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 18) {
                VStack(alignment: .leading, spacing: 10) {
                    Text(replacingCurrentWallet ? "Restore from seed words and replace the wallet currently on this device." : "Restore your wallet from seed words.")
                        .font(.body)
                        .foregroundStyle(mutedText)
                    if replacingCurrentWallet {
                        Text("This clears local Bark wallet data before opening the restored wallet. Your Nostr profile and contacts stay on this device.")
                            .font(.body)
                            .foregroundStyle(walletAccent)
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
                        if manager.state.busy.openingWallet {
                            ProgressView()
                                .tint(.white)
                        }
                        Label(replacingCurrentWallet ? "Replace wallet" : "Restore wallet", systemImage: "arrow.down.circle.fill")
                            .frame(maxWidth: .infinity)
                    }
                }
                .buttonStyle(PrimaryButtonStyle(color: replacingCurrentWallet ? walletAccent : rebelGreen))
                .disabled(!canRestore)

                if case .error(let message) = manager.state.setup {
                    Text(message)
                        .font(.footnote)
                        .foregroundStyle(walletAccent)
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
    @Environment(\.walletAccent) private var walletAccent

    func makeUIView(context: Context) -> UITextView {
        let textView = UITextView()
        textView.delegate = context.coordinator
        textView.backgroundColor = .clear
        textView.textColor = UIColor(primaryText)
        textView.tintColor = UIColor(walletAccent)
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
        textView.tintColor = UIColor(walletAccent)
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
    let onHaptic: (HapticFeedback) -> Void
    @Environment(\.walletAccent) private var walletAccent

    var body: some View {
        VStack(spacing: 16) {
            Button {
                revealed.toggle()
                onHaptic(revealed ? .notificationWarning : .selection)
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
                    onHaptic(.impactLight)
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
        .background(walletAccent, in: RoundedRectangle(cornerRadius: 12))
        .foregroundStyle(primaryText)
    }
}

struct BackupCheckBox: View {
    @Binding var checked: Bool
    let text: String
    let onToggle: () -> Void
    @Environment(\.walletAccent) private var walletAccent

    var body: some View {
        Button {
            checked.toggle()
            onToggle()
        } label: {
            HStack(spacing: 12) {
                Image(systemName: checked ? "checkmark.square.fill" : "square")
                    .font(.title3)
                    .foregroundStyle(checked ? walletAccent : mutedText)
                Text(text)
                    .foregroundStyle(primaryText)
                Spacer()
            }
        }
    }
}
