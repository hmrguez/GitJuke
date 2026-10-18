# Recovery and storage protection checkpoint — 2026-10-17

Status: prerequisite hardening implemented; native notification delivery remains unavailable. Production implementation is authorized under the settled specification. This slice changes no product decisions, GitHub content, authentication, permissions or release settings. No Git repository or applicable AGENTS.md was found in this workspace or its ancestors.

## Implemented behavior

The audit found that SQLite failures were surfaced as “monitoring stopped” while the worker could continue remote reads. Worker ingestion, queue acknowledgements, scheduling, failure/backoff maintenance and the covered local write commands now propagate storage failures into a runtime protection state. It cancels the current work, prevents subsequent automatic and explicit child reads, and preserves cached reading and local preferences where SQLite remains usable. Status exposes `storageStopped`, and its health message remains protective even when Pause or a theme change updates the ordinary runtime message.

Available-space checks precede scheduling writes and individual API reads. Idle serialized maintenance runs at one-minute intervals. A stopped runtime waits one minute before retrying a free-space check, an actual committed revision write and a PASSIVE WAL checkpoint. Failed probes keep it stopped. Successful repair restarts reconciliation, preserves notification server deadlines and Pause, and never directly dispatches or marks Activity seen. A PASSIVE checkpoint's result rows are checked: SQL success with uncheckpointed WAL frames is treated conservatively as an incomplete checkpoint. Releasing a blocking reader allows a later health probe to succeed. No VACUUM or destructive database reset was added.

A transport failure now distinguishes an ordinary outage from an interrupted recovery. An ordinary outage captures its catch-up boundary when recovery starts, including activity during that outage. An interrupted recovery retains its original boundary across repeated failures and database reopening. This prevents post-boundary activity from being incorrectly reclassified as missed activity. Successful notification/discovery/source reconciliation is still required before completion.

Recovery summary membership is inserted by SQLite in the same transaction as the summary and progress transition. Rust no longer collects the entire recovery membership into a `Vec`. A single cross-PR summary retains eligible, unseen, unexpired members; expired/seen members are excluded. Existing terminal `unavailable` and `attempted` intents remain terminal across recovery and reopening. No new pending delivery path was introduced: future enabling still cannot replay this slice's unavailable backlog.

The GraphQL identity path now propagates a failed identity-storage update rather than treating that write error as an identity mismatch.

## Evidence

All new evidence is deterministic. The worker test launches the actual bounded subprocess runner against the existing test gh executable and uses real SQLite connections/triggers and a held WAL reader. There is no production fixture fallback. Tests advance only the internal storage-probe deadline to avoid one-minute sleeps; production timing remains sixty seconds.

| Subcase | State | Evidence |
| --- | --- | --- |
| Worker write failure stops remote processes | Pass, synthetic | Queue-insert fault; unchanged child-read count during protection, including an explicit refresh attempt. |
| Cached reading and local preferences during protection | Pass, synthetic | Cached authored page remains available; Pause and theme persist while the queue-write fault remains active. |
| Repair requires committed write success | Pass, synthetic | Metadata-write fault rejects the probe; no further children. Removing it permits repair/reconciliation without resuming Pause. |
| Checkpoint result validation and repair | Pass, synthetic | A held SQLite read snapshot leaves WAL frames uncheckpointed; store and worker enter protection. Releasing it allows checkpoint/repair without data loss. |
| Local settings write failure | Pass, synthetic | Settings-insert trigger makes theme saving fail and immediately sets storage protection. |
| Ordinary outage versus interrupted recovery | Pass, synthetic | Ordinary recovery takes its restoration boundary; repeated recovery outages and reopening preserve an existing boundary. Terminal unavailable/attempted intent states survive. |
| Bounded recovery summary grouping | Pass, synthetic | 1,001 candidates produce one 999-member summary after excluding one seen and one expired record. Repeated completion cannot duplicate the summary. Membership streams through SQLite. |
| Existing ingestion atomicity, deadlines, Pause and watch rotation | Pass, synthetic | Existing rollback, conditional hydration, notification polling, identity, 151-watch rotation and overflow coverage retained. |
| Live completed recovery and native dispatch | Pending | No new authenticated GitHub or native UI execution in this slice. Historical evidence remains in the prior checkpoint. |
| Full disk/read-only/IOERR and kill-at-every-commit matrix | Pending | Triggered SQLite errors and a real partial checkpoint establish the listed paths only. Startup under exhausted storage, settings/cache behavior when all writes fail, all migration/retention/compaction boundaries and storage growth reporting still need work/qualification. |

Native onboarding, permission/category controls, attempt reservation/revalidation, ambiguous dispatch crash semantics and click routing remain unimplemented. Broader storage/fault qualification and recovery compatibility are still open. This prerequisite slice does not establish dispatch readiness. All 29 grouped release scenarios remain Pending; positive live fixtures previously marked Blocked remain Blocked. The exact signed artifact, physical performance, accessibility and advertised OS matrix remain unqualified.

## Verification

- `cargo test --workspace --offline`: **49 tests pass** (48 core, one desktop IPC test).
- `npm run check`: TypeScript/frontend production build and workspace Clippy pass with warnings denied.
- `cargo fmt --all -- --check`: Pass.
- `npm run tauri build -- --debug --bundles app --target aarch64-apple-darwin`: Pass; local arm64 development `.app`, 30.94 MiB. It was not launched during this slice, so this is build evidence only.
- Final development executable SHA-256: `8fa73a5acbd3cda31ae7715622bb017f9180203c31b2a59e88f8e770b26a4934`.

No notification prompt, login registration, Developer ID signing, publication or distribution was performed. No additional native/live compatibility or release pass is inferred from these checks.
