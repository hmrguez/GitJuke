# Bounded first-page source initialization — 2026-10-17

Status: production implementation and targeted storage/fault/restart qualification complete. Development version **0.1.0**, schema **8** unchanged. No Git repository or applicable AGENTS.md exists in the workspace or ancestors. Settled contracts are unchanged.

## Audit and implementation sequence

The next storage audit found that `Store::begin_source` deleted every prior `source_stage` row for a discovery source in one transaction. The existing PR-purge restart guard protected denial-affected traversals, but ordinary first-page resets had no durable guard if their deletion were split into transactions. Recovery readiness also used physical resource-health rows without checking source-restart guards.

The selected sequence was: reuse the existing schema-8 guard as a publication barrier; replace the broad deletion with bounded source-local transactions; guard staging and recovery readiness; qualify mutation counts, SQL failures, process death and actual worker repair; update acceptance evidence. This finishes one first-page storage prerequisite. The audit retains LRU admission, source/watch/check publication, team enumeration and recovery-completion mutation bounds as separate work.

## Production behavior

`source_cleanup.rs` first commits `source_restart(source)` plus a presentation revision increment, **two direct mutations** for a new guard. A previously committed guard is reused without another revision increment. No global generation or watch epoch changes. Space admission runs before the barrier and every cleanup transaction.

Each cleanup transaction removes at most **128 direct rows**, scoped by literal source equality and a bounded rowid subquery. Final guard removal shares that allowance: the 257-row fixture commits 128, 128 and 2 mutations (one remaining staged row plus the marker). Rust retains no victim list or cleanup cursor. Earlier committed deletions survive a later failure; the failing transaction rolls back in full. `Store::begin_source` completes synchronously under existing runtime serialization or returns a storage error into protection.

While the guard exists, `stage_source` and `publish_source` reject that source. Existing list/health readers retain published content while reporting incomplete discovery. Health wording now covers both resource denial and interrupted initialization. A newly committed barrier revision lets the existing worker error path invalidate the displayed snapshot after partial cleanup failure. Other source staging, published memberships, content, watch identities/reasons, settings/Pause, identity, Activity/unseen/dedup/progress/baseline/recovery boundary, terminal attempted/unavailable intents and membership/attempt history, and notification validators/server deadlines are preserved.

Recovery completion now checks restart guards for authored/direct/teams and currently enrolled team sources. A physically complete old resource-health row cannot make unfinished initialization ready. Obsolete team-source guards do not independently block recovery. Summary/progress publication semantics and mutation counts are otherwise unchanged; native delivery remains absent.

A successful fresh first-page response enters `begin_source`, completes remaining cleanup, removes the guard and stages only its new response before continuing/publishing normally. Existing worker continuation logic replaces a guarded obsolete continuation with a fresh first-page traversal before its next discovery page read. The legacy authored adapter also uses `begin_source`. This slice does not change team-list `teams_stage` initialization or Activity/check staging initialization.

Startup retains its existing globally bounded policy: abandoned source staging and discovery descriptors are discarded, **the restart guard survives**. A source cannot publish until fresh initialization succeeds. In-session storage repair preserves the guard and may leave old source staging until the next first-page initialization. It does not claim the interrupted traversal completed. No new schema/migration is necessary: the existing schema-8 guard already blocks older schema-8 publication and forces fresh traversal initialization; historical implementations clear their source staging atomically.

## Evidence

All inputs are deterministic. SQLite, filesystem persistence, Rust runtime, subprocess execution and SIGKILL are real. Worker responses use the existing synthetic gh executable. No live GitHub read or native app launch occurred. Local environment is **arm64 macOS 27.0 (26A428)**; this does not qualify physical M1/8-GiB/macOS-15 performance or expand the release matrix.

| Subcase | State | Evidence |
| --- | --- | --- |
| Barrier and cleanup mutation bounds | Pass, real SQLite | New barrier changes exactly two rows, retrying an existing guard changes none. The 257-row source requires 128/128/2 cleanup mutations; each returned count equals SQLite total-changes and is at most 128. Empty completed cleanup changes nothing. Generation remains unchanged. |
| Logical guard and isolation | Pass, writable/read-only Store | Staging/publication reject pending initialization; cached list remains available/incomplete. All seeded published/durable table values are compared after each batch and raw restart. Independent source staging remains unchanged by initialization. Fresh complete publication restores a complete list with original membership. |
| Barrier failure | Pass, SQLite trigger | Revision-update failure rolls back the guard insert; all 257 old stage rows and the original revision survive. |
| Later-batch failure | Pass, SQLite trigger/retry | Deletion of row 200 aborts the second batch. First 128 deletions survive; all 129 remaining rows survive repeated failure, guard/revision remain stable and published/durable values are unchanged. |
| Final-marker failure and restart | Pass, SQLite trigger/reopen | Failure deleting the guard rolls back its companion final stage deletion. Raw reopen retains the last stage row and marker. Normal startup discards abandoned staging but preserves the guard and history; publication is rejected until fresh initialization. |
| Process interruption | Pass, **six actual SIGKILL cases** | Before/after barrier commit; before/after first cleanup commit; before/after final guard-removal commit. Raw inspection precedes startup cleanup. Remaining source-stage rows are 257/257/257/129/1/0; marker is absent only before the barrier commit and after final commit. SQLite integrity passes; original published/durable values and independent staging survive. Protected readers honor the guard; production restart plus fresh initialization/publication restores complete membership. |
| Recovery readiness | Pass, real Store | Otherwise-ready recovery with physically complete source health remains in recovery with unchanged revision while its source guard exists. Successful initialization removes that blocker and normal completion succeeds. |
| Worker failure, repair and fresh traversal | Pass, actual Runtime/process/SQLite with synthetic gh | A manual first-page discovery during Pause commits the first 128 deletions then faults. Storage protection emits the committed revision, rejects Refresh, preserves cached list/membership and ledger counts. Fault removal and repair behind a controlled remote deadline start no extra children and preserve Pause, identity and polling deadline. Guard survives repair. Releasing the deadline permits fresh initialization/publication, removes old staging/guard and retains independent staging, watch count and history. Restart stays locked/paused without automatic reads. |

All new crash hooks and environment switches compile only in tests. SIGKILL and SQL abort do not establish power-loss durability, physical filesystem exhaustion, read-only filesystems or IOERR behavior. Normal startup separately removes unrelated abandoned staging; isolation claims apply before that cleanup.

## Limits and next prerequisites

The bound is per **first-page source cleanup transaction**. It does not bound total synchronous duration, SQLite query/scanning/sorting/index/page work, temporary storage or process memory. It does not bound the following source-page staging/publication transaction, shared LRU admission or all first-page writers. There is no VACUUM, shrink or whole-database-size guarantee.

Next useful candidates remain shared LRU admission with atomic publication, source/watch/check publication, team/Activity/check first-page reset bounds, and recovery-completion bounds preserving summary/progress semantics. Broader cardinality/time/temp qualification, remaining metadata lifetimes/terminal compaction, startup recovery UX for missing/legacy/corrupt/unreadable databases, physical storage/power-loss faults, sufficient peak migration space and transient/temp accounting remain Pending.

Native alerts, reservation/revalidation, unattempted-intent recovery, pre-dispatch attempts, ambiguous dispatch and click routing remain unimplemented/unverified. Attempted/unavailable history stays terminal. All **29 grouped release scenarios remain Pending** and existing Blocked live fixtures remain Blocked; no dispatch readiness or specification change follows from these tests.

## Verification

- Targeted source-cleanup tests and paused worker failure/repair test: Pass. Six new test entries comprise five source-cleanup entries (including the subprocess-only child) and one worker test.
- `npm run check`: Pass; TypeScript/frontend production build and workspace Clippy with warnings denied.
- `cargo fmt --all -- --check`: Pass.
- `npm run tauri build -- --debug --bundles app --target aarch64-apple-darwin`: Pass; **31.15 MiB** Apple Silicon development `.app`, build only.
- Bundled executable SHA-256: `e44b01a2ed256df8926ea5843aa203dee7978822fce6ea392d00ff1678585b9c`. All three new source-initialization fault-switch strings, the PR-purge phase switch and the migration phase switch are absent from the executable.
- Acceptance matrix remains exactly 29 grouped rows, all Pending.

`cargo test --workspace --offline`: **144 tests pass** (143 core, one desktop IPC), including full-size 500-MiB functional capacity/fault fixtures, all 120 migration SIGKILL cases and 88 injected migration space failures, plus the six new source-initialization SIGKILL cases. No native migration or dispatch readiness is inferred.

No live GitHub read, native app launch, GitHub content/credential change, notification-permission/login change, signing, publication or distribution occurred.
