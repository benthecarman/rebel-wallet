import Foundation
import NwcMobileApple

final class KeychainSecretStore: SecretStore {
    private let vault = NwcKeychainVault(
        service: "com.rebelwallet.app",
        accessGroup: KeychainSecretStore.keychainAccessGroup
    )

    func getSecret(key: String) -> String? {
        vault.string(forKey: key)
    }

    func setSecret(key: String, value: String) -> Bool {
        vault.setString(value, forKey: key)
    }

    func deleteSecret(key: String) -> Bool {
        vault.deleteValue(forKey: key)
    }

    private static var keychainAccessGroup: String? {
        guard let value = Bundle.main.object(
            forInfoDictionaryKey: "RebelWalletKeychainAccessGroup"
        ) as? String else {
            return nil
        }
        let trimmed = value.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty, !trimmed.hasPrefix("$(") else {
            return nil
        }
        return trimmed
    }
}
