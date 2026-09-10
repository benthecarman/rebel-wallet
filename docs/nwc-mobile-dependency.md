# `nwc-mobile` dependency policy

Rebel Wallet consumes `nwc-mobile` from the `ntheile/nwc-mobile` repository,
which is controlled by the same GitHub owner as this fork. Both the Rust crates
and the Swift package must use the same full commit revision. Mutable branches,
tags, and unpinned Git references are not allowed.

RustSec tooling does not provide complete advisory coverage for Git-sourced
packages. The Rebel Wallet maintainer who proposes an `nwc-mobile` revision
change therefore owns the source audit and must include the following evidence
in the dependency-update pull request:

1. Review the complete source and dependency diff between the old and proposed
   `nwc-mobile` commits. Explain security-sensitive changes to parsing,
   authorization, persistence, replay handling, payments, native bridges, build
   scripts, and procedural macros.
2. Confirm that the proposed commit is reachable from reviewed `nwc-mobile`
   history and that its required CI passed. Review that repository's
   `SUPPLY_CHAIN.md`, `deny.toml`, locked dependency graph, and compile-time
   build-unit allowlist.
3. Update all Rust crate revisions, the Swift package revision, `Cargo.lock`,
   and `Package.resolved` together. Reject any unexpected source, checksum, or
   transitive dependency change.
4. Run Rebel Wallet's locked Rust tests and checks, generated-binding contract
   checks, and iOS Debug and Release builds before merging.
5. Obtain a second maintainer review for changes that affect authorization,
   payment execution, secrets, durable storage, native packaging, or the
   dependency execution surface.

If the `ntheile/nwc-mobile` repository changes ownership, becomes unavailable,
or can no longer enforce its security controls, freeze the current reviewed
revision. Do not update the dependency until the exact reviewed source has been
moved to a repository controlled by the Rebel Wallet maintainers or vendored
with preserved license and provenance information.
