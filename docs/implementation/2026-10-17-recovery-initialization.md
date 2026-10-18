# Bounded recovery initialization checkpoint — 2026-10-17

Status: production storage prerequisite implemented through recovery initialization, restartable SQLite batches, process-kill fixtures and worker protection/repair. Development version **0.1.0**, schema **4**. No Git repository or applicable AGENTS.md exists. Settled decisions are unchanged.

## Audit and behavior

After bounded startup cleanup, `Store::activity_recovery` still reset every hydration target and provisional pending intent in one transaction. The same path runs on Resume, long scheduler gaps, transport recovery and storage repair. `recovery_cleanup.rs` now owns that initialization.

A small transaction first persists recovery mode, the boundary and notifications-incomplete state, together with the revision (two direct row mutations). Normal/offline cycles take the new restoration boundary; incomplete recovery/recovery-offline cycles retain their original boundary. This commits before cleanup, so an interruption cannot cause a later restart to move the boundary past post-boundary activity.

Subsequent transactions share a **global 128 direct row mutation maximum** across target resets (`done=0,due=0`), abandoned Activity staging, Activity pending jobs and provisional `pending`→`recovery` intent transitions. Already-reset targets do not consume the allowance. Rowid ordering makes each batch deterministic; no cursor or victim list is retained in Rust or persisted. Space is checked before the boundary transaction and every cleanup transaction. The caller returns success only after an empty cleanup pass. Initialization remains synchronous under the runtime's serialized access: scheduling and remote reads cannot observe partial initialization as ready.

Earlier batches survive a later failure; the failing batch rolls back. Retrying/reopening repeats only unfinished work and preserves the committed boundary. Existing runtime storage-error handling opens the current-schema protected cache after startup failure, or stops an existing worker. Repair must finish initialization and the existing committed health/checkpoint requirements before allowing reads; Pause remains authoritative and startup content remains identity-locked until explicit validation.

Published content/checks, watch membership, stored identity/preferences, Activity/unseen/deduplication/progress/catch-up membership, terminal intent states/membership, attempts and server polling deadlines survive. Schema remains 4; migrations remain fully atomic. This supersedes only the historical single-transaction recovery-initialization implementation.

The pending-to-recovery transition remains provisional. It does **not** reconstruct unattempted membership into a deliverable summary. Terminal `attempted` and `unavailable` outcomes remain terminal; native delivery is unavailable and no replay backlog is introduced.

## Evidence

New evidence is local, deterministic and uses real SQLite. The worker uses the existing synthetic gh subprocess. Process-kill tests launch the actual Rust test executable, enter `Store::open` followed by `activity_recovery`, stop at test-only barriers, send actual SIGKILL and reap the child. No production fault hooks, live GitHub reads or native app launches are introduced.

| Subcase | State | Evidence |
| --- | --- | --- |
| Global mutation bound and idempotence | Pass, real SQLite | 257 rows in each of four cleanup classes span nine mutating transactions plus an empty pass. Every returned count equals SQLite total-changes delta and is at most 128. Boundary/revision transaction changes two rows. Repeated recovery changes only those two rows, retaining the original boundary. |
| Durable state preservation | Pass, real SQLite | Every batch checks identity/Pause, original unseen value, baseline/catch-up progress, published content/checks, authored watch membership, explicit attempted/unavailable states, intent members, attempt history and notification validator/deadline. |
| Late-batch rollback and restart | Pass, SQLite triggers | First 256 target resets commit before the final target faults. A subsequent intent fault rolls back companion pending-job deletions after earlier batches commit. Reopen/retry retains the original boundary and terminal ledgers; fault removal completes initialization. |
| Kill after boundary commit | Pass, actual subprocess SIGKILL | Boundary 123 persists; all 257 targets remain unreset. SQLite integrity passes; reopening and recovery with time 999 finishes while retaining boundary 123. |
| Kill inside uncommitted batch | Pass, actual subprocess SIGKILL | All 257 targets retain their original values after the first uncommitted batch rolls back; boundary survives. Normal reopening/recovery completes without lost durable history. |
| Kill after batch commit | Pass, actual subprocess SIGKILL | Exactly 129 unreset targets remain after the first 128-row commit. Integrity passes; reopening finishes with the original boundary and durable history. |
| Worker protection and repair | Pass, actual worker/process/SQLite with synthetic gh | Startup commits 256 target resets, then faults. Runtime starts protected, locked and paused. Refresh and cached-list exposure are rejected while locked; another failed repair starts no remote children. Fault removal completes repair without resuming Pause or starting reads, preserves the boundary/server deadline, and leaves no pending delivery. Explicit Refresh validates the bound identity and permits cached list access while Pause remains set. |

## Limits and remaining work

This bounds direct mutations per **recovery-initialization** transaction, not total initialization duration, SQLite scans/sorts/temp/index/page work or whole-database size. Recovery completion still inserts summary membership and deletes catch-up progress transactionally without a row-mutation bound; target enrollment, broad invalidation/purge, LRU admission and other publication paths also need broader bounds. No VACUUM or file-shrink guarantee exists.

Physical exhaustion/readonly/IOERR, full-size 500-MiB capacity, temp accounting, peak migration space, migration/ingestion/intent/attempt/delivery kills and missing/legacy/corrupt/unreadable startup UX remain Pending. The three initialization kills do not qualify power loss or those other boundaries. Actual native schema-4 migration, native alerts and all **29 grouped release scenarios remain Pending**; existing Blocked live fixtures stay Blocked. No dispatch readiness is inferred.

## Verification

- `cargo test --workspace --offline`: **89 tests pass** (88 core, one desktop IPC), including four new recovery-module entries (one subprocess-only child entry) and one new worker test.
- `npm run check`: Pass; TypeScript/frontend build and workspace Clippy with warnings denied.
- `cargo fmt --all -- --check`: Pass.
- Strengthened explicit terminal-state assertions: targeted `cargo test -p gitjuke-core --offline --lib recovery_cleanup` passes.
- `npm run tauri build -- --debug --bundles app --target aarch64-apple-darwin`: Pass; **31.02 MiB** local Apple Silicon development `.app`, build only.
- Bundled executable SHA-256: `322d13a11fc1a1a6cc1d5a0e6cd305274133513f07ceec38956a2e0e1b29445e`.

The initial workspace run hit a timeout in the existing `inclusive_bounds_and_stdin_closed` transport fixture (its configured deadline is two seconds; the shared error text says 30 seconds). A separate transport-suite rerun passed all five tests. No transport code, timeout or assertion was changed. The final workspace result above records the subsequent full rerun; the initial transient timeout remains noted rather than treated as a specification exception.

No credentials, GitHub content, permissions, login, signing, publication or distribution settings changed.
