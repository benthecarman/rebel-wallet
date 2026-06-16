# Rebel Wallet

Native iOS wallet built with [RMP](https://github.com/nickthecook/rmp) (Rust Multiplatform).
The SwiftUI shell renders native iOS screens while the Rust core owns wallet,
Nostr, persistence, and routing state.

## MVP Scope

- iOS bundle id: `com.rebelwallet.app`
- Bark wallet backend from local `../bark/bark`
- Signet Ark server and Esplora defaults
- iOS Keychain storage for wallet seed and Nostr secret key
- Local sqlite/files for non-secret wallet and app state
- Setup/restore, balance, Ark send/receive, Lightning invoice pay/receive
- Activity/history and Nostr profile, contacts, contact list, and direct messages
- NWC/NWA intentionally excluded

## Quick Start

```bash
brew install xcodegen
cargo check -p rebel-wallet-core
just ios-build
cd ios && xcodegen generate
```
