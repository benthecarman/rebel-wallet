# Rebel Wallet

Native iOS wallet built with [RMP](https://github.com/nickthecook/rmp) (Rust Multiplatform).
The SwiftUI shell renders native iOS screens while the Rust core owns wallet,
Nostr, persistence, and routing state.

## MVP Scope

- iOS bundle id: `com.rebelwallet.app`
- Bark wallet backend from local `../bark/bark`
- Mainnet and Signet defaults plus configurable Regtest ASP and Esplora servers
- iOS Keychain storage for wallet seed and Nostr secret key
- Local sqlite/files for non-secret wallet and app state
- Setup/restore, balance, Ark send/receive, Lightning invoice pay/receive
- Activity/history and Nostr profile, contacts, contact list, and direct messages
- Nostr Wallet Connect strings with optional NWC Wake push handling

## Quick Start

```bash
brew install xcodegen
cargo check -p rebel-wallet-core
just ios-full
```

Shared iOS signing defaults are documented in `.env.sample`. Copy it to the ignored `.env` file and set your machine-specific values:

```bash
cp .env.sample .env
```

```env
IOS_BUNDLE_ID=com.YOURORG.rebelwallet
IOS_DEVELOPMENT_TEAM=YOURTEAMID
IOS_APP_GROUP_ID=group.com.YOURORG.rebelwallet
NWC_WAKE_SERVER_URL=https://YOUR_NOTIFICATION_SERVER
```

Use `just ios-xcodeproj` instead of running `xcodegen generate` directly so `.env.sample`, `.env`, and the optional ignored `.env.local` override are loaded before the Xcode project is regenerated.

Rust-generated Swift bindings and XCFramework metadata are committed so a
checkout can be opened in Xcode immediately. After changing the Rust FFI
surface, run `just ios-full` and commit the regenerated artifacts.

To build, install, and launch on a connected iPhone, use:

```bash
just run-ios-phone
```

To connect to Regtest, open **Settings → Network → Regtest** and enter the ASP and Esplora URLs. The defaults use
`http://127.0.0.1:3535` and `http://127.0.0.1:3000`, which reach services on the Mac from the iOS Simulator. On a
physical iPhone, replace `127.0.0.1` with the server computer's LAN address and allow local-network access when iOS
prompts.

You can optionally pass a bundle id when launching a local fork:

```bash
just run-ios-phone com.YOURORG.rebelwallet
```

That argument only controls the bundle id used for `devicectl` launch. The Xcode project is generated from `.env.sample` plus local `.env` and `.env.local` overrides, so keep `IOS_BUNDLE_ID`, `IOS_APP_GROUP_ID`, and signing values aligned with the app id and entitlements in your Apple developer account.

## Apple Developer Setup

For a local fork, create or update these identifiers in your Apple Developer account:

- App ID: `com.YOURORG.rebelwallet`
- Notification Service Extension App ID: `com.YOURORG.rebelwallet.NotificationService`
- App Group: `group.com.YOURORG.rebelwallet`
- Keychain access group: same team prefix + `com.YOURORG.rebelwallet`

Enable these capabilities on both the main app and the notification service extension:

- App Groups, with `group.com.YOURORG.rebelwallet`
- Keychain Sharing, with the shared keychain group

Enable Push Notifications on the main app id. The notification service extension does not need the Push Notifications capability, but it does need the same App Group and Keychain Sharing capability so it can read the NWC wake snapshot and queued wake data.

For APNs, create an Apple Push Notification authentication key in the developer portal and keep the `.p8` file, Key ID, and Team ID for the wake/notification server. The app itself only needs the signing team, app id, app group, and push capability; the server uses the APNs key to send wake pushes.

Your ignored `.env` (or optional `.env.local` override) should line up with those identifiers:

```env
IOS_BUNDLE_ID=com.YOURORG.rebelwallet
IOS_DEVELOPMENT_TEAM=YOURTEAMID
IOS_APP_GROUP_ID=group.com.YOURORG.rebelwallet
IOS_APS_ENVIRONMENT=development
NWC_WAKE_SERVER_URL=https://YOUR_NOTIFICATION_SERVER
```

After changing any of these values, regenerate the Xcode project:

```bash
just ios-xcodeproj
```
