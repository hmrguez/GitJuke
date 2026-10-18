# Bounded shared LRU admission — 2026-10-17

Status: production shared-admission implementation, targeted mutation/fault/restart and worker qualification complete. Development version **0.1.0**, schema **8** unchanged. No Git repository or applicable AGENTS.md exists in the workspace or ancestors. Settled contracts are unchanged.

## Audit and implementation

The audit found eight payload-writing paths calling a shared eviction function that could delete an arbitrarily long LRU prefix inside their publication transaction. Age cleanup also had separate content/check allowances, with check-completeness updates outside the counted deletions. Breaking those transactions into partial publications would expose incomplete source membership, new heads or Activity/progress/intent state.

The implemented sequence was: introduce a shared transactional publication wrapper; cap all admission mutations under one allowance; roll back candidates requiring additional space and reclaim only previously committed disposable payloads in bounded transactions; replay the SQL publication; qualify late failures and process death at the actual 500-MiB ceiling; update acceptance evidence.

All eight writers now use `content_budget.rs`:

- Detail/API `Store::put`.
- PR source staging and completed source publication.
- Selected-PR registration and watched-head publication.
- Staged/completed check-context pages.
- Activity staging and completed Activity/progress/eligibility-intent publication.

The wrapper executes the writer's SQL in a candidate transaction and runs bounded admission before commit. If the candidate fits after that admission, it commits normally. Otherwise **the whole candidate rolls back**, including its admission deletions, before independent reclamation starts. The wrapper then retries the writer in a new transaction. Caller closures keep every effect in SQL, reconstruct attempt-local state, and read their inputs from arguments or non-evictable staging; no gh request, UI event, OS delivery or external side effect occurs inside a retried closure. Activity's provisional intent IDs and SQLite sequence changes roll back with each candidate. Existing publication crash hooks remain after successful admission and before commit; post-commit hooks remain outside the wrapper.

## Bounds, ordering and atomicity

The admission contribution to any candidate transaction is at most **128 direct row mutations**, shared across opportunistic expiration, ordinary content deletion, published-check deletion and check-completeness invalidation. The publisher's own mutations and revision remain separate: this is **not a 128-mutation bound on the entire publication transaction**.

Each independent reclamation transaction also has a **global 128-mutation bound**, this time including its presentation revision update. It reserves one row for that revision, leaving at most 127 for payload deletion and necessary check-state invalidation together. Already-normalized check states consume no update allowance. Removing a published context and marking its snapshot incomplete happen atomically; neither can commit alone. Every committed independent batch increments the revision so readers can invalidate payload/completeness views even if the subsequent publication fails.

The shared budget remains **524,288,000 bytes, inclusive**, counting UTF-8/BLOB bytes in ordinary content, legacy/source/Activity staging, and staged/published check contexts. Staging is never an eviction victim. A candidate whose non-evictable staged bytes alone exceed the ceiling fails before any independent reclamation, without committing pointless cache loss or partial progress. Otherwise expiration and pressure share the existing deterministic `(accessed, kind, key/PR, commit, context)` ordering. Pressure stops after the smallest sufficient prefix, including zero-byte entries that precede a required victim. A check update and deletion need two available mutation slots when its state requires invalidation; the batch ends rather than skipping that older victim.

SQLite selects at most 128 victim identities and byte lengths into Rust, drops the cursor, then applies mutations. Payloads are not deserialized or collected. This replaces the previous single-boundary/unbounded-prefix deletion with bounded metadata collection and bounded per-attempt SQL mutation work. Query scan/sort/temp allocation and total synchronous operation duration remain unqualified.

Space admission precedes every candidate attempt and independent batch. The runtime continues to serialize the operation. A later reclamation fault rolls back its whole batch while earlier reclamation remains committed. A later publication fault leaves its new membership/head/content/Activity/seen/dedup/progress/intents uncommitted; prior disposable reclamation may remain visible. Small admissions still reclaim inside publication and retain whole-operation rollback behavior, as the existing selected/head/check fault tests verify. A stale optional head publication returns without admission side effects.

Independent reclamation preserves watch identities/reasons/epochs, source/Activity staging, identity/settings/Pause, minimal Activity and unseen state, seen/dedup/progress/baseline/recovery boundary, terminal attempted/unavailable intents, membership/attempt history, notification validators and server deadlines. Published check completeness is disposable metadata and becomes incomplete when its payload is evicted. No generation increment or new admission marker is needed: every committed batch is a valid ordinary cache state; pending publication has rolled back. Restart follows existing abandonment/reconciliation policy rather than completing a half-published transaction. Schema 8 and migration behavior remain unchanged.

## Evidence

All inputs are deterministic. SQLite, filesystem persistence, Rust worker/process execution and SIGKILL are real. Worker responses use the existing synthetic gh executable. No live GitHub read or native app launch occurred. Local environment is arm64 macOS **27.0 (26A428)**. This neither qualifies physical M1/8-GiB/macOS-15 performance nor expands the release matrix.

| Subcase | State | Evidence |
| --- | --- | --- |
| Global independent-batch bound | Pass, real SQLite | 513 mixed content/distinct-check payloads plus 257 contexts sharing an already-normalized state require more than seven batches. Every successful batch's SQLite total-changes delta equals its returned count and is at most 128, including revision. Durable values are compared after each batch; complete check states never outlive their last evicted context. |
| One speculative admission allowance | Pass, real SQLite | 513 expired mixed content/check rows with distinct states exercise expiration plus pressure in one candidate. Combined admission total-changes is at most 128. Rollback restores all 513 payloads and completeness. Existing UTF-8/inclusive-ceiling/tied-LRU/zero-byte-prefix tests pass. |
| Impossible staging | Pass, real SQLite | Non-evictable staging larger than the scaled ceiling rolls back its payload/progress/revision without committing any old-cache deletion. All 513 old payloads survive. Existing actual-ceiling Activity staging refusal/fault coverage is retained. |
| Later reclamation failure | Pass, SQLite trigger/raw restart | A row-200 deletion fault retains earlier bounded committed reclamation, rolls back the entire later candidate/batch, leaves new content and progress absent, and preserves all seeded durable values. Check state/payload consistency survives raw reopen. |
| Publication failure after reclamation | Pass, SQLite trigger/restart | A progress-insert fault after prior check reclamation rejects the next publication without removing any further committed payloads or advancing progress. Removing the fault and reopening permits successful publication under the same budget. |
| Candidate replay and terminal history | Pass, real SQLite/restart | At least three candidate executions commit exactly one new Activity, unseen, dedup, progress, terminal unavailable intent and membership set. Provisional intent IDs/sequences roll back: committed new intent is ID 3 beside original attempted/unavailable IDs 1/2. Terminal attempted history stays unchanged. This tests SQL replay atomicity, not native dispatch. |
| Actual-ceiling many-victim admission | Pass, **524,288,000-byte fixture** | 513 mixed 1-KiB payloads plus future-dated bulk filler start exactly at the production ceiling. A real `Store::put` of 384 KiB requires several independently committed reclamation batches, then one atomic publication; 129 old small payloads remain. Durable values survive restart; the new content is readable and shared bytes remain at/below the ceiling. Filler and un-watched check metadata do not establish actual watch capacity. |
| Process interruption | Pass, **five actual SIGKILL cases at the production ceiling** | Kill after candidate rollback; before/after first independent reclaim commit; before/after final publication commit. Raw inspection precedes normal startup. Old small payload counts are **513/513/428/214/129**. New content is absent except after final publication commit; revisions change only for committed reclamation/publication. Integrity checks pass, durable values match and check completeness is consistent. Production restart and a fresh missing publication succeed without terminal-history changes. |
| Worker protection/repair | Pass, actual Runtime/process/SQLite with synthetic gh at the production ceiling | 513 two-byte payloads plus bulk filler force many-victim admission from normal-sized API responses. A later deletion fault preserves the first committed reclamation and rejects selected-PR registration: no new locator/list/overview or membership commits. Protection retains durable counts, exposes a newer revision and rejects Refresh. Fault removal and repair behind a controlled remote deadline start no extra children, preserve identity/Pause/poll deadline and allow a fresh read afterward. Restart remains paused/locked without automatic reads. |
| Existing publication/crash/migration behavior | Pass, full workspace suite | Full-size small-victim rollback, selected/head/check rollback, Activity staging/publication/progress/intent/summary kills, bounded first-page/purge/clear/invalidation/recovery, migration and process/IPC checks remain in the workspace suite. All pass; final result is recorded below. |

All new fault hooks/environment switches compile only in tests. Process death and SQL abort do not establish physical filesystem exhaustion/readonly/IOERR or power-loss durability.

## Remaining gaps and acceptance

Admission mutations and independent reclamation transactions now have the bounds above. **Source/watch/check/Activity publication's own mutations, team/Activity/check first-page initialization and recovery-completion mutation bounds remain separate gaps.** The wrapper can replay a large publication, so this does not bound its cumulative write work, time, temporary storage or memory. Broader SQLite cardinality/query/sort/temp/time qualification, metadata lifetimes/terminal compaction, startup recovery UX for missing/legacy/corrupt/unreadable databases, physical storage/power-loss faults, sufficient peak migration allocation and transient/temp accounting remain Pending. No VACUUM/file-shrink/whole-database-size guarantee exists.

Native delivery, reservation/revalidation, unattempted-intent recovery, pre-dispatch attempts, ambiguous dispatch and click routing remain unimplemented/unverified. Existing attempted/unavailable history stays terminal. All **29 grouped release scenarios remain Pending**; existing Blocked live fixtures remain Blocked. No dispatch readiness or specification change follows from this slice.

## Verification

- Targeted shared-admission, full-ceiling kill and worker tests: Pass.
- `npm run check`: Pass; TypeScript/frontend production build and workspace Clippy with warnings denied.
- `cargo fmt --all -- --check`: Pass.
- `npm run tauri build -- --debug --bundles app --target aarch64-apple-darwin`: Pass; **31.21 MiB** Apple Silicon development `.app`, build only.
- Bundled executable SHA-256: `5f5807f29cc7c4ecbfec637dd05d7948be66183fc78d32128af8d6ac37222b57`.

- `cargo test --workspace --offline`: **154 tests pass** (153 core, one desktop IPC). Ten new entries comprise nine admission entries (including the subprocess-only child) and one full-ceiling worker test. The suite includes all five new actual-ceiling SIGKILL cases, retained full-size capacity/fault tests, existing Activity/publication/recovery crash qualification, 120 migration SIGKILL cases and 88 injected migration space failures.
- All three new admission fault-switch strings, the source-initialization and PR-purge phase switches, and the migration phase switch are absent from the bundled executable.
- Acceptance matrix remains exactly 29 grouped rows, all Pending.

The integration-test binary took 119.99 seconds in this run; its 1,001-request partition case completed successfully after a long-run notice. Process inspection confirmed it had completed while that notice was investigated. No test timeout/expectation was relaxed and no performance result is claimed. Controlled query/cardinality/time/native responsiveness qualification remains Pending.

The first worker fixture faulted after successful selected registration, during the following overview write. Registration and overview remain separate existing transactions. The fault now targets an earlier victim after a committed reclamation prefix and verifies registration rollback explicitly; no production boundary or assertion was weakened.

No GitHub content/credentials, native notification permission, login, signing, publication or distribution changed.
