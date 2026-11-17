# Reversible controls isolation test (0.1.2)

For unsolicited movement, an absent Escape menu or broken input, close WoW,
open **Controls troubleshooting**, then choose **Create clean controls profile
(keep backup)**. Press Play and test before adding any other addons.

The launcher preserves the complete old WTF folder (including settings,
bindings, macros and local addon data) and third-party Interface/AddOns in a
dated WoWFriendsRecovery folder inside the selected `_forever_` installation.
Nothing is deleted. Game assets, characters, quests and server accounts are
not changed. Recovery refuses to run while any Wow.exe process is present.

The new local profile disables gamepad input, click-to-move and tutorial hints,
and disables automatic synchronization of the old settings and bindings for
this test. The game recreates its normal default controls. With support sharing
enabled, only our observer is reinstalled and its control-state panel is visible.
Reports include a bounded allowlist of control settings and whether our observer
is installed or explicitly disabled; other addon names and account-folder names
are not reported. No passwords or chat contents are collected.

To undo, close WoW and choose **Restore previous controls and addons**. The test
profile is also preserved rather than deleted. Repeated prepare clicks do not
create additional resets. Recovery rejects linked/reparse-point target folders.

This is a diagnostic isolation test, not proof of the underlying cause and not
a guarantee that the original gameplay problem is fixed. It can distinguish an
old profile/addon/input configuration from a client or server fault. Windows 10
in-game verification on the affected computer is still required.
