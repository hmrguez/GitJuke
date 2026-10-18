# Retention maintenance checkpoint — 2026-10-17

Status: a concrete retention slice is implemented through ingestion, SQLite, protected-cache reads and periodic worker maintenance. Production implementation remains authorized. Schema remains **3**; the settled product contract is unchanged. No Git repository or applicable AGENTS.md was found.

## Audit and behavior

The audit found that 90-day Activity cleanup ran only on successful publication and deleted the entire expired set in that ingestion transaction. Paused/offline sessions could retain expired rows indefinitely. Activity pages filtered the 90-day minimal-entry lifetime but joined payloads without enforcing the separate 30-day content lifetime. Re-reading expired source IDs could also cache payloads without an associated retained Activity entry.

The serialized worker now performs one retention transaction at its existing 60-second storage-maintenance deadline, including while Pause is persistent and when no remote job is eligible. Storage must first pass the free-space check. Each transaction removes at most **128 expired Activity entries**, their corresponding payloads and obsolete catch-up membership, plus at most **128 additional expired/orphan content rows**. The Activity candidate query uses the existing occurrence index and retains no more than 128 IDs in Rust. Successful ingestion uses the same bounded Activity cleanup helper. Remaining candidates wait for a later maintenance pass; read-time filtering immediately enforces visibility even before physical row deletion.

Payload deletion, expired minimal-entry removal, catch-up membership removal and the visible revision update commit atomically. Cleanup does not mark anything seen. Identity-only seen/deduplication records, intent membership, unavailable/attempted outcomes, attempt history and the baseline survive. Empty retention passes do not bump the revision themselves; the existing committed health probe still does. SQLite can reuse the released pages. No VACUUM, schema migration or automatic reset was added.

Activity pages now omit expired payload data without a write, including on a query-only connection, while keeping the retained minimal entry and its original unseen state. Old source pages still establish durable deduplication but cannot create expired Activity/payloads/intents. An already-expired source ID with a changed timestamp cannot resurrect an entry because its deduplication barrier survives. Ingestion only caches an Activity payload when its retained minimal entry exists.

A maintenance write failure rolls back its transaction and enters the existing storage-protection path. Remote reads remain stopped. Repair requires cleanup success, the actual committed probe and a complete checkpoint. Repair preserves Pause and does not create pending native delivery or restart remote children while paused.

## Evidence

All new evidence is deterministic, using real SQLite. The worker test launches the actual bounded process runner with the test-only synthetic gh executable for initial reconciliation. It then counts child reads while paused, during failure and after repair. Tests advance only the internal maintenance deadline; production remains at 60 seconds. No live GitHub reads or native app launch was performed in this slice.

| Subcase | State | Evidence |
| --- | --- | --- |
| Bounded expired Activity cleanup and restart | Pass, deterministic SQLite | 257 expired entries are removed in 128/128/1 batches, with a database reopen between passes. Payloads/catch-up membership disappear together. Empty follow-up does not change the retention revision. |
| Durable identity/outcome preservation | Pass, deterministic SQLite | Mixed seen/unseen values, all dedup IDs, both terminal intent states and their member identities, explicit attempt history and baseline survive every batch/reopen. |
| Exact retention boundaries | Pass, deterministic SQLite | Entries/payloads exactly at 90/30 days survive; one second later they are removed. Identity ledgers survive. |
| Separate content batch and old orphan cleanup | Pass, deterministic SQLite | 129 expired content rows leave one after the first pass; the next pass removes it and a recently fetched orphan Activity payload. |
| Transaction failure | Pass, synthetic SQLite trigger | Failure at minimal-entry deletion rolls back prior payload/catch-up deletion and the revision. Removing the fault permits cleanup. |
| Protected-cache lifetime and source replay | Pass, actual query-only SQLite | Expired Activity is absent; retained unseen entry loses only its expired payload. Rehydrating an expired ID with a new timestamp and a previously unknown old ID creates no new Activity/payload/intent. |
| Paused worker maintenance and protective repair | Pass, real worker/process/SQLite with synthetic gh | Deadline-driven cleanup happens while paused without additional child reads. Delete fault enters protection; explicit Refresh is rejected. Fault removal allows cleanup/probe/checkpoint repair, preserving Pause and the child-read count. No pending intent is created. |
| Complete storage bounds/reclamation qualification | Pending | Mutation counts are bounded, not execution time or SQLite temporary space. Content cleanup can scan; existing shared-budget eviction/startup cleanup remain separate paths requiring broader bounds. Freed pages are reusable, but no file-shrink or total-database bound is claimed. |
| Terminal-intent compaction and temporary accounting | Pending | Existing terminal records already hold identity/outcome metadata and remain intact. Further compaction, all obsolete check/source payload classes and transient/temp/peak-migration accounting still need qualification. |
| Physical faults and process-kill boundaries | Pending | No physically exhausted/read-only filesystem, IOERR injection or process kill at commit/dispatch boundaries was exercised. Trigger rollback/reopen evidence does not substitute for those cases. |
| Startup recovery UI and native dispatch | Pending | Missing/legacy/corrupt/unreadable databases still need recovery UX. Native onboarding/categories/permission, reservation/revalidation, unattempted-intent recovery, ambiguous delivery and click routing remain unimplemented/unverified. |

All **29 grouped release scenarios remain Pending**. Previously Blocked live fixtures remain Blocked. This slice does not establish native dispatch readiness. No permission, login, credentials, signing, publication or distribution settings changed.

## Verification

- `cargo test --workspace --offline`: **62 tests pass** (61 core and one desktop IPC test), including six new retention tests.
- `npm run build`: TypeScript and frontend production build pass, including the final development-app build's frontend step.
- `cargo clippy --workspace --all-targets --offline -- -D warnings`: Pass.
- `cargo fmt --all -- --check`: Pass.
- `npm run tauri build -- --debug --bundles app --target aarch64-apple-darwin`: Pass; local Apple Silicon development `.app`, 30.99 MiB. Build evidence only; it was not launched.
- Final bundled development executable SHA-256: `761a8773b3a6c45e379336a5070359105f8ab0f0b7f897d6314be4d9493fc570`.

No signed release, native delivery, live capacity, physical performance or complete storage qualification pass is inferred from these checks. The next storage work remains recovery UX for startup failures, physical IO/space and process-kill fixtures, migration peak space, remaining payload classes/compaction and temporary-file accounting. Native delivery must then implement reservation/revalidation and recoverable unattempted membership together, while preserving existing terminal history.
