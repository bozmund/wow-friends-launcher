# WoW Friends Launcher

A native Rust desktop launcher for an invitation-only WoW Forever server. The public application contains no server invitations, passwords, private certificates, game files or personal machine configuration.

## Windows users

1. Download `wow-friends-launcher-windows-x86_64.exe` from this repository's Releases.
2. Put it beside your existing private `invite.json` and `ca.crt.pem`, then open it. Existing launcher settings are imported automatically. Alternatively, choose **Use existing launcher folder** in the window, or paste that folder (or the `invite.json` path) into the text field and click **Import path**; this works even if the Windows file picker does not open.
3. Select the **game's** `_forever_/Wow.exe` if it was not discovered. You no longer need an Install Update CMD or to select Start WoW.exe.
4. Complete **Windows setup** once. This requests administrator approval for the private game's public certificate and the localhost game hostname. The game itself runs without administrator privileges.
5. Click **Play**, using your existing private-server account. Minimize the launcher while playing; it carries the game connection.

You still need a legitimately installed game. Game files are not included here. Existing installations and settings are preserved.

## Versions and updates

- The window displays the actual game executable version and the server's accepted builds.
- Duplicate Battle.net region rows are deduplicated. A second active region is not itself a version error.
- A genuinely newer or inconsistent game installation is explained clearly; no silent game update, downgrade or broad version-check bypass occurs.
- The server can publish `acceptedBuilds` in its authenticated `/status` response. Legacy private invitations fall back to the previously verified client `1.60.1.70938`.
- Launcher updates are checked at startup and every six hours. They download only from this repository's GitHub releases and must pass SHA256SUMS verification and the GitHub asset digest when present.
- Automatic launcher installation waits until no game is running/starting. Accounts, invitations, saved games and settings are not replaced. Failed downloads/checksums preserve the installed application.
- Launcher updates and Blizzard game updates are separate. Supporting a new game build requires server/connector compatibility verification.

## Private support

Optional support reports operate even when a version check prevents Play. They include bounded game error logs, detected build, control state, responsiveness and memory of the game launched by this application. They do not include screenshots, memory dumps, passwords or other addons' saved data. See [PRIVACY.md](PRIVACY.md).

Windows invitations are stored outside the application using user-scoped DPAPI. The old private invitation file is not deleted. Never upload an invitation or personalized distribution archive to this repository.

## Build and checks

Windows x86-64 MSVC is the tested release target. Install Rust, Visual Studio C++ Build Tools, then:

```text
cargo test --locked
cargo clippy --locked -- -D warnings
cargo build --locked --release
```

The build compiles the bundled connector from its included source. No private server files or credentials are needed to build. Linux/macOS game launching is not claimed as tested support.

Set `WOW_FRIENDS_LAUNCHER_DATA` to a separate empty folder to test a build beside an existing installation without touching its settings or invitation.

Useful read-only commands:

```text
wow-friends-launcher.exe --version
wow-friends-launcher.exe --doctor --output report.json
wow-friends-launcher.exe --transport-test --output transport.txt
```

`--doctor` reports detected installation, server readiness, compatibility and setup state without printing invitation credentials. `--transport-test` opens the authenticated relay and checks the native world handshake; it requires free local game ports and does not log in or alter a character. Gameplay still needs separate testing.

## Operator invitation format

Supply this **privately**, never as a release asset:

```json
{
  "name": "Player",
  "account": "player@local.invalid",
  "token": "PRIVATE_PER_PLAYER_RELAY_INVITATION",
  "relay": "https://your-server.example",
  "allowedBuilds": ["1.60.1.70938"]
}
```

Include only the public game CA as `ca.crt.pem` beside the invitation, or use its `caPem` field. A server password is optional; users may enter their existing password directly in the game. Each invitation is separate and revocable. There is no public account-registration endpoint.

## Connector and license

The desktop UI, relay, version detection, settings, diagnostics and updater run in Rust. A small bundled GPL connector remains C++, compiled from `vendor/connector.cpp`. It preserves the previously tested private working-key behavior and starts only the explicitly selected game child. It does not rewrite game executables or mapped executable regions during normal launch. Its upstream provenance is in the source header.

Source is provided under GPL-2.0-only; see [LICENSE](LICENSE). WoW and Blizzard trademarks and game content are not part of this project.
