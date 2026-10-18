# Bounded startup reclamation checkpoint — 2026-10-17

Status: production storage prerequisite implemented through startup, bounded SQLite cleanup, actual process-kill fixtures and worker protection/repair. Development version **0.1.0**, schema **4**. No Git repository or applicable AGENTS.md was found. The settled specification is unchanged.

## Audit and implementation

`Store::open` previously deleted every abandoned traversal and reset every session flag in one transaction. A large interrupted traversal could therefore produce an unbounded cleanup write before the worker started. Cleanup now uses restartable transactions with a **global maximum of 128 direct row mutations per transaction**, shared across all cleanup statements. Matching unchanged rows do not consume the allowance or keep cleanup running.

The covered classes are legacy/source/team/Activity staging; staged check contexts; obsolete discovery/check/legacy/Activity pending jobs; selected-session watch flags and reasons; manual/foreground flags on surviving pending jobs; and the single discovery deadline reset. Published checks/content, authored/direct/team watch identities and reasons, stored selection, settings/Pause/identity, reconciliation health, Activity, seen/deduplication, progress/baseline/catch-up membership, terminal intents/membership, attempt history and notification validators/deadlines are preserved by these transactions.

Startup checks available space before each batch and repeats until no cleanup mutations remain. No worker or remote read starts while startup cleanup is incomplete. Committed batches remain committed after a later failure or interruption; the failing/uncommitted batch rolls back. Every operation is idempotent, so reopening completes remaining work without a cleanup cursor or schema migration. Clearing selected-only session state never deletes the remembered selection or other watch reasons. Surviving previous-session jobs lose their manual/foreground flags, preserving persistent Pause.

This supersedes the historical storage-availability checkpoint's **single-transaction startup cleanup implementation**. The change affects abandoned disposable/session work only. **Schema upgrades still commit all migration steps in one transaction**, and ingestion/progress/intent atomicity is unchanged. A partially cleaned database is never published as ready for monitoring. Current-schema cleanup failure still opens the protected read-only runtime; identity locking remains mandatory. Repair must finish cleanup, recovery writes, retention, committed health probing and checkpointing before leaving protection. Pause and interrupted recovery boundaries survive.

The bound covers direct row mutations in these cleanup transactions. It does **not** bound index/page writes, SQLite scans/sorting/temp space, total startup duration, migration work or subsequent recovery-state updates. `activity_recovery` target/intents updates, LRU admission, other broad invalidations/purges and full-size storage/time/space qualification remain separate work. There is no VACUUM, file-shrink or whole-database-size guarantee.

## Evidence

All evidence is local and deterministic. SQLite is real; the worker uses the existing synthetic gh subprocess fixture. Crash tests launch the Rust test executable through the real `Store::open` path, stop at test-only barriers and send actual **SIGKILL**, then reap the child. Barriers are compiled only under `cfg(test)`; no production fault switch is added. No live GitHub read, native app launch, permission or dispatch occurred.

| Subcase | State | Evidence |
| --- | --- | --- |
| Global 128-row mutation bound | Pass, real SQLite | 257 rows in each transient class plus all four stale pending-job prefixes span more than 20 transactions. Each batch's returned mutation count equals SQLite's `total_changes` delta and is at most 128. Published and durable state is checked after every batch and reopening. Empty cleanup terminates without extra mutations. |
| Complete class cleanup and session reset | Pass, real SQLite | All staging and stale jobs disappear; published check row, authored watch flags/reason and durable records remain. Surviving job flags reset, discovery becomes due, and notification deadline remains unchanged. |
| Late-batch failure and restart | Pass, SQLite trigger | First 128 deletions commit. A source-stage fault in the next transaction rolls back its companion final legacy-stage deletion. Reopening while faulted retains that row; removing the fault permits cleanup with original durable state. |
| Kill during an uncommitted batch | Pass, actual subprocess SIGKILL | Child dies after executing 128 deletions but before commit. All 257 original stage rows remain after reopening SQLite; integrity check passes. Normal startup then cleans them and preserves identity/Pause, published checks, unseen/dedup/progress, terminal outcomes/membership and attempts. |
| Kill between committed batches | Pass, actual subprocess SIGKILL | Child dies immediately after first batch commit. Exactly 129 of 257 stage rows remain. SQLite integrity passes; normal startup finishes cleanup while preserving the same published/durable invariants. |
| Worker protected startup and repair | Pass, actual worker/process/SQLite with synthetic gh | Later-batch fault starts a protected, identity-locked, paused runtime after the first 128 deletions. Explicit remote refresh is rejected; another failed repair performs no child reads. Removing the fault completes repair without automatic reads, retaining Pause, notification deadline, recovery boundary and durable ledger counts. Explicit Refresh validates the original identity and exposes the cached list without resuming or creating pending native delivery. |
| Existing migration/protection/retention behavior | Pass, deterministic suite | Prior schema upgrade rollback, future/corrupt preservation, first-batch startup failure, storage-protection and retention tests pass unchanged. |
| Broader crash, physical and performance qualification | Pending | The two startup cleanup boundaries do not qualify migration, ingestion/progress/intent/attempt/dispatch kills, power loss, physical exhaustion/readonly/IOERR, peak migration/temp space or full 500-MiB time/capacity. Total startup/query duration and recovery-state/LRU mutation bounds remain unqualified. |
| Native alerts and release gates | Pending / existing Blocked fixtures | Native delivery and actual native schema-4 migration remain Pending. Previously Blocked positive live fixtures remain Blocked; all **29 grouped release scenarios remain Pending**. |

## Verification

- `cargo test --workspace --offline`: **84 tests pass** (83 core and one desktop IPC). Five additional test entries comprise four behavioral tests and the subprocess-only child entry. Targeted startup cleanup and worker tests also pass.
- `npm run check`: Pass; TypeScript, frontend production build and workspace Clippy with warnings denied.
- `cargo fmt --all -- --check`: Pass.
- `npm run tauri build -- --debug --bundles app --target aarch64-apple-darwin`: Pass; **31.02 MiB** local Apple Silicon development `.app`, build only.
- Bundled development executable SHA-256: `dd8bde115c13c56ad7971a383df933b39b5ff5d848e6b807d9a0b29a3b660bda`.

The initial targeted compile caught an unsupported `usize` SQLite test-row conversion; it was corrected to an integer conversion before all passing checks. No failed behavior is accepted as a specification exception. No credentials, GitHub content, signing, login, notification permission, publication or distribution settings changed.
