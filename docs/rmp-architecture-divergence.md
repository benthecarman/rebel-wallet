# RMP Architecture Divergence Report

This document compares Rebel Wallet against the RMP Architecture Bible:

https://raw.githubusercontent.com/rust-multiplatform/rmp/refs/heads/master/rmp-architecture-bible.md

The project follows several core RMP ideas already: it has a Rust core, UniFFI bindings, a SwiftUI shell, Rust-owned wallet/Nostr/persistence state, a channel-based actor, full-state updates, and native Keychain storage. The items below highlight where the current implementation diverges from the stricter RMP standard.

## 1. Capability Bridges Are Ad Hoc

The bible describes capability bridges as typed, bounded lifecycles: Rust decides when a native capability is needed, native executes the OS API, native reports raw data, and Rust decides the outcome. Rebel Wallet currently implements several native capabilities directly in Swift views/controllers.

Evidence:

- QR scanning is implemented in `QRScannerViewController` using `AVCaptureSession`.
- QR scan results are handled by Swift closures that dispatch follow-up Rust actions.
- Photo selection and image loading are Swift-local before dispatching base64 image data to Rust.
- Clipboard paste reads `UIPasteboard.general.string` directly from Swift.

Why it matters:

QR scanning, photo selection, and clipboard access are legitimate native capabilities, but without typed lifecycle contracts Rust does not fully own when they open, close, retry, or report errors.

Suggested remediation:

- Define callback interfaces or explicit capability request state for QR scanning, photo picking, and clipboard reads.
- Have Rust request the capability and accept raw results or errors.
- Keep native code limited to OS handles, lifecycle, and raw data delivery.

## 2. Testing Strategy Is Missing

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

## 3. Busy State Is Too Coarse

The bible recommends domain-specific busy flags so each UI surface can render loading state accurately. Rebel Wallet uses a single `busy: bool`.

Evidence:

- `AppState` has `pub busy: bool`.
- The same flag is used across setup, sync, send, receive, Nostr upload, profile operations, and contact operations.

Why it matters:

A single busy flag can make unrelated screens appear blocked, can show incorrect spinners, and makes it harder for native UI to determine which controls should be disabled.

Suggested remediation:

- Replace `busy: bool` with a `BusyState` record.
- Start with fields such as `bootstrapping`, `opening_wallet`, `syncing_wallet`, `creating_invoice`, `sending_payment`, `uploading_profile_picture`, `publishing_nostr`, and `refreshing_contacts`.
- Update Swift to observe the specific flag relevant to each screen.

## Suggested Priority

1. Split `rust/src/lib.rs` into RMP-style modules.
2. Move send/receive/activity display derivation and validation out of Swift and into Rust state fields.
3. Convert local receive/send workflow state into Rust-owned flow state.
4. Tighten navigation ownership with route projection and full stack reconciliation.
5. Add focused Rust tests for the moved policy and formatting logic.
6. Introduce typed capability bridges for QR scanning, photo picking, and clipboard access.
7. Add at least one second-platform target or smoke app to keep the Rust core honest.
