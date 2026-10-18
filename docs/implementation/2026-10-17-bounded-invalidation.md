# Bounded global invalidation checkpoint — 2026-10-17

Status: production invalidation, restart/repair integration and targeted fault qualification implemented. Development version **0.1.0**, schema **5**. No Git repository or applicable AGENTS.md exists in the workspace or ancestors. Settled contracts are unchanged.

## Audit and implementation

After full-size functional content-budget qualification, remaining storage gaps include broad mutation bounds, recovery-completion mutations, query/sort/temp execution and accounting, remaining lifetimes/terminal compaction, physical faults/power loss, peak migration allocation and startup recovery UX. Global invalidation still deleted all pending/staged work and updated every check state in one transaction. It runs on identity/configuration invalidation and is also a prerequisite of the existing generic resource-purge path.

`invalidation.rs` now commits a **three-row barrier transaction**: a singleton invalidation marker, the incremented generation and incremented presentation revision. It then reclaims disposable work in restartable transactions with a **global maximum of 128 direct row mutations**, shared across legacy/Activity/source/team staging, pending descriptors, staged check contexts and check-completeness updates. Unchanged check states do not consume the allowance. Marker removal shares the final transaction's allowance. Free space is checked before the barrier and each cleanup transaction. No cursor or unbounded identity collection is retained in Rust.

All check readers treat snapshots as incomplete with `Content invalidated` while the marker exists, including snapshots whose physical rows have not yet been updated. Published check payloads remain readable as partial cached data, while terminal-check classification and recovery completion reject pending invalidation. Older generations cannot publish through the existing generation guard. Published content/checks, watch membership, identity/settings/Pause, minimal Activity/unseen/dedup/progress, terminal attempted/unavailable intents and membership/attempt history, and notification validators/server deadlines survive invalidation.

The operation returns success only after marker removal commits. A later failure preserves earlier batches and rolls back the failing batch. Retrying an unfinished invalidation retains its original generation/revision barrier. Startup completes pending invalidation **before** ordinary startup cleanup; current-session storage repair completes it before retention, health probing/checkpoint and resumed remote eligibility. The existing runtime storage-error path blocks reads from GitHub while preserving identity-validated cached access. A newly started protected runtime remains identity-locked under the existing contract.

This replaces the old single-transaction implementation of **global invalidation only**. Clear downloaded content, resource-specific purge, LRU admission and recovery-completion writes retain their separate implementations and remaining mutation-bound gaps. Invalidation now also discards abandoned team staging, which previously survived the global cleanup despite its generation being obsolete.

## Schema and compatibility

Schema **5** adds only the `invalidation` singleton table. The 0/1/2/3/4-to-5 upgrade remains one atomic, fully synchronized migration transaction, with admission before every required step and commit. The schema bump is intentional: schema-4 binaries must reject a database that may contain partially completed schema-5 invalidation rather than ignore its marker. There is no downgrade migration/reset or native binary-replacement qualification.

Migration fixtures now cover every supported input through schema 4. Historical reconstruction tests remove the new table when reconstructing an older schema. The small schema-4-to-5 DDL can fit in SQLite's minimum cache, so child tests explicitly flush dirty pages **without committing** before the pre-commit migration barrier, in addition to their small cache. The parent still requires real sidecar bytes beyond the header. These controls remain test-only; production cache/synchronization policies are unchanged.

## Evidence

All new inputs are deterministic fixtures. SQLite and killed processes are real; the runtime test uses the existing synthetic gh subprocess. No live GitHub read or native launch occurs.

| Subcase | State | Evidence |
| --- | --- | --- |
| Global transaction bound and progress | Pass, real SQLite | 257 rows in each of seven cleanup classes span 15 cleanup transactions. Every returned mutation count equals SQLite's total-changes delta and is at most 128, including marker removal. Barrier changes exactly three rows. Empty resume makes no mutations; retries retain the generation/revision. |
| Published and durable history | Pass, checked after every batch/restart | Published payload/check row, authored watch, bound identity, Pause, source health, unseen/dedup, baseline/catch-up progress, attempted/unavailable outcomes and original memberships/attempt, recovery boundary and notification validator/deadline survive. No terminal record becomes pending delivery. |
| Logical read barrier | Pass, real Store/read-only SQLite | A complete terminal-check fixture classifies before invalidation. Immediately after the barrier, its physical check row is still complete, but protected check reading exposes payload as incomplete and terminal classification returns no candidates. After cleanup, physical check state remains incomplete. Old-generation publication is rejected. |
| Barrier failure | Pass, SQLite trigger | Revision update fails after marker insertion and generation update; all three barrier changes roll back, leaving no marker or cleanup mutation. Removing the fault permits the normal operation. |
| Later-batch and final-marker failure | Pass, SQLite triggers/reopen | First 256 legacy-stage deletions commit; an Activity-stage fault rolls back the companion final legacy deletion. Repeated invalidation and failed startup preserve the barrier. A later marker-delete fault rolls back the final seven check-state updates and marker removal together, preserving logical incompleteness until repair. Fault removal permits complete startup cleanup without another generation increment. |
| Process interruption | Pass, **five actual SIGKILL cases** | Kill after barrier commit, before/after first batch commit and before/after final marker-removal commit. Parent verifies SIGKILL/reaps and inspects raw SQLite before restart: stage rows are 257/257/129/0/0; physically complete check rows are 257/257/257/7/0; marker survives in every case except final commit. Integrity passes. Production Store startup finishes remaining work and preserves the barrier/history. The child represents an already-open session so active traversal fixtures are not prematurely discarded by startup cleanup. |
| Worker protection/repair and restart | Pass, real Runtime/process/SQLite with synthetic gh | Validated paused runtime faults during configuration invalidation after two committed batches. Cached overview remains accessible, check health is incomplete, Refresh is rejected, and failed repair starts no extra reads. Removing the fault completes cleanup/repair under a controlled remote deadline, preserving identity, Pause, notification deadline and ledgers; old-generation publication remains rejected. Restart after repair remains paused and identity-locked with no new child invocation. |
| Schema-5 migration admission/crash matrix | Pass, **60 actual SIGKILL cases / 40 injected space failures** | Schemas 0/1/2/3/4 in DELETE and WAL modes cover every required step, final commit admission and pre/post-commit crash boundaries. Pre-commit states preserve the entire previous schema/data; committed states match the complete schema-5 reference. Raw snapshots precede startup cleanup; sidecar observation proves disk writes before pre-commit migration kills. Existing SQL failure, future/corrupt preservation and paused runtime migration-stop/repair tests also pass. |

The new invalidation switches and all process barriers are test-only. A process kill or SQL abort does not qualify power loss, physical exhaustion, read-only storage or IOERR.

## Limits and acceptance

The mutation bound applies to invalidation transactions, not total synchronous operation duration, SQLite scans/sorting/index/page work or temporary space. Scheduling remains blocked under the runtime's serialized access until invalidation succeeds or storage protection takes over. No wall-time, memory, whole-database-size, VACUUM or file-shrink guarantee is added. Full-size budget functional evidence remains applicable to the unchanged accounting/LRU algorithms and is rerun in the workspace suite; worst-case performance/cardinality/temp qualification remains Pending.

Clear/purge/LRU/recovery-completion mutation bounds, remaining source/metadata lifetimes and compaction, transient/temp accounting, physical I/O/power loss, sufficient peak migration allocation and missing/legacy/corrupt/unreadable startup UX remain Pending. Actual native schema-5 migration and compatible/incompatible signed binary replacement remain Pending.

Native delivery remains absent. Existing attempted/unavailable history stays terminal; unattempted-intent reconstruction, reservation/revalidation, durable dispatch attempts, ambiguous OS dispatch and click routing remain unimplemented/unverified. No dispatch readiness or native/live capacity is inferred. All **29 grouped release scenarios remain Pending**; existing Blocked positive live fixtures remain Blocked.

## Verification

- `cargo test -p gitjuke-core --offline invalidation`: Pass; new mutation/read-barrier, failure/restart, five-kill and worker tests (plus the existing matching head-invalidation test).
- `cargo test -p gitjuke-core --offline --lib migrations`: Pass; 60 actual SIGKILL cases, 40 injected space failures and runtime migration stop/repair.
- `cargo test --workspace --offline`: **113 tests pass** (112 core, one desktop IPC), including all full-size content-capacity tests on schema 5. Five new test entries comprise three invalidation behaviors, their subprocess-only entry and the runtime test.
- `npm run check`: Pass; TypeScript/frontend production build and workspace Clippy with warnings denied.
- `cargo fmt --all -- --check`: Pass.
- `npm run tauri build -- --debug --bundles app --target aarch64-apple-darwin`: Pass; **31.05 MiB** local Apple Silicon development `.app`, build only.
- Bundled executable SHA-256: `c2f2fd209ad929c903d07b0b58a0b9649dca2d30a71360f4bee1c5b1603c7570`. All three invalidation environment-switch strings and the migration phase switch are absent from the executable.
- Acceptance matrix inspection confirms exactly 29 grouped rows, all Pending.

The initial migration run established that schema-5 DDL alone did not spill WAL pages under the small-cache fixture; the test-only explicit flush now establishes disk-backed pre-commit writes rather than dropping that assertion. The first invalidation crash child ran ordinary startup cleanup before entering the operation, which removed its active-stage fixture; it now models an already-open session before invoking the real invalidation method. No production behavior failure was accepted or contract weakened.

No native app launch, live GitHub read, credential/content/permission/login change, signing, publication or distribution occurred.
