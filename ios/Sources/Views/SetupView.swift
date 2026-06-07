import SwiftUI

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

            if manager.state.busy.bootstrapping || manager.state.busy.openingWallet {
                ProgressView()
            }
            Spacer()
        }
        .padding(22)
        .foregroundStyle(primaryText)
        .background(pageBackground.ignoresSafeArea())
    }
}
