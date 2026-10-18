# Published check reclamation checkpoint — 2026-10-17

Status: production storage slice implemented through publication, cached reads, periodic worker maintenance and protective repair. Development version **0.1.0**, schema **4**. The settled specification is unchanged. No Git repository or applicable AGENTS.md was found.

## Behavior

Published check payloads previously consumed the shared 500-MiB budget without participating in age/LRU reclamation. Check reads also had no 30-day visibility filter. Schema 4 adds `check_state.accessed`, initially copied from the last successful `fetched` timestamp (or zero when absent). All schema steps, including this initialization, commit together before scheduling. Existing schema 1/2/3 upgrades remain transactional; current-schema protected startup now requires schema 4. A legacy database that cannot migrate still stops with its original schema preserved; broader startup recovery UX remains unfinished.

Successful complete check publication records refresh/access time. Reading the current head/merge check page updates nonexpired snapshots' access time without changing their successful fetch timestamps or revision. Expired snapshots cannot revive through an access. Protected reads perform no writes; the first denied access-time write enters storage protection and retries the cached read. PR-summary payloads on the check response also enforce their existing 30-day lifetime.

Shared admission now streams a single LRU order across ordinary content rows and published check-context rows. Order is access time, payload class (ordinary content first on a tie), then stable content key or PR/commit/context keys. Rust retains one boundary tuple. Byte accounting still includes all staging classes; staging is never an eviction victim. A staging-only excess fails admission and rolls back publication. The inclusive production ceiling remains **500 × 1024 × 1024 bytes**.

Before any check rows are deleted, their snapshot is marked incomplete with `Cached checks evicted`, in the same transaction. Retained rows can still display as partial cached checks; deletion never creates a complete successful empty result. Watch membership, current heads, source generations, minimal Activity, unseen values, deduplication, baseline, intent membership and terminal unavailable/attempted history survive. Old commit payloads and orphan published contexts are reclaimable too; staged contexts remain protected until the existing traversal lifecycle removes them.

Publication and periodic maintenance each expire at most **128 published check-context rows** per transaction, in addition to the previous Activity/content limits. At most 128 associated check-state rows are invalidated. Exact 30-day age remains inclusive. Maintenance runs on the existing 60-second deadline during persistent Pause. Read-time filtering immediately hides expired checks before physical cleanup, including on a query-only connection. Expired or incomplete checks cannot supply terminal Activity classification or satisfy authored-check recovery readiness.

Maintenance failure rolls back the entire retention transaction, including companion content deletion and completeness invalidation, then enters the existing storage-protection path. Repair requires successful cleanup, committed health probe and checkpoint. Pause and the no-backlog-replay boundary survive; repair does not start remote children while paused.

## Evidence

All new evidence uses real SQLite and deterministic fixtures. Worker tests launch the actual bounded process runner against synthetic gh responses. No live GitHub read, native app launch, permission prompt or native delivery occurred.

| Subcase | State | Evidence |
| --- | --- | --- |
| Mixed shared LRU and inclusive limit | Pass, deterministic SQLite/reopen | Reduced private byte ceiling retains an exact-fit mixed cache, then removes the oldest content and one check row in stable key order. Staging survives admission; snapshot completeness becomes false. Watch reasons and durable identity/outcome history survive reopening. Production limit is unchanged. |
| LRU failure atomicity | Pass, SQLite trigger | Check deletion failure rolls back a new payload, revision and snapshot invalidation; old content and both check rows remain. |
| Bounded check age cleanup | Pass, SQLite/reopen | 129 contexts survive the exact 30-day boundary, then expire in 128/1 batches across a restart. Delete failure rolls back completeness and revision before successful retry. |
| Protected read expiry and access semantics | Pass, actual read-only SQLite | Access advances access time while preserving successful fetch time. After expiry, check-page payloads and terminal Activity classification are empty, completeness is false and the stored payload/access value remains untouched. The pre-expiry workflow fixture produces one terminal Activity candidate. |
| Schema 3-to-4 success/failure | Pass, SQLite trigger | Initialization failure rolls back the added column and schema version, retaining prior fetched data. Retry initializes access from fetched and preserves identity/Pause. Existing schema-1 upgrade and multi-version rollback tests pass. |
| Worker maintenance failure/repair | Pass, actual worker/process/SQLite with synthetic gh | Paused maintenance encounters a check deletion fault and enters protection; companion expired content and original complete state remain. Repair removes contexts and invalidates completeness, preserving Pause, child-read count and absence of pending native intents. |
| First check access write failure | Pass, actual query-only runtime connection | A check read is the first operation to discover denied writes; it succeeds through the protected retry. Existing navigation, explicit-refresh rejection, repair and no-replay assertions remain passing. |
| Full-size capacity, physical faults and migration peak space | Pending | Reduced-limit algorithm tests do not qualify 500-MiB time/memory/temp behavior. No physical filesystem exhaustion/read-only/IOERR, process kill at commit boundaries or peak migration measurement was performed. |
| Broader reclamation and durable growth | Pending | LRU mutation count, SQLite query/sort/temp space and execution time remain unbounded/unqualified. Startup cleanup, obsolete metadata/source lifetimes, terminal-intent compaction and transient-file accounting still need work. No VACUUM, total-database-size or file-shrink guarantee is introduced. |
| Native alerts and positive live fixtures | Pending / previously Blocked | Native onboarding/categories/permission/reservation/revalidation/dispatch/click routing and unattempted-intent recovery remain absent. Terminal unavailable/attempted history stays terminal. No dispatch readiness is inferred. Previously Blocked positive live fixtures remain Blocked. |

All **29 grouped release scenarios remain Pending**. Historical native/live evidence remains historical, with schema-4 native migration still requiring actual app execution. No credentials, GitHub content, login, permission, signing, publication or distribution settings changed.

## Verification

- `cargo test --workspace --offline`: **75 tests pass** (74 core and one desktop IPC), including six new tests. The final strengthening of the expired-workflow assertion was checked separately with `cargo test -p gitjuke-core --offline --lib protected_check_reads`.
- `npm run build`: TypeScript and frontend production assets pass.
- `cargo clippy --workspace --all-targets --offline -- -D warnings`: Pass.
- `cargo fmt --all -- --check`: Pass.
- `npm run tauri build -- --debug --bundles app --target aarch64-apple-darwin`: Pass; **31.01 MiB** local development `.app`, build only.
- Bundled development executable SHA-256: `c46ae66c1ebd4ef4f247cc5d5ab67413ebdcd20c344e59cd3cf4d749ef979106`.

The initial verification found an overbroad SQL edit adding a nonexistent resource-state column; it was corrected before the passing runs. Clippy also caught a boolean-assert style issue. Strengthening the expiry test exposed snake-case fixture fields where the serialized model uses camelCase; correcting the fixture established a positive pre-expiry classification assertion. Both were corrected. No failure was accepted as a release exception.
