# Migration storage admission and crash durability checkpoint — 2026-10-17

Status: production migration guards and their process-crash/late-failure qualification are implemented. Development version **0.1.0**, schema **4**. No Git repository or applicable AGENTS.md was found in the workspace or ancestors. Settled contracts are unchanged.

## Audit and implementation

Remaining storage gaps include LRU admission and broad invalidation/purge mutation bounds, recovery-completion mutations, source/metadata lifetimes and compaction, full-size 500-MiB/query/temp/physical-I/O qualification, peak migration space and startup recovery UX. The previous slice qualified Activity transactions under SIGKILL; migration interruption still had only ordinary SQL-error evidence. `Store::open` checked free space before opening the database and configured full SQLite synchronization only after the upgrade transaction.

`migrations.rs` now owns the existing schema 0/1/2/3→4 SQL, unchanged, and its single transaction. The writable connection explicitly uses **synchronous FULL before migration writes**, for either an existing rollback journal or WAL. Future-version and integrity rejection still precede migration configuration; existing future/corrupt-file preservation tests remain in force. Journal-mode conversion to WAL still follows the complete migration commit. Current-schema writable connections also retain FULL synchronization.

The production free-space checker runs before **every required schema step** and **immediately before commit**, in addition to the existing opening check. Any failed check returns a storage error and rolls back all schema steps, data backfills and `user_version` changes together. No intermediate version commits, cleanup cursor, new schema or destructive fallback is added. Startup cleanup and worker creation still wait for a complete upgrade. This supersedes the historical single free-space admission check for a multi-step upgrade.

The unchanged floor is **1 GiB available**. These repeated checks do not estimate peak migration allocation, reserve space, interrupt an individual SQL statement when free space changes or guarantee enough room for arbitrary database growth. Peak migration/temp/rollback/WAL space qualification remains required; the floor is not presented as a complete migration-space preflight.

## Evidence

All new data is synthetic; SQLite, filesystem sidecars and process death are real. Fixtures reconstruct schemas from the supported historical migration SQL, not from old signed application binaries. The successful reference upgrade uses an exact copy of each closed fixture so timestamps and all input rows match. No live GitHub read or native app launch occurs.

| Subcase | State | Evidence |
| --- | --- | --- |
| Late storage admission | Pass, 28 injected space failures / real SQLite | Schemas 0, 1, 2 and 3 in both DELETE-journal and WAL modes fail each required-step admission and final commit admission in turn. Complete schema SQL, all tables/rows (including sequence state) and `user_version` match the original snapshot after every failure. A later normal `Store::open` completes the upgrade. The injected checker models a space failure; it does not exhaust the host volume. |
| Durability configured before writes | Pass, actual connection state | Admission tests start with synchronous OFF and verify FULL after rejection. Every child migration barrier checks synchronous=2, including the barrier before any schema write. No unsafe mode is introduced in production. |
| Kill at migration boundaries | Pass, **44 actual subprocess SIGKILL cases** | For each supported input schema and both journal modes: kill before steps, after every required schema step, immediately before commit and immediately after commit. The parent verifies SIGKILL, reaps the child and inspects SQLite before normal startup cleanup. All pre-commit deaths recover exactly the previous schema and all original rows. Post-commit deaths expose exactly the complete schema-4 reference, including all backfills and durable state; no intermediate schema survives. Integrity checks pass throughout. |
| Disk-backed migration writes | Pass, filesystem observation | Child-only cache size is reduced to two pages. Every pre-commit case verifies a rollback journal larger than its header or WAL larger than its header before sending SIGKILL, so the matrix includes writes that reached sidecar files. This is not a power-loss/fsync or peak-space measurement. |
| Restart continuation | Pass, production Store/SQLite readers | Every killed case reopens through `Store::open`. Original identity, Pause, custom appearance, configuration, revision/generation, unseen/dedup/attempt state, remembered selection and authored/direct watch reasons survive. Migration 2 rebuilds locators/reasons from legacy rows. Migration 4 backfills all 257 check states from fetched timestamps, with NULL→0. Published check payloads remain. Startup removes abandoned staging/selected-session state after upgrade, as before; cached lists remain readable through Store. Fresh databases have no fabricated identity or historical baseline. |
| Schema-3 terminal history | Pass, checked after crash/reopen | Existing Activity, baseline/catch-up progress, attempted/unavailable outcomes, member identities, original recovery boundary, attempt history and notification validator/deadline survive. No pending delivery backlog is introduced. |
| Failed upgrade → runtime stop → repair | Pass, real Runtime/SQLite with synthetic executable | A schema-3 backfill trigger aborts migration; `Runtime::start` returns an error, the entire original schema/data snapshot remains and no reader executable runs. Removing the fault permits upgrade and paused, identity-locked startup. The fixture executable remains uninvoked; cached list exposure is rejected until original-identity validation. The ordinary recovery revision increment is distinguished from migration preservation. |
| Existing failure and protection paths | Pass, deterministic workspace suite | Existing schema-1 multi-step SQL rollback, schema-3 backfill failure, future/corrupt preservation, current-schema protected startup, storage repair and Activity crash tests remain in the workspace suite. |

There are **22 kills per journal mode** (7/6/5/4 for schemas 0/1/2/3) and **14 space failures per mode** (5/4/3/2). Test barriers, cache-size overrides, synthetic executables and injected checkers are test-only. Production has no migration fault switch or reduced cache policy.

## Limits and acceptance

The guarded upgrade remains one atomic transaction. This slice does not bound migration row mutations, wall time, scans/sorting/index work, total database size or peak migration/WAL/journal/temp bytes. Checks between steps cannot prevent a mid-statement FULL/IOERR; physical exhaustion/readonly/IOERR and power-loss qualification remain Pending. No VACUUM, file shrink, backup/export or reset flow was added.

A failed legacy-schema migration still stops startup with originals preserved. The intact-current-schema protected cache cannot read the legacy schema; missing/legacy/corrupt/unreadable startup recovery UX remains Pending. The repaired runtime test does not establish native schema-4 migration, identity validation through real gh, compatible/incompatible signed binary pairs, replacement/rollback or release signing.

Native delivery remains absent; existing terminal intent history stays terminal. No reservation/revalidation, unattempted-membership reconstruction, pre-dispatch marker, ambiguous OS dispatch or click-routing pass is inferred. Existing positive live fixture gaps remain Blocked. All **29 grouped release scenarios remain Pending**; only the listed migration subcases gain Pass evidence.

## Verification

- `cargo test -p gitjuke-core --offline --lib migrations`: Pass; 44 killed children, 28 late-space failures, runtime stop/repair and the subprocess-only child entry.
- `cargo test --workspace --offline`: **97 tests pass** (96 core, one desktop IPC), including four new migration entries: two store qualification tests, the runtime test and the subprocess-only entry.
- `npm run check`: Pass; TypeScript/frontend production build and workspace Clippy with warnings denied.
- `cargo fmt --all -- --check`: Pass.
- `npm run tauri build -- --debug --bundles app --target aarch64-apple-darwin`: Pass; **31.02 MiB** local Apple Silicon development `.app`, build only.
- Bundled executable SHA-256: `da5da5c3d296ef0a800150548ee873a53c3a55a17a02b66e1adc71cdc920b6c0`. Migration fault-switch strings are absent from this executable.

Initial test assertions incorrectly expected the migration-only revision after runtime recovery had legitimately incremented it, and independently seeded reference databases could differ at a fetch-timestamp second boundary. The fixtures now compare exact closed inputs and account for the separate recovery revision. No production behavior failure was accepted as a contract exception.

No GitHub content, credentials, permission, login, signing, publication or distribution settings changed.
