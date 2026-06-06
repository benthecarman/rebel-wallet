# RMP Architecture Divergence Report

This document compares Rebel Wallet against the RMP Architecture Bible:

https://raw.githubusercontent.com/rust-multiplatform/rmp/refs/heads/master/rmp-architecture-bible.md

The project follows several core RMP ideas already: it has a Rust core, UniFFI bindings, a SwiftUI shell, Rust-owned wallet/Nostr/persistence state, a channel-based actor, full-state updates, and native Keychain storage. The items below highlight where the current implementation diverges from the stricter RMP standard.

## 1. Platform Coverage Is iOS-Only

RMP expects the Rust core to serve iOS, Android, and desktop targets with thin native UI layers. This repo currently declares and documents only an iOS app.

Evidence:

- `rmp.toml` only has an `[ios]` section.
- `README.md` describes the app as a native iOS wallet.
- There are no `android/`, `desktop/`, `crates/*-desktop`, or CLI app targets in the repo.

Why it matters:

The Rust core may still be portable, but missing non-iOS targets means architecture drift can hide in Swift because there is no second platform forcing shared business logic.

Suggested remediation:

- Add an Android or desktop smoke target early, even if minimal.
- Use that target to validate that payment parsing, routing, activity display, contacts, and Nostr flows are not Swift-specific.

## 2. Rust Core Is a God File

The bible recommends a top-level FFI layer plus domain-oriented modules such as `state.rs`, `actions.rs`, `updates.rs`, and `core/` submodules. Rebel Wallet currently places FFI types, actor logic, persistence, wallet operations, Nostr operations, upload logic, payment parsing, formatting, and helpers in `rust/src/lib.rs`.

Evidence:

- `rust/src/lib.rs` is 1,775 lines.
- `AppState`, `AppAction`, `AppUpdate`, `FfiApp`, `AppCore`, storage helpers, Nostr helpers, Bark wallet helpers, and display helpers all live in the same file.

Why it matters:

Large single-file cores become harder to test, harder to split by responsibility, and easier to accidentally expose internal implementation details through the FFI boundary.

Suggested remediation:

- Move FFI-visible state types to `rust/src/state.rs`.
- Move `AppAction` to `rust/src/actions.rs`.
- Move `AppUpdate`, `CoreMsg`, and `AsyncMsg` to `rust/src/updates.rs`.
- Move actor implementation into `rust/src/core/mod.rs`.
- Split wallet, Nostr, contacts, activity, receive, send, and persistence helpers into focused `core/` modules.

## 3. Swift Duplicates Business and Display Logic

RMP's golden rule is that native UI renders data and dispatches intents; Rust owns business logic, formatting, validation, and derived display fields. The SwiftUI layer currently derives several values that should be Rust-owned.

Evidence:

- `ContentView.swift` formats sat amounts with local `formatSats` helpers.
- `SendView` locally determines whether a destination is Lightning by checking string prefixes.
- `SendView` locally computes `hasInsufficientBalance`, `canSend`, and send error text.
- `ActivityRow` derives counterparty labels, method icons, message text, and amount text from raw `ActivityItem` fields.

Why it matters:

These decisions will need to be reimplemented on Android and desktop. They can also diverge from Rust's payment policy; Rust already performs send validation in `pay_destination`, `pay_lightning_invoice`, and `pay_ark_address`.

Suggested remediation:

- Add Rust-computed display fields such as `balance_display`, `amount_display`, `signed_amount_display`, `send_can_submit`, `send_error_text`, `send_destination_kind`, `activity_icon_kind`, and `activity_display_title`.
- Keep raw values available for copy/share/debug flows, but render Rust-computed display fields in Swift.
- Add Rust tests for send validation and activity display derivation.

## 4. Native Owns Too Much View-State Derivation

RMP allows native UI state for purely visual concerns, but flow state and user-visible outcomes should generally be represented by Rust state and actions. Some current receive/send flow state lives entirely in Swift.

Evidence:

- `ReceiveView` owns `method`, `showingResult`, `showingSuccess`, `shownSuccessId`, and amount initialization behavior.
- `ReceiveView` decides when to show the success screen by observing `lightningPaid`.
- `SendView` owns `showingSuccess`, `successResult`, `successAmountSat`, and draft destination behavior.

Why it matters:

These local decisions are part of the user-visible workflow, not just rendering. Another platform would need to recreate them, and edge cases can diverge across platforms.

Suggested remediation:

- Model receive method and receive workflow phase in Rust, for example `ReceiveState.method` and `ReceiveState.phase`.
- Model payment success as Rust state or a dedicated side-effect update with a revision.
- Dispatch explicit actions such as `SelectReceiveMethod`, `BeginReceiveRequest`, `DismissPaymentSuccess`, and `ResetSendDraft`.

## 5. Navigation Has a Swift Shadow

RMP says Rust should own navigation state through a router, with native navigation reacting to that state. Rebel Wallet has a Rust `Router`, but Swift keeps a local `navPath` and only reports pop events back to Rust.

Evidence:

- `ContentView` declares `@State private var navPath: [Screen] = []`.
- Swift mirrors `manager.state.router.screenStack` into `navPath`.
- Swift only dispatches `UpdateScreenStack` when the new path is shorter than the old path.

Why it matters:

Navigation can become split-brained: Rust owns the canonical router, but Swift can temporarily hold a divergent stack or make decisions Rust never sees.

Suggested remediation:

- Add route projection helpers in Rust for mobile navigation.
- Treat Swift's `NavigationStack` path as a projection of Rust state.
- Report all platform-initiated navigation changes back to Rust, not only pops.
- Consider replacing ad hoc screen-stack mutations with domain-specific navigation actions.

## 6. Capability Bridges Are Ad Hoc

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

## 7. Testing Strategy Is Missing

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

## 8. Busy State Is Too Coarse

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
