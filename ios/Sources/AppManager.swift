import Foundation
import Observation
import Security

@MainActor
@Observable
final class AppManager: AppReconciler {
    let rust: FfiApp
    var state: AppState
    private var lastRevApplied: UInt64

    init() {
        let fm = FileManager.default
        let dataDirUrl = fm.urls(for: .applicationSupportDirectory, in: .userDomainMask).first!
        let dataDir = dataDirUrl.path
        try? fm.createDirectory(at: dataDirUrl, withIntermediateDirectories: true)

        let rust = FfiApp(dataDir: dataDir, secretStore: KeychainSecretStore())
        self.rust = rust

        let initial = rust.state()
        self.state = initial
        self.lastRevApplied = initial.rev

        rust.listenForUpdates(reconciler: self)
        rust.dispatch(action: .bootstrap)
    }

    nonisolated func reconcile(update: AppUpdate) {
        Task { @MainActor [weak self] in
            self?.apply(update: update)
        }
    }

    private func apply(update: AppUpdate) {
        switch update {
        case .fullState(let s):
            if s.rev <= lastRevApplied { return }
            lastRevApplied = s.rev
            state = s
        }
    }

    func dispatch(_ action: AppAction) {
        rust.dispatch(action: action)
    }
}

final class KeychainSecretStore: SecretStore {
    private let service = "com.rebelwallet.app"

    func getSecret(key: String) -> String? {
        var query = baseQuery(key: key)
        query[kSecReturnData as String] = true
        query[kSecMatchLimit as String] = kSecMatchLimitOne

        var result: AnyObject?
        let status = SecItemCopyMatching(query as CFDictionary, &result)
        guard status == errSecSuccess, let data = result as? Data else {
            return nil
        }
        return String(data: data, encoding: .utf8)
    }

    func setSecret(key: String, value: String) -> Bool {
        let data = Data(value.utf8)
        var query = baseQuery(key: key)
        let update: [String: Any] = [kSecValueData as String: data]

        let status = SecItemUpdate(query as CFDictionary, update as CFDictionary)
        if status == errSecSuccess {
            return true
        }

        query[kSecValueData as String] = data
        query[kSecAttrAccessible as String] = kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly
        return SecItemAdd(query as CFDictionary, nil) == errSecSuccess
    }

    func deleteSecret(key: String) -> Bool {
        SecItemDelete(baseQuery(key: key) as CFDictionary) == errSecSuccess
    }

    private func baseQuery(key: String) -> [String: Any] {
        [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: key
        ]
    }
}
