# Bounded Clear downloaded content checkpoint — 2026-10-17

Status: production clear barrier, reclamation, reader/startup/repair integration and targeted fault qualification implemented. Development version **0.1.0**, schema **6**. No Git repository or applicable AGENTS.md exists in the workspace or ancestors. Settled contracts are unchanged.

## Audit and implementation

The remaining storage audit found that Clear downloaded content deleted every cache/staging/check row and reset every target/resource in one transaction. This slice replaces that operation with a durable logical barrier followed by restartable reclamation. LRU admission, resource-specific purge, source/watch publication and recovery-completion mutation bounds remain separate gaps.

`clear_content.rs` commits a **three-row barrier transaction**: a singleton `content_clear` marker, a generation increment and a presentation revision increment. Cleanup transactions then share a **global maximum of 128 direct row mutations** across ordinary content (including Activity excerpts), legacy/Activity/source/team staging, pending descriptors, all staged/published check contexts, Activity target due resets, check completeness and resource completeness. Final marker removal shares the allowance. Already-normalized target/check/resource rows consume no allowance. Free space is checked before the barrier and every cleanup transaction; no unbounded victim collection or persisted cursor is needed.

Once the barrier commits, detail/list/check readers hide downloaded payloads even if their physical rows have not yet been reclaimed. Source health reports incomplete with `Downloaded content cleared`. Activity still exposes minimal retained records and their original unseen values, while its direct payload join hides excerpts, actor, URL and head payload fields. Terminal-check classification and recovery completion reject pending clear. Old-generation results cannot republish cleared content. The runtime emits the committed revision even when a later cleanup batch fails, so an open reader reloads through the barrier.

Earlier batches survive a later error; the failing batch rolls back. Retrying an unfinished clear retains the original generation/revision. Startup completes pending clear before invalidation and ordinary startup cleanup. Current-session repair completes it before retention, the committed-write/checkpoint probe and remote eligibility. Invalidation completes an unfinished clear before starting a new invalidation; Clear finishes an earlier invalidation before committing its own barrier. Runtime execution remains serialized, and storage faults enter the existing protection path.

Identity, settings/Pause, watch membership, minimal Activity/unseen/dedup/progress, baseline/recovery boundary, terminal attempted/unavailable intents and membership/attempt history survive clear. Notification validators and server polling deadlines are preserved. Activity target identity/completion remains; its local hydration due is reset as in the previous implementation. Team staging is now also removed. No native alert or replay backlog is introduced.

## Schema and compatibility

Schema **6** adds only the singleton `content_clear` table. Older binaries must reject this schema rather than ignore an unfinished logical clear. Schema 0/1/2/3/4/5 upgrades remain atomic, fully synchronized, with space admission before every required step and final commit. There is no destructive reset, downgrade migration, VACUUM or file-shrink guarantee.

Migration tests iterate every supported input version. Historical-schema reconstruction fixtures now remove `content_clear` when reconstructing schemas 1 and 3. The complete migration matrix expands to **78 actual SIGKILL cases and 54 injected space failures**, covering DELETE and WAL modes. Its test-only pre-commit dirty-page flush and sidecar-byte assertions remain intact. Actual native schema-6 migration and signed compatible/incompatible binary replacement remain Pending.

## Evidence

All new input is deterministic. SQLite, filesystem persistence and killed subprocesses are real. The worker regression uses the existing synthetic gh executable, not live GitHub. No native launch occurs. Tests ran on local arm64 macOS **27.0 (26A428)**; this neither qualifies the physical M1/8-GiB/macOS-15 performance target nor expands the advertised release matrix.

| Subcase | State | Evidence |
| --- | --- | --- |
| Global transaction bound | Pass, real SQLite | 257 rows in each of ten cleanup classes require 21 cleanup transactions. Every returned mutation count matches SQLite total-changes and is at most 128, including final marker removal. Barrier changes exactly three rows. Empty resume makes no mutations. A separate fixture with 1,000 already-normalized check states and targets needs only the marker deletion. |
| Logical visibility and durable history | Pass, writable/read-only Store | Before the barrier a terminal check classifies and an Activity excerpt is readable. Immediately after it, physical snapshots remain complete but detail/list/check payloads and Activity excerpts are hidden; minimal Activity remains unseen and health is incomplete. Every batch/restart preserves identity/Pause, watches, ledgers, terminal states, target completion, recovery boundary and notification validator/deadline. Old-generation publication is rejected. |
| Recovery completion barrier | Pass, real Store | An otherwise-ready recovery remains incomplete with unchanged revision while clear is pending. Removing only the marker in the controlled fixture permits normal completion, demonstrating that the logical barrier is the blocking condition. Production never removes it before reclamation finishes. |
| Barrier failure | Pass, SQLite trigger | A revision-update abort rolls back marker and generation together. All 257 content rows and original revision/history remain. |
| Later batch and final-marker failure | Pass, SQLite triggers/reopen | A delete abort at content row 200 retains the first 128 committed deletions and rolls back every earlier delete in the failing second batch. Repeated clear and startup retain the barrier/generation; protected readers hide the remaining payloads. A final-marker abort rolls back its ten companion resource-state updates. Removing the fault lets startup complete without another barrier increment. |
| Process interruption | Pass, **five actual SIGKILL cases** | Kill after barrier commit, before/after first batch commit, and before/after final marker-removal commit. Raw SQLite inspection precedes startup cleanup: content counts are 257/257/129/0/0; physically complete resource rows are 257/257/257/10/0; marker survives except after final commit. Integrity checks pass. Protected reads honor unfinished clear; production reopening completes cleanup and preserves durable history. |
| Worker protection/repair | Pass, real Runtime/process/SQLite with synthetic gh | Paused validated runtime faults after committed cleanup batches. Clear returns an error but emits its committed barrier revision; cached payload views are empty/incomplete, minimal Activity and ledgers survive, Refresh is blocked. Failed repair starts no additional children. Removing the fault completes cleanup/probing while retaining Pause, identity, polling deadline and ledgers; old-generation publication remains rejected. Restart after repair stays paused and identity-locked with no additional child reads. |
| Migration admission/crash matrix | Pass, **78 kills / 54 space failures** | Every supported input schema 0–5 in DELETE/WAL modes rolls back completely before commit or exposes the complete schema-6 reference after commit. Raw inspection precedes startup cleanup; pre-commit sidecar writes remain required. Existing SQL-failure, future/corrupt preservation and runtime legacy-migration stop/repair coverage is retained. |

Clear crash hooks and environment switches compile only in tests. Process death and SQL abort do not establish power-loss, physical exhaustion, read-only filesystem or IOERR behavior.

## Limits and acceptance

The mutation bound applies to each clear transaction, not total synchronous duration, SQLite scan/sort/index/page work, temporary allocation or memory. Clear still completes synchronously under serialized runtime access or enters storage protection. Physical files may retain reusable free pages and exceed the payload ceiling. Full-size functional content-budget qualification is rerun against schema 6; this slice does not add worst-case cardinality/time/temp/native responsiveness or 50-watch capacity evidence.

LRU admission, resource-specific purge, source/watch publication and recovery-completion mutation bounds; remaining source/metadata lifetimes and terminal compaction; transient/temp accounting; sufficient peak migration allocation; physical storage/power loss; and missing/legacy/corrupt/unreadable startup recovery UX remain Pending.

Native delivery remains absent. Existing attempted/unavailable history stays terminal. Unattempted-intent reconstruction, reservation/revalidation, pre-dispatch attempts, ambiguous OS delivery and click routing remain unimplemented/unverified. No delivery readiness follows from these storage tests. All **29 grouped release scenarios remain Pending**, and existing Blocked positive live fixtures remain Blocked.

## Verification

- `cargo test --workspace --offline`: **120 tests pass** (119 core, one desktop IPC), including full-size capacity and expanded schema-6 migration qualification. Seven new entries comprise six clear tests (including the subprocess-only entry) and one worker test.
- The final revision-emission change also passes `cargo test -p gitjuke-core --offline --lib clear_partial_failure`; historical-schema reconstruction corrections pass the store integration suite.
- `npm run check`: Pass; TypeScript/frontend production build and workspace Clippy with warnings denied. Final Rust edits also pass offline workspace Clippy.
- `cargo fmt --all -- --check`: Pass.
- `npm run tauri build -- --debug --bundles app --target aarch64-apple-darwin`: Pass; **31.07 MiB** Apple Silicon development `.app`, build only.
- Bundled executable SHA-256: `a43d49d54112d6afd1f51af67c2b1896f913e55eb14655d31d43a1a39ec33ae5`. All three clear fault-switch strings and the migration phase switch are absent from the executable.
- Acceptance matrix remains exactly 29 grouped rows, all Pending.

The unchanged-row fixture initially used `:memory:`, which cannot satisfy Store's filesystem space admission; it now uses a temporary disk-backed database. Historical schema fixtures were updated to remove the newly added table. These are fixture corrections, not relaxed production requirements.

No live GitHub read, native app launch, GitHub content/credential change, permission/login change, signing, publication or distribution occurred.
