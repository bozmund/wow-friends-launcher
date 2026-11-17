# Validation for 0.1.0

Windows x86-64 MSVC, Rust 1.97.0.

Passed locally:

- 21 unit tests: active-region deduplication; real executable/version metadata selection; refusal of inconsistent or unsupported builds; accurate installed/accepted messages; DPAPI invitation roundtrip; private settings replacement; URL/header/config injection rejection; diagnostic opt-in defaults; Unicode byte limits and credential/path filtering; diagnostic collection before a blocked launch; fragmented binary relay and cancellation; updater origin, version and checksum rejection.
- A separate Windows integration test replaces a disposable running executable with a valid executable copy. Existing projects and game installations are not test fixtures.
- `cargo fmt --check` and `cargo clippy --locked -- -D warnings`.
- Native Rust HTTPS status request to an authenticated server.
- Native Rust WebSocket relay receiving the actual native world-server handshake.
- A Rust launcher support report was received privately by the server before any game launch.
- The desktop window opened and remained responsive.
- Source and distributable boundary checks: no invitations, passwords, personal machine paths, private server domains/IPs or certificates are packaged in the public application/source.

Still requires user gameplay validation:

- One-time Windows certificate/localhost setup approval on a fresh Windows installation.
- Game login with the player's existing password and actual in-game controls/tutorial reproduction.
- Real release-to-release automatic update after the first release is published. Checksum rejection and Windows running-file replacement are independently tested, but there was no older published Rust release available for a live upgrade test.

The version-detection fix does not declare a newer game protocol compatible. The server's verified accepted-build list remains authoritative. Neither Linux nor macOS gameplay is claimed as tested.
