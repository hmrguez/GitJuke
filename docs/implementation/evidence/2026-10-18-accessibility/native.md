# Captured native accessibility audit

2026-10-18, macOS 27.0 (26A428), arm64. Actual unsigned Tauri/WebKit → IPC → Rust → isolated SQLite with an explicitly configured local synthetic gh executable. No live GitHub request or provider-compatibility claim. The normal app was built, not launched. Both core palette colors and settled source/storage/IPC/receipt limits are preserved.

## Setup and artifacts

- Worktree: `/private/tmp/gitjuke-accessibility`, detached from initial commit 5599c29 with the current uncommitted reader baseline copied in. Concurrent runtime/log references copied mid-edit were replaced with the original pre-logs baseline supplied by that chat; none of that isolation repair is part of the integration patch.
- App: **GitJuke Accessibility — LOCAL SYNTHETIC FIXTURES**, `dev.hmrguez.gitjuke.qualification.accessibility`.
- Data: `/Users/hmrguez/Library/Application Support/dev.hmrguez.gitjuke.qualification.accessibility/gitjuke.sqlite3`.
- Local source: `/private/tmp/gitjuke-accessibility-fixture-20261018`, empty configuration directory and fake gh, initially paused. The normal stored account and the prior qualification app's data were not reconfigured.
- The preparer creates thirteen initially unseen source records. It refuses existing data and the normal namespace before writes. `fixture-verification.json` records 14 source-contract assertions. It never links into production.
- `artifact.json` records final source and both native artifact hashes/identifiers. Final isolated and normal `.app` builds pass. No signing, installation, publication or distribution.

The copied Cargo cache was inconsistent while other builds were active; a clean isolated build resolved it. Later low free space caused the actual storage guard to stop remote reads. Task-owned disposable compiler outputs were reclaimed; the guard was preserved and repair resumed reads. These are infrastructure/storage observations, not performance evidence. CUA activation/inspector calls also timed out around bundle replacement; refreshed bindings resumed work. A late optional Paper computed-color inspection timed out; no native computed-color assertion is claimed from it.

## Window and actual text size

The nominal 960×640 outer window was genuinely too short: it supplied 608px below its 32px title bar. A bottom-edge resize to 960×672 gave the required reader space. The normal and qualification minimum height is now 672px on the tested native window configuration. `native-final-minimum-before.jpg` and `native-final-minimum-after.jpg` are both 960×672; an attempted bottom-edge reduction to 640 was rejected.

A temporary read-only Web Inspector ResizeObserver logged `CONTENT – 960 – 140` while tools were docked, `CONTENT – 960 – 640` after closure, and 140 again after reopening. `native-content-measurement-ax.txt` records that sequence. The observer was disconnected. This is an actual content measurement, excluding native chrome.

Keyboard Command = four times selects 200%. `native-font-100-ax.txt` records computed root **12px** / description **13px**, and `native-inspector-200-ax.txt` records description **26px**. This enlarges actual text rather than applying a screenshot/CSS transform or browser page zoom. Metadata, headings and diff rows follow the same root scale. The enlarged shell switches to explicit list/detail/Back. Screenshots include default-width Overview, minimum Overview, Changes, replies, Checks, Activity and Preferences at 200%; view-local vertical and code horizontal scrolling remain available.

Paper metadata was deterministically measured at 3.10:1 on its canvas before correction. The final stylesheet uses the same ink/surface colors with a 70% metadata ink mix, giving 4.88:1 on the canvas; thread metadata has no compounded opacity. The regression requires ≥4.5:1 on canvas, surface, quiet and hover in Graphite and Paper. `native-paper-thread-metadata-200.jpg` shows actual final Paper thread metadata, dates, fallback labels and source controls wrapping in the minimum reader. Line guides failed at 4.23:1 on an added row and now use the existing secondary text tone; another contract test requires ≥4.5:1 on normal/added/deleted backgrounds in both palettes. `native-paper-diff-line-guides-200.jpg` and its AX capture show the final corrected Paper patch at 200%, terminal row 403 and stable diff focus. Numerical ratios are the deterministic stylesheet/palette computation; this is not a full WCAG audit.

## Keyboard, focus and destinations

- j → Enter opened the loaded PR and focused its Overview tab. Reader End selected/focused Checks; Tab reached its named scroll panel and Page Down preserved that target.
- Exact Activity destinations focus their committed article; Tab reaches named scrollable bodies. Explicit segment replacement resets the replaced text scroll and focuses the new body, including at terminal disabled-button boundaries.
- Comments: 50 then 5 of 55. Native Tab traversal reached **First page**, Enter restored the first 50-message page and stable Conversation focus. Reviews: independently 50 then 5. Thread metadata: 20, 20, 15. Selected `file-40.txt` replies: 50 then 5. Back restored the exact `file-40.txt` control on thread page three. Enlarged replies remain readable in the minimum view.
- Preferences opened by keyboard and exposed a named modal, all four sections, and a 200% text-size control. Focus stayed within the modal, and Escape restored the Preferences opener. GitHub configuration and Clear actions were inspected without submitting them.
- k from the Activity region focused the last loaded check row; Enter opened the named check dialog. Tab focused **Activity source body** containing `External 1 · SUCCESS`; Escape restored the same check row. This was an already-seen synthetic historical record, not a new receipt case.
- Browse releases exact-source authorization. An actual compact return originally fell back to the Activity region despite the source record still existing. The shell regression reproduces this without mouse focus (`origin-before.log`) and passes after carrying explicit Activity/PR origin IDs (`origin-after.log`). The final native Browse → Back restores the exact LEFT Activity row (`native-final-browse-back-origin-ax.txt`).

## Byte boundaries and exact seen state

`initial.json` contains thirteen local source identities, all unseen. `unicode-segment-1.json`, `unicode-segment-2.json` and `unicode-repeated.json` preserve that exact map while the source metadata reports a complete **131,072-byte** body and a digest. The body is 81,920 UTF-16 units. Segment one ends at character 32,767, segment two starts with the intact emoji, and segment three ends at 81,920. The final distinct segment alone changes `local-unicode-limit` to seen. Repeated 1→2 visits do not complete coverage. Native AX records show the correct segment and body focus.

The **131,073-byte** source renders an explicit oversized placeholder and remains unseen. `left-body.json` adds only the explicitly opened complete LEFT source to seen. Wrong-commit/mixed-side/negative/outdated/wrong-file bodies are valid complete source records and correctly become seen when explicitly opened; invalid anchors do not authorize fabricated diff navigation. After these cases exactly seven local records are seen. The oversized source and five unopened original records remain unseen. Neither Mark page seen nor external GitHub navigation was used.

The same map remains after all ordinary discussion paging. All fifteen captured identities, including two already-seen synthetic check events outside the thirteen-record source oracle, survive final Quit/relaunch/revalidation. Those check entries do not qualify a reader-induced new transition. Historical baseline/recovery completion remains unqualified.

## Supplied patch and anchors

The selected complete file is **7,843 UTF-8 bytes / 403 semantic rows**, consisting of one hunk header and 201 old/new line pairs. Native pages cover 1–200, 201–400 and 401–403. Current LEFT **199–201**, commit `bbbbbbbbbb…`, path `file-0.txt`, attaches only with matching context. Keyboard Return to diff selects page three and retains the named diff focus. The range crosses a page boundary; attachment does not imply all range lines are simultaneously visible.

Continuous End reaches the terminal supplied rows. At 200% the viewport recomputes row geometry, retains focus and reaches row 403. Old/new columns and added/deleted text labels remain in the native AX table. Complete model traversal is separately locked by deterministic tests; these small native fixtures do not qualify maximum-tier DOM, layout cost or resident memory.

Against that same complete patch, native captures establish these negative labels and absence of Return to diff:

| Source | Label |
| --- | --- |
| Valid but different commit | Recorded commit does not match this patch side |
| RIGHT end / LEFT start | Ambiguous multi-side range |
| Negative supplied line | File-level or unavailable line |
| Null current position | Outdated · original bbbbbbbbbb:201 |
| Other supplied path | Different file · anchor unavailable in this patch |

The original RIGHT source was left unopened in this slice and remains unseen, helping test isolation. Its previous 202-row positive native qualification remains historical; no new 403-row RIGHT claim is made.

## Reduced motion and VoiceOver

Reduce Motion originally **off**, temporarily **on**. Native `matchMedia` was true and computed transition duration was 0s. Original off restored and recorded. VoiceOver originally **off**; its existing caption-panel preference was already on and was not changed.

Two VoiceOver enable attempts did not produce a running VoiceOver process, verifiable spoken output or cursor traversal. The final system switch reports on, while the exact VoiceOver process probe has zero matches; off was then restored. This does not qualify VoiceOver. AX inspection and keyboard tests are not substitutes. A functioning native screen-reader environment is still required across destinations, viewport/text-size combinations, dialogs, code and discussions.

The audit app, System Settings and VoiceOver Utility opened for this task were stopped after capture. No login or notification permissions were enabled.

## Evidence qualification

`verify-reader-accessibility.py` verifies the saved observation files: **63 checks**. It is a captured-evidence verifier, not an unattended native UI runner. The **71 deterministic tests** comprise 31 frontend, 35 reader integration, four Conversation core and one IPC test. Source/fixture assertion counts are separate. One opt-in live test remains ignored.

No new live provider evidence, physical M1/8-GiB/macOS-15 benchmark, native maximum DOM/process memory or timing benchmark, full advertised OS test, signed/quarantined artifact or native alert claim is made. Full native keyboard/VoiceOver matrices and merged logs/query/alerts controls remain to rerun. All 29 grouped release scenarios remain Pending.
