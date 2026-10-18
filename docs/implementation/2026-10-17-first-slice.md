# Production implementation checkpoint — 2026-10-17

Status: first vertical slice implemented and exercised; **not release ready**. The user's production authorization supersedes the historical planning-only statements in the wayfinder assets. No settled requirement was changed or deferred out of v1.

## Implementation sequence

1. Validate installed gh/stored identity/API/native-stack/process assumptions and connect a real Tauri → Rust → gh → SQLite → React read path. This checkpoint covers that first slice.
2. Extend the same runtime with complete authored/direct/team/selected watch reconciliation, query persistence/search, compact head/check snapshots, independent notification deadlines and measured quota fairness.
3. Implement source-complete individual activity hydration/classification, durable seen/deduplication/progress/alert intents, baseline/recovery boundaries, native permission/onboarding and delivery revalidation (including the viewed PR).
4. Add the bounded one-file diff reader and independently paginated 50-message discussions, checks/logs and source-specific unavailable states, preserving all accepted byte/row/DOM/revision bounds.
5. Complete retention/storage/migration fault handling, installed-path LaunchAgent and lifecycle integration, then accessibility/performance and separately authorized signed-artifact qualification. Signing/provisioning/publication remains unauthorized.

These are engineering increments under the existing contract, not new product decisions. The full acceptance matrix remains the release authority.

## Implemented path

- `src-tauri` is a real Tauri 2 app, with `dev.hmrguez.gitjuke`, 960×640 minimum content dimensions, macOS 15 bundle minimum, local React assets and a narrow event capability. No frontend HTTP/shell/filesystem plugin is installed. A single-instance plugin, Rust-owned tray and Cmd-Q cancellation path share one core runtime. The build is Apple Silicon development only.
- `crates/gitjuke-core` owns scheduling, SQLite and gh. The serial worker reads authored PR pages every 60 seconds when eligible and selected overview/native-stack resources on demand/ten-second eligibility while the reader is open. These are the **implemented resources only**, not evidence of the complete watch/check/activity scheduler. No alert dispatch exists yet.
- gh discovery retains a canonical executable path from an explicit saved absolute path or known installation location. The child environment is cleared and rebuilt from OS-home, locale/keychain and a narrow proxy/CA allowlist, explicitly setting GH_CONFIG_DIR and noninteractive settings. Token/host/repo/config overrides, arbitrary developer injection, shell startup files and current-directory resolution are excluded. Only fixed read operations are exposed in Rust; no argument-vector command exists in IPC.
- One child at a time, including probes. Process groups receive TERM then KILL/reap on timeout/overflow/cancellation, with production constants of 30 seconds, 16 MiB stdout, 64 KiB stderr and two seconds grace. Failed/truncated bytes are not parsed as successful data. Raw stderr/private responses are not forwarded to UI diagnostics.
- `gh auth status --active --hostname github.com --json hosts` must report a successful active stored account; a bounded `GET /user` verifies the stable ID. Content reads have pre/post stable-ID checks. Missing credentials and mismatch lock cached GitHub content, invalidate generations and preserve the original binding. No race-free external-account-change guarantee is claimed.
- Runnable descriptors coalesce and cap at 128. Foreground service yields to the oldest queued background item after three foreground jobs. Due deadlines persist independently of descriptors. Membership stages one Search page at a time and commits only after successful traversal. Source-incomplete or >1,000-result Search output currently reports an explicit incomplete state and preserves previous membership; **partitioned recovery beyond Search's ceiling is still pending**, not an accepted watch cap.
- SQLite schema 1 separates disposable `content`/`stage` from settings, original identity, watch reasons, resource metadata, due deadlines, and reserved durable seen/deduplication/progress/attempt tables. Only the read/cache/settings paths use these ledgers so far; table existence does not constitute implemented Activity/delivery semantics. Clear invalidates pending generations and preserves durable tables. Future/corrupt databases are not silently replaced.
- List IPC pages stop at 100 records with a conservative byte budget below 1 MiB. Oversized overview bodies become explicit >128 KiB placeholders. Overview is escaped plain text; remote resources/HTML do not execute. Native stack membership comes only from the pinned REST API (`2026-03-10`): list by PR, then detailed stack for authoritative member order/trunk. Errors remain unknown, and successful empty lists are distinct nonmembership.
- The production UI uses the approved Stacks structure, locally bundled JetBrains Mono and Carbon/Parchment presets plus four persisted custom colors. It labels the implemented resource scope. No production imports reference prototype fixture data. The prototype remains separate.

## Recorded checks

Environment: Apple Silicon development host, macOS **27.0 / 26A428**, Xcode **27.0 / 27A5228h**, Rust **1.95.0**, gh **2.92.0 (2026-04-28)**, Tauri Rust **2.12.4**, API **2026-03-10**, schema **1**, app development version **0.1.0**. Mach-O inspection confirms **arm64** and `LC_BUILD_VERSION minos 15.0` (SDK 27.0). This host does **not** satisfy the physical M1/8-GiB/macOS-15 benchmark. No supported release OS matrix has been qualified.

| Subcase | State | Evidence / procedure |
| --- | --- | --- |
| Frontend TypeScript and production assets | Pass | `npm run build`; bundled local font files; no fixture fallback. |
| Native arm64 development executable/app | Pass | `npm run tauri build -- --debug --bundles app --target aarch64-apple-darwin`; local 29-MiB app opened through macOS. No release signature/notarization claim. |
| Rust workspace static checks | Pass | `cargo clippy --workspace --all-targets --offline -- -D warnings`. |
| Core deterministic suite | Pass | `cargo test -p gitjuke-core`: **19 tests pass**; tests in `tests/{github,runtime,store,transport}.rs` and queue unit test. See limitations below. |
| Stored gh context / stable identity | Pass (development context) | Sanitized actual gh reads resolved `github.com / hmrguez / 104702477`; token source was Keychain. Stable ID checked again after reads. No tokens exported, no credentials/scopes/accounts changed. |
| Live authored discovery, empty path | Pass | HTTP 200, `total_count=0`, `incomplete_results=false`. A complete empty search is distinct from a failed read. |
| Positive authored/private/request/team/mention fixtures | Blocked | This run has no positive authored fixture; no designated private/team/mention fixture was exercised. No content was created to fill coverage. |
| Live PR overview | Pass (public fixture) | Existing `cli/cli#14719`; current head and nullable/body schema parsed by `cargo run -p gitjuke-core --example diagnose -- cli/cli 14719`, then loaded through the native UI's known-PR form. Evidence output prints counts/schema booleans rather than PR bodies. |
| Native-stack negative path | Pass (public fixture) | Same PR, stack-filter endpoint HTTP 200 with an empty list, rendered as confirmed no published stack. No extension installed/required. |
| Positive native published stack | Blocked | No authorized positive fixture identified/exercised. Synthetic authoritative order/trunk checks pass but do not establish live provider compatibility. |
| Identity failure / stale commits | Pass (tested subcases) | Deterministic worker test changes fake-gh stable ID; both list/detail IPC lock while original identity persists. Store tests reject late content after clear and denied-resource purge. Headerless gh auth-loss diagnostic is classified without exposing diagnostic bytes. |
| Partial traversal / restart / IPC page count | Pass (tested subcases) | 150-member staged traversal, incomplete replacement/restart, 100/50-record pages and retained previous watch membership. Future/corrupt schema byte preservation; Pause/settings/identity restart; durable ledger rows survive clear. |
| Queue/process bounds | Pass (tested subcases) | 128/coalescing/129th deferred; three foreground then background; inclusive and one-over parameterized stdout/stderr limits; stdin EOF; timeout/cancel/helper removal and following read; sanitized environment. Tests use shorter timeouts/smaller byte ceilings through the same implementation. Full production-size/quota/50-watch acceptance remains Pending. |
| Worker end-to-end / Pause | Pass (deterministic) | Fake executable → actual process runner → worker → SQLite → typed list/detail snapshots. Pause prevents automatic reads and survives restart; explicit manual sync preserves Pause. |
| Native appearance / persistence / Quit | Pass (development subcases) | CUA inspected the real `tauri://localhost` app: public overview/head/nonmember state; four controls; Carbon→Parchment; persisted light/Pause on full restart. Restored Carbon and Resume after testing. Cmd-Q exited the development process. |
| Development duplicate launch | Pass (subcase) | `open -n` briefly creates a second process, which hands off/exits; subsequent process inspection finds one app process and existing reader state. This is not simultaneous login/notification or signed-installed-app evidence. |
| Window close / background monitoring | Pending full evidence | Close-to-hide handler and reader-only lifetime exist. CUA activation reopens the window, so that interaction alone does not independently prove hidden-window monitoring cadence. Authored/requested 10-second snapshots are not implemented. |
| Launch at login | Pending | Plugin/registration/installed-path/OS-disable handling not implemented. No LaunchAgent was registered. Opt-in remains unavailable/default off. |
| Signed release, performance, delivery, accessibility | Pending | No Developer ID, notarization, staple, DMG distribution, native notifications/permission request, VoiceOver, 200% reading, 50-watch capacity or M1 performance qualification. |

The native development checks preceded a final rebuild with small auth/list hardening; those affected paths are covered by the deterministic suite. A subsequent native launch confirmed the corrected 0–0 empty range. The original prototype still builds independently with the patched Vite dependency (historical fixture scope remains unchanged). Treat the final source and dependency locks as authoritative. The prototype's old screenshots/builds remain fixture-only.

## Known incomplete engineering work

The first slice does not satisfy any full grouped release scenario merely because some subcases pass. Complete review/team watch discovery, local saved-query CRUD, snapshots/check attempts, GitHub notifications and per-source validators/X-Poll-Interval, activity hydration/eligibility, seen destinations, alerts/onboarding, recovery summaries and all reading surfaces are pending. Viewed-PR alerts must remain eligible when alert dispatch is added.

The initial store includes a shared disposable-content accounting/eviction path and a disk-space preflight, but **full 500-MiB/30-day/90-day, staged-download, durable-growth warning, low-disk cached-read availability, write/checkpoint stop/recovery, bounded compaction and migration fault coverage are not finished**. Schema 1 currently has only initialization/future-schema/corruption rejection; there is no version-upgrade pair or destructive-reset UI. Full resource/head/query generations and denial propagation into future Activity bodies must be extended alongside those resources. Selected-only cancellation, persisted reselect, byte-chunked details and full outstanding-work recovery need further work. Current known-PR overview checks do not prove requested-watch or checks cadence with the window closed.

Retry-After (numeric/date), quota reset and persistent coarse backoff are implemented; adaptive exponential backoff, separate budget accounting and measured cost/payload handling are still pending. These outstanding items are preserved as requirements, not treated as release exceptions.

## Primary references checked during implementation

- [Tauri system tray](https://v2.tauri.app/learn/system-tray/) and [single-instance plugin](https://v2.tauri.app/plugin/single-instance/).
- [GitHub native-stack REST schema](https://docs.github.com/en/rest/pulls/stacks?apiVersion=2026-03-10).
- Existing settled research and ticket contracts remain linked from the acceptance matrix. A successful live nonmember response does not validate positive stack order/provider behavior.
