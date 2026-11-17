# Validation for 0.1.1

Windows 11 Pro (build 26300) x86-64 MSVC, Rust 1.97.0. **Windows 10 was not available and is not claimed as tested.**

Passed locally:

- 32 unit tests (`cargo test --locked`) plus the Windows running-binary replacement integration test. New in 0.1.1:
  - path import: quoted, single-quoted, spaces, Croatian and CJK folder names, a file inside the old folder, a missing `invite.json`, a missing folder, an invalid invitation;
  - programmatic UI tests (egui_kittest, AccessKit tree): typing a path and clicking **Import path** with the native picker disabled; **Use existing launcher folder** through the worker thread while the window keeps rendering and the paste field stays usable; a picker that returns immediately is reported as "could not open"; invitation and game cannot be switched while a game is starting/running, including a late picker result; ticked support reporting without an invitation explains that nothing can be sent; picker messages/errors stay inside the default 840x760 window.
- `cargo fmt --check`, `cargo clippy --locked --all-targets -- -D warnings`, `scripts/check_public.py`.
- Real release executable on Windows 11 with an isolated data folder: the native folder picker opened in front of and owned by the launcher; choosing a Unicode/space folder imported a fixture invitation; Cancel showed "Nothing was selected"; a quoted non-existent pasted path (Enter key) showed its error at the top; an unreachable fixture server showed an explained upload error without URL or credentials.

Root-cause notes for "Use existing launcher folder does nothing" (0.1.0):

- 0.1.0 drew notices and errors at the very bottom of a non-scrolling window. At the default 840x760 size this line is below the visible area, so a failed import (for example a folder without `invite.json`) looked like no reaction. Messages are now at the top and content scrolls.
- 0.1.0 ran the dialog on the render thread without an owner window, and rfd reports an open failure exactly like a cancel. 0.1.1 runs it on its own STA thread, owned by the launcher window, distinguishes an instant failure, and always offers path import.
- The exact failure on the Windows 10 machine was not reproduced; these are the confirmed defects that produce the reported symptom.

Still requires user validation: Windows 10 picker behaviour, game login/gameplay, live release-to-release update. Build 1.60.1.70959 is not declared compatible.
