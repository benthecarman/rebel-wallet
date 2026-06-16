Once you're done with a change, build it on to my phone with `just run-ios-phone` if my iPhone is visible to
`xcrun devicectl list devices`. A Wi-Fi paired phone may show as `available (paired)` instead of `connected`; that
still counts. We don't need to build the app for the simulator.

Nostr profile metadata and profile pictures are cache-managed by the Rust core. When adding or changing a profile
fetch path, route kind-0 metadata through `profile_contact_from_metadata_json` / `FetchedProfileContact`, then through
the actor's fetched-profile cache helpers before updating UI state. Swift views should render Rust-provided cached
profile image URLs and should not fetch remote pfps directly except for explicit edit-preview UI.
