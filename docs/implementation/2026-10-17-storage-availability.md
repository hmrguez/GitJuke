# Storage availability checkpoint — 2026-10-17

Status: a concrete storage-protection vertical slice is implemented through startup, worker, SQLite, cache navigation, recovery and typed presentation. Production implementation remains authorized; the settled specification is unchanged. Native delivery is still unavailable. No Git repository or applicable AGENTS.md exists in the workspace or its ancestors.

## Audit and selected slice

Inspection of `activity_store`, `runtime_activity`, the worker/storage path and the resolved runtime/notification contracts found two prerequisites that were still broken: low-space startup aborted before a runtime could expose protective status, and cache detail/list access attempted SQLite writes even when all writes were failing. Schema upgrades also committed version steps separately, so a later failure could leave an intermediate schema committed.

These are addressed below. The notification audit also confirms that `new_intent` still creates terminal `unavailable` entries. Existing unavailable/attempted history is preserved. `activity_recovery` has a provisional pending-to-recovery transition, but does not reconstruct unattempted intent membership into a deliverable recovery summary. There is no safe native delivery path yet: permission/category/enabling boundaries, transactional reservation, current identity/head/seen/expiry/category revalidation, unattempted-intent recovery, ambiguous attempted-delivery crash handling and click routing must be implemented and tested together before dispatch can be enabled. The current focused PR remains eligible; no focus-based suppression was added. This slice introduces no pending delivery state or backlog replay.

## Behavior

For an existing, intact **current-schema** database, a failed startup storage check or a failed startup cleanup write now opens a read-only cache connection and starts in protection. It exposes stored preferences, identity metadata, storage measurements and protective status. It defers cleanup, reconciliation and remote processes until storage recovery. GitHub content remains locked until the original identity is validated; exhausted storage cannot bypass that isolation. A persistent Pause also prevents automatic identity reads after repair. Explicit Refresh then validates the original account under the usual budget rules.

After a runtime has validated identity, protection permits cached authored/requested lists, PR detail, checks and Activity without needing access-time writes. The first cache read that encounters a failed access-time update switches into protection and retries the cache read without that write. Actual read failures still propagate. Expired list/detail payloads are filtered without requiring deletion; missing/expired list content produces incomplete state while watch membership survives. Cached selection during protection is in-memory only and schedules no remote read. Local settings/seen/clear changes still require a committed write; failures are reported and do not claim persistence or acknowledgement.

Explicit discovery/activity refresh rejects storage protection before queuing remote work. Protection cancels current work; the worker and per-response gate retain their existing remote-read stops. A repair from read-only startup requires successful writable reopening/transactional cleanup, recovery writes, a committed revision probe and a completed WAL checkpoint before replacing the protected connection. A failure keeps protection active. Production probes remain at one-minute intervals. Pause, existing polling deadlines and an interrupted recovery's original boundary survive repair. Ordinary restart still starts its recovery boundary at restart, as specified.

All schema steps now execute in one transaction: schema 1-to-3 failure rolls back the schema-2 steps as well as the schema-3 steps. Startup transient-state cleanup also commits as one transaction. Free-space checks precede creation/migration; no unbounded VACUUM, destructive reset or schema downgrade was added. Schema remains **3**.

Status/Preferences now report logical database, WAL, shared-memory and rollback-journal file sizes separately from disposable JSON/staging bytes and the 500-MiB content limit. The sidebar and Preferences warn when those measured files reach 1 GiB, and explain that Clear downloaded content preserves growing durable ledgers. Unrelated files are excluded. These are apparent file sizes, sampled without an atomic filesystem snapshot; **transient SQLite sort files are not measured**, explicitly stated in the UI. Complete peak/temp disk accounting and bounded reclamation remain Pending. No forced in-memory SQLite sort policy or whole-database size guarantee was introduced.

## Evidence and applicability

All new behavioral evidence is deterministic. Worker tests launch the actual bounded subprocess runner against the test gh executable and use real SQLite. No production fixture fallback exists. Low-space tests inject only the internal free-space checker; production uses `statvfs` and the existing 1-GiB threshold. Tests advance the probe deadline rather than sleeping one minute. This slice did not launch the native app or make live GitHub reads. Previous native/API evidence remains historical with its original applicability statements.

| Subcase | State | Evidence |
| --- | --- | --- |
| Existing-schema low-space restart and repair | Pass, synthetic space signal / real runtime and SQLite | No additional gh children or database-byte changes while protected; status/settings available, content identity-locked. Repair preserves Pause, server deadline and interrupted boundary. Explicit Refresh validates the identity without resuming Pause. |
| Startup cleanup failure and repair | Pass, synthetic SQLite trigger | Startup deletion fails, incomplete staging survives, read-only protected runtime starts. Failed repair stays stopped. Removing the fault permits transactional cleanup, probe, identity validation and monitoring. |
| Every connection write denied | Pass, real SQLite READONLY responses | `PRAGMA query_only` rejects all writes on the worker connection. First failed access-time update still returns cached detail; lists/checks/Activity and cached navigation work. Settings/Pause/seen/clear fail without revision change. Failed health probe starts no children; repair preserves Pause/deadline and creates no pending delivery. |
| SQLite FULL rollback | Pass, real SQLite FULL response / synthetic page ceiling | `max_page_count` rejects a 900-KiB cache insert. Revision and prior content/identity survive; failed content is absent. This does not fill the host volume. |
| Read-only expiration | Pass, deterministic | Expired detail is absent without deletion; expired list is incomplete and retains watch membership. |
| Multi-version migration rollback | Pass, deterministic SQLite failure | A schema-3 table collision after schema-2 steps leaves schema 1, original identity/Pause and no committed schema-2 table. Removing the collision permits complete schema-3 upgrade. |
| Measured-file warning boundary | Pass, filesystem fixture | Database/WAL/SHM/journal sizes included; unrelated file excluded. Sparse fixture checks exactly one byte below and at 1 GiB without allocating a GiB. Missing sidecar contributes zero. This is not native visual evidence. |
| Actual filesystem exhaustion, permission failure and IOERR | Pending | Injected space check, connection READONLY and page-ceiling FULL cover only the paths above; no physically exhausted/read-only volume or IOERR fixture was exercised. |
| Missing/legacy database with no space; corrupt/unreadable current cache | Pending availability UX | Read-only fallback requires an intact current schema. Missing data or required migration still stops startup with files preserved; no automatic reset or fresh baseline is fabricated. A dedicated startup recovery screen remains needed for these cases. |
| Kill-at-every-commit, peak migration space and reclamation | Pending | Migration failure rollback is covered; process-kill boundaries, true insufficient migration space, complete retention/compaction and transient/temp accounting remain open. |
| Native notification delivery and positive live fixtures | Pending / existing fixture gaps Blocked | No permission/category/dispatch/click implementation or additional native/live verification in this slice. Existing unavailable backlog remains terminal. |

All 29 grouped release scenarios remain **Pending**. The positive direct/team/mention/retry/50-watch live fixture gaps retain their prior Blocked/Pending status. No policy was weakened to call this slice complete. Native delivery prerequisites are not yet qualified.

## Verification

- `cargo test --workspace --offline`: **56 tests pass** (55 core, one desktop IPC test).
- `npm run check`: TypeScript/frontend production build and workspace Clippy pass with warnings denied.
- `cargo fmt --all -- --check`: Pass.
- `npm run tauri build -- --debug --bundles app --target aarch64-apple-darwin`: Pass, local arm64 development `.app`, 30.97 MiB. Build evidence only; it was not launched in this slice.
- Final development executable SHA-256: `b02ddeb99d11b73c5ca27dd20ce2a3f9db649a066323bdfb21bf1a96c4b493a7`.

No signing, login registration, notification permission, publication or distribution was performed. No live/native/performance/release pass is inferred from the deterministic tests or development build.
