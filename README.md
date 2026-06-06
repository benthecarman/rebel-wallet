# Rebel Wallet

Native iOS wallet built with [RMP](https://github.com/nickthecook/rmp) (Rust Multiplatform).
The SwiftUI shell renders native iOS screens while the Rust core owns wallet,
Nostr, persistence, and routing state. A host CLI smoke target also links the
Rust core so non-iOS builds catch accidental Swift-only architecture drift.

## MVP Scope

- iOS bundle id: `com.rebelwallet.app`
- Bark wallet backend from local `../bark/bark`
- Signet Ark server and Esplora defaults
- iOS Keychain storage for wallet seed and Nostr secret key
- Local sqlite/files for non-secret wallet and app state
- Setup/restore, balance, Ark send/receive, Lightning invoice pay/receive
- Activity/history and Nostr profile, contacts, contact list, and direct messages
- NWC/NWA intentionally excluded

## Host Smoke Target

```bash
cargo run -p rebel-wallet-cli
```

The CLI is intentionally minimal: it instantiates `FfiApp` with an in-memory
secret store and prints core state. It exists to keep the Rust core buildable
outside the iOS app while Android or desktop UI targets are still pending.

## Quick Start

```bash
brew install xcodegen
cargo check -p rebel-wallet_core
just ios-build
cd ios && xcodegen generate
```
