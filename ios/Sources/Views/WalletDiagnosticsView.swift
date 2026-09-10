import SwiftUI
import UIKit

struct WalletDiagnosticsView: View {
    @Bindable var manager: AppManager
    @State private var confirmingRefresh = false

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 16) {
                HStack {
                    Button("Reload local records") {
                        manager.dispatch(.reloadWalletDiagnostics)
                    }
                    .disabled(manager.state.walletDiagnosticsLoading)
                    Spacer()
                    Button {
                        UIPasteboard.general.string = manager.state.walletRefreshStatus + "\n\n" + manager.state.walletDiagnostics
                    } label: {
                        Label("Copy report", systemImage: "doc.on.doc")
                    }
                    .disabled(manager.state.walletDiagnostics.isEmpty || manager.state.walletDiagnosticsLoading)
                }
                if manager.state.walletDiagnosticsLoading {
                    ProgressView("Reading wallet records…")
                }
                Button("Force refresh VTXOs") {
                    confirmingRefresh = true
                }
                .disabled(manager.state.walletRefreshRunning || manager.state.busy.syncingWallet || manager.state.busy.sendingPayment)
                .confirmationDialog("Refresh wallet VTXOs?", isPresented: $confirmingRefresh, titleVisibility: .visible) {
                    Button("Request refresh") {
                        manager.dispatch(.forceRefreshWalletVtxos)
                    }
                    Button("Cancel", role: .cancel) {}
                } message: {
                    Text("Requests new VTXOs for locally spendable funds, even if they are not due for refresh. Fees may apply and funds may be temporarily unavailable. This cannot override a server rejection of spent VTXOs.")
                }
                if manager.state.walletRefreshRunning {
                    ProgressView("Requesting refresh…")
                }
                if !manager.state.walletRefreshStatus.isEmpty {
                    Text(manager.state.walletRefreshStatus)
                        .textSelection(.enabled)
                }
                Text(manager.state.walletDiagnostics)
                    .font(.system(.footnote, design: .monospaced))
                    .textSelection(.enabled)
                    .frame(maxWidth: .infinity, alignment: .leading)
            }
            .padding(16)
        }
        .navigationTitle("Wallet Diagnostics")
        .background(pageBackground)
        .foregroundStyle(primaryText)
    }
}
