# Bounded check-access denial checkpoint — 2026-10-17

Status: production per-PR check purge, reader/classification barriers, startup/repair integration and targeted fault qualification implemented. Development version **0.1.0**, schema **7**. No Git repository or applicable AGENTS.md exists in the workspace or ancestors. Settled decisions are unchanged.

## Audit and scope

The remaining audit confirmed unbounded mutations in shared LRU admission, full PR/resource purge, source/watch/check publication and recovery completion. It also found that `deny_checks` deleted every staged/published check context and updated every check snapshot for a PR in one transaction, retained check-derived Activity excerpts, and lacked a per-PR epoch barrier. Paginated check denials retained prior check payloads instead of invoking that purge.

This slice completes the **check-access denial** path. It does not claim that full PR denial (`activity_purge`, `purge_watch`, generic `purge`), ingestion/publication, shared admission or recovery completion now have mutation bounds. Query/sort/temp execution and accounting, source/metadata lifetimes, terminal compaction, physical storage/power loss, peak migration allocation and missing/legacy/corrupt/unreadable startup recovery UX remain open.

## Production behavior

`check_purge.rs` first commits a **three-row barrier**: update the current watch epoch/error/local retry due, insert a marker keyed by stable PR ID, and increment the presentation revision. The update requires the request's original watch epoch. A stale or missing target makes no mutations; an unfinished purge resumes without another epoch/revision/deadline change. The global generation and unrelated PR epochs remain unchanged.

Cleanup uses restartable transactions with a **global maximum of 128 direct mutations** across five classes: staged/published check contexts, check-derived Activity payloads, staged check Activity, matching pending check traversals, and check-state completeness/error updates. Final marker removal shares the allowance; already-normalized check states consume none. Each transaction processes at most one marker identity, without an unbounded Rust collection. Free-space admission precedes the barrier and every cleanup batch.

While the marker exists, check readers return `Checks unavailable` without its check payloads. PR summaries/overviews, non-check Activity excerpts and other PRs remain readable. Activity's direct payload join and generic payload reads hide affected check excerpts even when physical rows survive a later batch failure. Minimal Activity records, unseen values and dedup/progress/terminal intent history survive. Matching uses exact stable IDs, case-insensitive literal repository equality and exact PR numbers; pending keys require the literal `checks:<stable-id>:` prefix, preserving neighboring IDs.

Terminal classification and check/head publication reject the affected marker. Recovery completion rejects pending check purge. The epoch increment also rejects late old-target success and denial results after reclamation completes. A newly authorized current-epoch head/check read may restore access; missing checks are never inferred to be a complete result. Existing notification validator/server polling deadlines, identity, settings/Pause, watches, Activity targets and durable history are preserved.

Startup completes pending check purges before Clear, global invalidation and ordinary abandoned-work cleanup. Current-session repair completes them before retention/probing/checkpoint and remote eligibility. Clear and invalidation finish earlier check purges before starting their own operation. A later fault preserves earlier committed batches and rolls back the failing batch. The existing worker storage-error path emits the committed revision, blocks remote reads and retains validated cached access; a protected startup remains identity-locked. No new reset, replay queue or native delivery path is introduced.

Head-field GraphQL denial uses the bounded purge. Paginated checks now route explicit HTTP resource denial and `FORBIDDEN`/`NOT_FOUND` errors scoped to the GraphQL `repository` path through the same path, with global generation, cancellation, identity-lock and watch-epoch guards. Other GraphQL errors still retain prior content as incomplete rather than becoming an inferred empty success. Tests exercise head-field `FORBIDDEN`, pagination `FORBIDDEN` and HTTP 403; live provider compatibility remains unverified.

## Schema and migration

Schema **7** adds only the `check_purge(pr_id)` marker table. Schema-6 binaries must reject the database rather than ignore unfinished denial cleanup. Schema 0–6 upgrades remain fully synchronized and atomic, with space admission before every required step and final commit. Historical-schema reconstruction fixtures remove the new table.

The existing migration matrix now covers **98 actual SIGKILL cases and 70 injected space failures**, across schemas 0–6 and DELETE/WAL modes. Pre-commit dirty-page flushing and sidecar-byte assertions remain test-only. Actual native schema-7 migration, peak migration allocation and signed compatible/incompatible binary replacement remain Pending.

## Evidence

Inputs are deterministic. SQLite, disk persistence, subprocess execution and SIGKILL are real. Worker and integration tests use the existing synthetic gh executable; no live GitHub read or native app launch occurred. Local environment: arm64 macOS **27.0 (26A428)**. This neither qualifies the physical M1/8-GiB/macOS-15 performance target nor expands the release matrix.

| Subcase | State | Evidence |
| --- | --- | --- |
| Global transaction allowance | Pass, real SQLite | 257 rows in each of five cleanup classes require 11 cleanup transactions. Every successful batch's total-changes delta equals its returned count and is at most 128, including final marker deletion. Barrier changes exactly three rows. Two independent markers with 257 already-normalized states each require only one marker deletion per transaction. Missing/stale targets and empty resume make no mutations. |
| Immediate visibility and classification | Pass, writable/read-only Store | A terminal legacy status and check Activity payload are readable before the barrier. Immediately afterward, physical check states are still complete but readers hide contexts/excerpts and terminal classification returns no records. Minimal Activity remains unseen; neighboring PR checks, PR overview/list content and ordinary comment excerpts survive. |
| Isolation and preservation | Pass, checked through batches/raw restart | Global generation, neighboring epoch, identity/Pause, watch membership, all 259 Activity/seen/dedup records, terminal attempted/unavailable intents, memberships/attempts, progress, recovery boundary, Activity target due/completion and notification validator/deadline survive. `P_10` pending/check data and PR 10 excerpts are untouched by denial of `P_1`; case-variant repository spelling matches the same denied resource. Normal startup separately removes abandoned staging/descriptors under its existing policy. |
| Stale publication/denial and restoration | Pass, real Store | Pending markers reject even a fabricated current-epoch head/check publication. After cleanup, the old epoch cannot publish or issue another denial. A new current-epoch authorized head plus complete explicit check page restores successful state; the old denial remains rejected. |
| Recovery readiness | Pass, real Store | Otherwise-ready recovery remains incomplete with unchanged revision while a check purge is pending; completing purge permits normal completion. |
| Barrier and later faults | Pass, SQLite triggers/reopen | A marker-insert abort rolls back the preceding epoch/error/due update. A context-delete abort at row 200 retains the first 128 committed deletions and rolls back its whole later batch. A payload-delete abort rolls back the companion final context delete and preceding excerpt deletions, retaining one context and all 257 check excerpts. Protected reads hide them. A final-marker abort rolls back its five companion state updates. Reopening/retrying retains the original barrier; removing faults completes cleanup. |
| Process interruption | Pass, **five actual SIGKILL cases** | Kill after barrier commit, before/after first cleanup commit and before/after final marker-removal commit. Raw inspection precedes startup cleanup: denied contexts are 257/257/129/0/0; physically complete denied states are 257/257/257/5/0; marker survives except after final commit. Integrity checks pass. Neighbor data and durable history survive; protected readers honor the marker and production startup completes reclamation. |
| Worker denial and paused repair | Pass, actual Runtime/process/SQLite with synthetic gh | A manual head read during Pause returns scoped denial and faults during a later cleanup batch. Protection exposes unavailable checks, retains overview and neighbor checks, emits a committed revision and rejects Refresh. Failed repair starts no extra children. Successful repair under a controlled remote deadline preserves Pause, identity, notification deadline and ledgers; stale denial is rejected. Restart stays paused and identity-locked with no extra reads. |
| Partial-purge startup | Pass, actual Runtime/SQLite | Startup encounters a later-batch purge fault and opens current-schema protected storage, locked and paused, with no synthetic child reads. Failed repair remains stopped. Fault removal permits reclamation/probing, preserving stable identity, original recovery boundary and notification validator/deadline; the identity lock and Pause remain. |
| Paginated access denial | Pass, actual Runtime/process/SQLite with synthetic gh | Separate resource-scoped GraphQL `FORBIDDEN` and HTTP 403 pagination cases purge staged/published checks and check excerpts. Watch count, Activity IDs/seen values and account availability survive. Checks remain explicitly unavailable/incomplete. |
| Migration matrix | Pass, **98 kills / 70 space failures** | Every supported input schema 0–6 in DELETE/WAL modes preserves the full prior schema/data before commit or matches the complete schema-7 reference afterward. Raw inspection precedes startup cleanup, with actual pre-commit sidecar writes required. Existing SQL/future/corrupt and runtime migration stop/repair tests remain covered. |

All new fault hooks/environment switches are test-only. SIGKILL and SQL abort do not establish power-loss, physical exhaustion, read-only filesystem or IOERR behavior.

## Limits and acceptance

The 128 bound covers direct mutations per check-purge transaction, not SQLite scan/sort/index/page work, total synchronous duration, temp allocation, process memory or whole-database size. Check denial remains serialized until completion or storage protection. No VACUUM or file-shrink promise is added. Existing full-size 500-MiB functional admission/fault tests are rerun against schema 7; they do not establish worst-case query/cardinality/temp/native responsiveness or actual 50-watch capacity.

Full PR/resource purge, LRU admission, source/watch/check publication and recovery-completion mutation bounds remain Pending, alongside the other audit gaps above. Native alerts, reservation/revalidation, unattempted-intent recovery, durable pre-dispatch attempts, ambiguous-delivery semantics and click routing remain unimplemented/unverified. Attempted/unavailable history remains terminal. All **29 grouped release scenarios remain Pending**; existing Blocked positive live fixtures stay Blocked. No dispatch readiness or specification revision follows from this slice.

## Verification

- `cargo test --workspace --offline`: **129 tests pass** (128 core, one desktop IPC), including full-size capacity and expanded migration matrices. Nine new entries comprise six check-purge tests (including the subprocess-only entry), two worker tests and one integration test covering both pagination denial modes.
- Targeted `check_purge` core/worker tests and pagination-denial integration tests: Pass.
- `npm run check`: Pass; TypeScript/frontend production build and workspace Clippy with warnings denied.
- `cargo fmt --all -- --check`: Pass.
- `npm run tauri build -- --debug --bundles app --target aarch64-apple-darwin`: Pass; **31.10 MiB** Apple Silicon development `.app`, build only.
- Bundled executable SHA-256: `ecb3bb0a0d0d3ef5e59442b643c0b8a0637da6719637b79646277452f2d7ae44`. All three check-purge fault-switch strings and the migration phase switch are absent from the executable.
- Acceptance matrix remains exactly 29 grouped rows, all Pending.

The first pagination regression assertion incorrectly expected every PR's check excerpt to disappear as soon as PR 1 denial completed. It now checks the denied PR explicitly, retaining isolation. A quoted JSON fixture string was corrected during compilation. No production requirement was weakened.

No live GitHub read, native app launch, GitHub content/credential change, notification-permission/login change, signing, publication or distribution occurred.
