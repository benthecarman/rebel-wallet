# RMP Architecture Divergence Report

Compared against the RMP Architecture Bible:

https://raw.githubusercontent.com/rust-multiplatform/rmp/refs/heads/master/rmp-architecture-bible.md

## Current Shape

Rebel Wallet follows the broad RMP direction:

- Rust core crate with `cdylib`, `staticlib`, and `rlib` outputs.
- UniFFI proc-macro bindings checked in for iOS.
- SwiftUI shell with a single `AppManager`, `@Observable` state, rev-guarded reconciliation, and `NavigationStack` driven by Rust `Router.screen_stack`.
- Rust-owned wallet, Nostr, persistence, navigation, busy flags, display strings, send validation, and capability request state.
- Bounded iOS capability execution for QR scan, clipboard read, photo pick, and Keychain-backed secret storage.
- Justfile recipes for host build, Swift binding generation, iOS cross-compilation, xcframework packaging, XcodeGen, and simulator build.

The project is currently an iOS-first RMP app, not a full iOS/Android/Desktop implementation.

## Divergences

### 1. No Android Platform Layer

The bible describes Android as a first-class RMP target with:

- `android/` Gradle project.
- Jetpack Compose UI.
- generated Kotlin UniFFI bindings checked in.
- `jniLibs/` Rust shared libraries.
- `AppManager` implementing `AppReconciler`.
- Android secure storage and capability shims.

This repo has no `android/` tree and no Android build recipes. This is a scope gap rather than architectural drift inside existing code.

### 2. No Desktop iced Platform Layer

The bible describes desktop as a direct Rust consumer using iced, with no UniFFI serialization overhead. This repo has no `desktop/` or `crates/*-desktop` app. The only direct Rust consumer is `crates/rebel-wallet-cli`.

This is also a scope gap, but it means the current workspace does not satisfy the bible's multiplatform target set.

### 3. Native View Derivation Contains Some Cross-Platform Policy

The bible allows thin native `ViewState` derivation only when it is mechanical. It says filtering, sorting, validation, formatting, and business rules should live in Rust and be exposed as state fields.

Examples in Swift:

- Contact search filtering is done in `ContactsView.contacts`.
- Contact lookup and message filtering for the detail screen are done in `ContactDetailView.contact` and `ContactDetailView.messages`.
- Restore phrase normalization, word counting, and `canRestore` are computed in `RestoreWalletView`.
- Server form `canSave` / `hasChanges` is computed in `ServersView`.
- Receive flow `showingResult`, `receiveText`, and the Lightning amount gate are computed in `ReceiveView`.

Some of this is harmless local UI state, but the more reusable policy should move into Rust fields or Rust actions:

- `filtered_contacts` / search action or query-backed contact projection.
- `current_contact` and `current_contact_messages` derived from router state.
- `restore_word_count`, `can_restore_wallet`, and normalized phrase handling.
- `receive.can_continue` and `receive.request_text`.
- `servers.can_save` / validation display state.

The code is already doing this well in several places: balances, fiat displays, send destination kind, send error text, send `can_submit`, receive status display, and activity display fields are Rust-derived.

### 4. Image Networking and Cache Are Native-Owned

`ProfileImageLoader` in `ios/Sources/Services/ProfileImageLoader.swift` owns URL fetching, HTTP status handling, in-flight request coalescing, and `NSCache` storage for profile images.

The bible generally assigns networking, caching, and persistence to Rust unless native execution is required for UX quality. Native image rendering is appropriate, but image fetch/cache policy is currently iOS-only.

Suggested direction:

- If profile image behavior must match across platforms, move fetch/cache policy to Rust and expose either cached file paths/data or a capability/request contract.
- If this remains iOS-only for pragmatic reasons, document it as an intentional native cache exception.

### 5. Secrets Are Persisted Through a Native Callback, Not Side-Effect Updates

The bible's example pattern uses dedicated secret-bearing `AppUpdate` variants for values that native must persist, and the native side applies those side effects before stale-rev guards.

Current state:

- `AppUpdate` has only `FullState(AppState)`.
- Rust receives a native `SecretStore` callback interface and directly calls `set_secret` / `get_secret`.

This is a reasonable capability-bridge design for Keychain-backed storage, but it diverges from the bible's side-effect-update pattern. The upside is that Rust owns credential timing and restore policy. The tradeoff is that secret persistence failures are collapsed to `bool` return values and are not represented in the update stream.

Suggested direction:

- Either document `SecretStore` as the project's chosen secure-storage capability bridge, or migrate wallet/Nostr secret creation/export flows to explicit `AppUpdate` side-effect variants with rev fields.
- If keeping `SecretStore`, make persistence failures visible in Rust state/toasts where user recovery is possible.

### 6. iOS AppManager Has No Testable Core Protocol

The bible recommends an `AppCore` Swift protocol that `FfiApp` conforms to, so previews and tests can run without a live Rust core.

Current state:

- `AppManager` stores `let rust: FfiApp` directly.
- No Swift protocol abstraction or test double exists.
- `ios/project.yml` has no test target.

Suggested direction:

- Add a small `AppCore` protocol with `dispatch`, `listenForUpdates`, and `state`.
- Store `let rust: AppCore`.
- Add Swift test or preview fakes once views are split out.

### 7. Release Profile Hardening Is Missing

The bible recommends release profile settings for mobile binary size:

- `lto = true`
- `codegen-units = 1`
- `strip = true`
- `panic = "abort"`

Current root `Cargo.toml` has only the workspace declaration and no `[profile.release]`.

Suggested direction:

- Add a workspace-level `[profile.release]` once release build/debugging tradeoffs are acceptable.

### 8. Testing Is Rust-Only and Narrow

The repo has focused Rust unit tests in `state.rs`, `activity.rs`, and `nostr_support.rs`. There are no checked-in iOS tests, Android tests, desktop tests, or integration tests that exercise the actor/update loop.

Suggested direction:

- Add actor-level Rust tests around `AppAction -> AppState`.
- Add persistence/restore tests for wallet/Nostr app data where dependencies allow.
- Add iOS tests after introducing `AppCore` test doubles.

## Non-Divergences Worth Preserving

- `FfiApp::dispatch()` is fire-and-forget.
- `rust/src/lib.rs` is a thin UniFFI boundary, with the actor implementation moved under `rust/src/core/`.
- Updates are full-state snapshots.
- `AppState.rev` is monotonic and the iOS bridge uses a stale-rev guard.
- Rust owns navigation state through `Router`.
- iOS reports platform pops back to Rust via `UpdateScreenStack`.
- iOS Keychain is kept native, which is consistent with the bible's secure credential storage guidance.
- QR scan, clipboard read, and photo pick are bounded capability requests where Rust opens/closes the request and native reports raw data back.
- iOS SwiftUI source is split into root, screen, component, service, capability, and theme files rather than one large `ContentView.swift`.
- Generated Swift UniFFI bindings and the xcframework are checked in.
- The iOS build recipes include the Xcode/Nix environment isolation pattern described by the bible.
