# Privacy and support data

Public releases are generic. They contain no personal invitation, account password, relay token, private certificate, home IP address, SSH configuration or WoW game data.

Private invitations and settings live in the current user's `WoWFriendsLauncher` application-data directory. On Windows the invitation is encrypted using user-scoped DPAPI. No GitHub access token is needed by players. The updater never attaches a private-server Authorization header to a GitHub request.

Support reporting defaults to off. An existing explicit support opt-in can be migrated from the old launcher. The window always shows whether reporting is enabled.

When enabled, the launcher sends a bounded report to the invited server approximately every 30 seconds, including before a successful game launch. It reads only:

- The selected game's version metadata.
- Bounded tails of known game graphics/general logs and its own connector log.
- The header of the latest game crash text, stopping before memory/DBG-DATA and environment sections.
- Its own support addon's saved observations, not other addons' saved data. Saved observations become available when the game saves them.
- Responsiveness and working-set memory of the application-launched game child.

Credential-looking lines, email addresses, Windows user-profile names and long hexadecimal values are filtered. Filtering is a precaution; do not put secrets in custom error messages. The server independently filters incoming reports.

The optional in-game addon sends small self-whisper messages with a diagnostic prefix: control/connection state, input/send counters and Lua error source locations. It never sends chat contents or commands. Native server logs identify the authenticated game account. Live messages and launcher uploads use the existing authenticated server connection and HTTPS relay respectively.

Unchecking support stops new launcher reports; an HTTP request already transmitted cannot be recalled. The in-game addon stops when the game exits and is moved out of AddOns on the next launch with support unchecked. No game settings, bindings, quests or character data are changed by the observer.

The intended server stores only the latest authenticated report per player and offers no public HTTP read endpoint. Server operators must separately configure their private retention and access policy; native game-log rotation is separate from uploaded-report retention.

No screenshots, desktop monitoring, arbitrary command execution, browser data, memory dumps or general remote access are implemented.
