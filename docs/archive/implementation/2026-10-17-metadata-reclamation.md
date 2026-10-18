# Disposable metadata reclamation checkpoint — 2026-10-17

Status: a production storage prerequisite slice implemented through bounded retention, SQLite transactions, worker protection/repair and explicit source rehydration. Development version **0.1.0**, schema **4**. No Git repository or applicable AGENTS.md was found. The settled specification is unchanged.

## Audit and behavior

Payload expiry and LRU reclamation previously left exact-URL validators and API cache-health rows indefinitely. Notification window URLs vary across traversals, so this metadata could accumulate even after its source payloads disappeared. Published check reclamation also left empty obsolete check-state rows, including previously successful empty snapshots.

The existing serialized 60-second retention transaction now reclaims three additional disposable metadata classes, with an independent **128-row maximum per class per pass**:

- Exact-URL validators without a corresponding physical `api:` payload, only when their stored server deadline has elapsed. Future deadlines survive eviction. A retained payload keeps its validator, including while waiting for a later payload-expiration batch.
- `api:` resource-state rows without either their payload or validator. Authored/direct/team reconciliation health and other resource-state classes are untouched.
- Check-state rows older than the inclusive 30-day lifetime, with no published or staged context, no pending traversal for that PR/commit, and no active watch whose head or merge commit matches. Current watched health, successful recent empty snapshots and unfinished traversals survive. Empty obsolete state may disappear in the same transaction that expires its last published context.

These run after existing bounded Activity/content/check-payload cleanup. All deletions and the retention revision commit together. Any failure rolls back companion payload deletion, metadata and revision and enters worker storage protection. Repair still requires successful maintenance, committed health probe and checkpoint. Persistent Pause and notification deadlines survive; repair starts no remote child while paused.

Subsequent explicit source hydration reads unconditionally when its validator has been reclaimed, then rebuilds cache/validator state through the existing gh transport. Deduplication prevents repeated Activity. No cleanup modifies durable watch identities/reasons, seen/deduplication, successful progress/baseline, due deadlines, terminal intent membership/outcomes or attempt history. Native delivery remains unavailable; existing unavailable/attempted records stay terminal.

No migration, VACUUM, reset, hard watch cap or file-shrink guarantee is introduced. This bounds additional row mutations and Rust-side identity storage; **SQLite scan/sort/temp space and execution time remain unqualified**. LRU and startup cleanup still need broader mutation bounds. This slice does not finish all source/metadata lifetimes or terminal-intent compaction.

## Evidence

All new evidence uses real SQLite and deterministic fixtures. The worker test runs actual owned gh subprocesses against the existing synthetic executable. Internal maintenance deadlines advance in tests; production timing remains 60 seconds. No live GitHub read, native app launch, notification permission, delivery, credential or remote-content change occurred.

| Subcase | State | Evidence |
| --- | --- | --- |
| Bounded metadata reclamation and restart | Pass, deterministic SQLite | 257 rows of each metadata class drain in 128/128/1 passes with a database reopen. Source reconciliation health and durable history survive. Empty maintenance does not bump revision. |
| Server deadline and payload ownership | Pass, deterministic SQLite | Validator with a retained exact payload survives; missing-payload validator remains until its future deadline, then becomes reclaimable at the deadline. API health follows validator removal. Durable notification due remains unchanged. |
| Check lifetime and traversal protection | Pass, deterministic SQLite | Exact-age successful empty state survives, then expires one second later. Current watched head/merge health remains explicitly incomplete after expiry; staged and queued obsolete contexts retain state. Published expired payload and empty obsolete state are removed together. |
| Atomic failure in every metadata class | Pass, SQLite trigger faults | Validator, API-health and check-state delete failures each roll back expired Activity/payload/catch-up and metadata deletion plus revision. Removing each fault permits cleanup; seen/dedup/terminal membership and server due survive. |
| Paused worker protection, repair and explicit rehydration | Pass, actual worker/process/SQLite with synthetic gh | Check-state cleanup failure rolls back companion source-cache/validator/health removal. Protection rejects remote refresh. Repair preserves Pause and polling deadline with no additional children. Explicit hydration then sends exactly one unconditional request for the reclaimed comments page, rebuilds its ETag, and leaves Activity count and absence of pending delivery unchanged. |
| Existing published-check cleanup fault fixture | Pass, updated expectation | The expired orphan's context and now-empty obsolete state disappear after repair. Earlier checkpoint's completeness-invalidation evidence still applies before state reclamation; retained current snapshots keep explicit health. |
| Full-size capacity, physical faults and broad reclamation | Pending | Full 500-MiB capacity/time/temp, physical exhaustion/readonly/IOERR, process-kill commit boundaries, peak migration space, startup/LRU mutation bounds, remaining source lifetimes and terminal-intent compaction remain required. |
| Native delivery, live positives and release qualification | Pending / existing Blocked fixtures | No native onboarding/categories/permission/reservation/revalidation/dispatch/click implementation or new native/live qualification. Existing Blocked positive fixtures and all **29 Pending grouped release scenarios** remain unchanged. Native schema-4 migration remains Pending. |

## Verification

- `cargo test --workspace --offline`: **79 tests pass** (78 core and one desktop IPC), including four new tests. The new retention tests and worker cleanup/rehydration test also passed targeted runs before the full suite.
- `npm run check`: Pass; TypeScript, frontend production assets and workspace Clippy with warnings denied.
- `cargo fmt --all -- --check`: Pass.
- `npm run tauri build -- --debug --bundles app --target aarch64-apple-darwin`: Pass; **31.01 MiB** local Apple Silicon development `.app`, build only.
- Bundled development executable SHA-256: `b957b4db7d70c9db3fed431c5e478309ada84b2a3029a1c3753c1ba86f385a19`.

No signing, provisioning, publication or distribution was performed. Native delivery and physical/reference-machine performance readiness are not inferred from these tests or the development build.
