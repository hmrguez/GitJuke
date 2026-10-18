# Native observations — 2026-10-18, Europe/Madrid

Method: native macOS CUA accessibility tree, screenshots and keyboard input against the built `GitJuke.app`, not browser fixtures or injected IPC. Host macOS 27.0 (26A428), arm64 debug development artifact; see artifact.json for exact final binary hash. Default configured window 1280×800 logical points; no M1/macOS-15 release-performance claim.

- Launched app using stored gh identity; no login, scope, credential or account changes.
- Opened existing public fixture cli/cli#14719. Changes fetched three file metadata records; head f3e2147f24… / base b758ce3564….
- Selected pkg/cmd/ruleset/view/view.go. Complete supplied patch: 403 UTF-8 bytes, nine rows. Native table exposed Old line / New line / Change and code. Deletion and addition each exposed line 62 on the correct side, with literal tabbed source, blank lines and change markers. Code scrolled horizontally.
- Selected Paged · semantic. Native UI exposed Page 1 of 1 and disabled Previous rows / Next rows. This small source does not verify multi-page native patch traversal.
- Focused Jump to hunk number, pressed Return then Right. CUA reported focus on the stable named unified-diff region, rather than document body. Native tree contained table/row/cell/change labels. This is not a VoiceOver review.
- Quit through the native GitJuke menu, rebuilt and launched the final artifact. Reading mode still read Paged · semantic, demonstrating persistence across the actual app restart.
- Final artifact displayed the cached 403-byte patch. Selecting pkg/cmd/search/issues/issues.go immediately removed the prior table and showed queued state under the new path. The new live patch subsequently rendered: 708 UTF-8 bytes, nine rows, correct old/new line 42.
- Restored the original Continuous preference with native menu/keyboard selection. Verified Continuous and the second patch's native semantic table. Left the current development app open for review; theme, identity, notification and monitoring preferences were not changed.
- Strict exact-file destination validation fell back to the visibly labeled Open PR Changes in GitHub. External browser navigation itself was not exercised.
- Background generic gh-read failures/native-stack unknown states occasionally appeared independently of successful Changes reads. No full background/provider qualification is claimed.

Pending/unavailable: native maximum/large-tier source and many-file fixtures; binary/generated/missing/incomplete/denied public examples; stale-head race under live native reads; compact 960×640 content, 200% text zoom, reduced motion, full keyboard/VoiceOver matrix; instrumented maximum DOM/source/process-memory bounds, release latency/frame-time and repeated 100-cycle stress. Synthetic tests do not replace these checks. No release scenario is promoted.
