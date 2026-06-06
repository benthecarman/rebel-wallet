# RMP Architecture Divergence Report

This document compares Rebel Wallet against the RMP Architecture Bible:

https://raw.githubusercontent.com/rust-multiplatform/rmp/refs/heads/master/rmp-architecture-bible.md

The project follows several core RMP ideas already: it has a Rust core, UniFFI bindings, a SwiftUI shell, Rust-owned wallet/Nostr/persistence state, a channel-based actor, full-state updates, and native Keychain storage. The items below highlight where the current implementation diverges from the stricter RMP standard.

## 1. Testing Strategy Is Missing

The RMP standard emphasizes testing Rust core logic without platform dependencies. This repo currently has no visible Rust or Swift tests.

Evidence:

- No `#[cfg(test)]` modules were found in `rust/src`.
- No test files were found under the usual repo paths.
- The only test references found were generated UniFFI comments.

Why it matters:

The app's core includes payment parsing, payment validation, contact merging, activity derivation, persistence loading, and Nostr state mutation. These are exactly the areas RMP expects to be verified in Rust.

Suggested remediation:

- Add unit tests for `parse_send_destination`, send validation, `merge_contacts`, `activity_from_movement`, `receive_status`, and app-data load/save behavior.
- Add actor-level tests that dispatch `AppAction` values and assert resulting `AppState`.
- Add a small fake `SecretStore` for deterministic wallet/Nostr state tests.

## Suggested Priority

1. Split `rust/src/lib.rs` into RMP-style modules.
2. Move send/receive/activity display derivation and validation out of Swift and into Rust state fields.
3. Convert local receive/send workflow state into Rust-owned flow state.
4. Tighten navigation ownership with route projection and full stack reconciliation.
5. Add focused Rust tests for the moved policy and formatting logic.
6. Introduce typed capability bridges for QR scanning, photo picking, and clipboard access.
7. Add at least one second-platform target or smoke app to keep the Rust core honest.
