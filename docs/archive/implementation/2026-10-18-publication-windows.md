# Bounded publication planning and completion queries — 2026-10-18

Status: **Pass, scoped implementation and verification**. Development **0.1.0**, SQLite schema **14**. This continues the [maintained accounting/indexed LRU checkpoint](2026-10-18-maintained-admission.md). No applicable AGENTS.md was found. Settled product contracts and release limits remain unchanged.

## Planning decision and implementation

The prior twelve-row application batches did not bound preparation: filtering, unions, joins, equality tests and already-prepared exclusions could traverse or sort arbitrarily large sets before producing a batch. Completion also counted joined contexts and evaluated broad readiness/summary relations. Adding a final LIMIT alone would leave those rejected prefixes unbounded.

`publication_planning.rs` replaces production preparation SQL with native candidate windows. Each table pass reads at most **12 candidates**, ordered by native rowid, within a fixed upper fence. Scope, eligibility, duplicate-journal and equality tests occur afterward, using indexed point reads. Request-array passes take at most 12 inputs by position; singleton plans use fixed point operations. Rejected and unchanged candidates still advance. Cursor, fence and prepared rows commit together; an empty final window advances the step. Signed rowid endpoints are supported. Invalid cursors, steps and phases fail closed and retain the original work.

The publication request still fixes accepted values and input references. Old canonical payload images remain separately charged; operational staging images are not duplicated. Existing images survive retries and migration. Request consumption, application, admission, old-image invalidation and cleanup keep their prior accounting and visibility boundaries. No preparation cursor changes presentation revision or certifies recovery.

Source reconciliation, selection, team pruning, heads, complete/partial checks, Activity and recovery summaries use the new drivers. Watch flags accumulate through three bounded passes: reset existing flags in the journal, include unchanged canonical reasons, then include prepared replacement reasons. These passes may retain zero-flag no-op rows. The fenced journal scan includes all prior membership plans without chasing its own newly inserted flag rows. The previous unrestricted SQL remains **test-only**, as an independent oracle for twelve publication variants.

Small-publication preflight uses saturated indexed counts. Source/reason, selected-location, team/pending-prefix and staged-check indexes ensure scope predicates do not hide a growing unrelated prefix. Selection unions have separately bounded inputs; team preflight counts the entire candidate prefix before obsolete-team filtering. Staging reset/removal uses indexed scope windows or exact incoming keys rather than a filtered unbounded input relation.

## Completion and mutation accounting

`publication_completion.rs` evaluates recovery readiness over **128-row native windows** in one SQLite read snapshot, with indexed point checks. It includes enrollment completeness, unfinished targets, pending discovery/check work, source restart guards, authored head/check health and required source health. It publishes no partial readiness cursor. Killed or failed reads cannot mark recovery normal or create a summary.

Catch-up key discovery scans bounded windows and retains at most 55 keys for small-path preflight. The small transaction re-reads at most 54 keys and checks Activity/seen membership by exact keys; larger summaries use durable publication planning. Source staging totals and staged/incoming check unions use indexed 128-key windows. Published check completion reads the maintained snapshot count from the LRU frontier rather than traversing all contexts. Case-insensitive prefix cleanup and exact native membership semantics are preserved.

Schema 14 adds a published-context count to the **existing frontier row**. Its insertion/deletion/staging/key-move triggers maintain count and first key together. This adds no separate counter-row mutation; actual trigger writes still count. The shared timestamp/content-before-check/native-key eviction order, missing-state timestamp zero and zero-byte victim behavior remain unchanged.

Every covered publication/maintenance transaction retains the **128-total-direct-mutation cap**, including triggers, admission, dependent updates, journal rows, cursor, revision and final markers. Preparing twelve charged old payload images costs at most 24 row mutations plus one cursor mutation before admission; existing guards measure actual changes and roll back an over-budget transaction. Application stays at twelve rows; cleanup retains its conservative trigger-aware allowance. The planning-to-application transition is a guarded metadata transaction. Atomic visibility still changes only once, after application; cached readers use protected old images until that commit.

The inclusive shared disposable ceiling remains **524,288,000 bytes**, including canonical bodies, staging/request inputs and retained old payloads. Durable Activity/seen/dedup/progress/baseline/recovery history and terminal attempted/unavailable intent, membership and attempt records remain protected. Pause, identity, validators/deadlines, denial/clear barriers and cached navigation keep their contracts.

## Migration and restart

Schema 13→14 installs indexes/cursors and backfills frontier counts in the existing FULL-synchronous migration transaction, with step/commit space checks. A failure leaves schema 13 intact. Older in-progress **planning** resets to step zero while retaining its prepared rows and protected images; duplicate keys make replay safe. Applying/committed journals retain their phase and application state. Startup/repair complete unfinished publication before transient abandonment or remote eligibility. Older binaries reject schema 14.

The rowid cursor/fence is operational state, not a domain identity. Pending publication keeps the existing serialized writer contract; no concurrent table rebuild/VACUUM is introduced. Standalone completion windows own a read transaction; callers already in a transaction use that snapshot where supported.

## Scoped evidence

[Evidence directory](evidence/2026-10-18-publication-windows/). **86 distinct passing test entries**, across separate final invocations; this is not a full workspace run:

| Subcase | State | Evidence |
| --- | --- | --- |
| Planning, completion and migration | **Pass: 13 entries** | [Query tests](evidence/2026-10-18-publication-windows/query-tests.log). Independent unrestricted preparation oracle across twelve variants; 257/1,025-identity unchanged, rejected, prepared and protected prefixes; scope-index preflight; staged/source/published count oracles; signed rowid endpoints and invalid cursor/phase protection. Captured statements return ≤128 rows, have ≤128 scan steps, <20,000 VM steps and zero sort/automatic-index steps. Maximum observed VM count is **1,416**, zero full-scan steps in these fixtures. Window counts grow with cardinality; operation duration is not qualified. |
| Snapshot/failure/retry | **Pass, scoped** | Same query log: concurrent WAL writer after the first readiness window cannot change that read snapshot; the next readiness read sees the blocker. Late preparation failure rolls back twelve additional retained images, byte charges and cursor together. Old cached content/history remain intact; retry/reopen publishes the accepted content once. |
| New process kills | **Pass: 10 actual SIGKILL cases** | DELETE/WAL × before/after late cursor commit (**4**); schema-13 migration before/after commit (**4**); late readiness read (**2**). Raw inspection checks exact journal/image/cursor/accounting prefixes, old cached visibility, migration version and durable completion/intent/attempt/deadline state. Restart finishes exactly once. Two inert child entry tests are included in the 13 entries above. |
| Publication visibility kills | **Pass: 10 actual SIGKILL cases** | [Publication kills](evidence/2026-10-18-publication-windows/publication-kills.log), one parent entry: DELETE/WAL application before/after commit, visibility before/after commit and committed cleanup. Independent totals/frontier counts, raw committed prefixes, old/new reader visibility and history survive restart. |
| Focused publication/admission/Activity | **Pass: 39 entries** | [Focused regressions](evidence/2026-10-18-publication-windows/focused.log), excluding actual/full-ceiling fixtures, broad kill tests and child entries. Includes exact maintained totals/LRU, late rollback/retry, small/journal snapshot equivalence, pending recovery barriers and schema-9 pending publication upgrade. |
| Worker protection/repair | **Pass: 4 entries** | [Worker regressions](evidence/2026-10-18-publication-windows/workers.log): interrupted publication/Activity/summary, query cursor and publication-failure repair, paused and before remote eligibility. Synthetic gh only. |
| Reader/history protection | **Pass: 11 entries** | [Reader regressions](evidence/2026-10-18-publication-windows/readers.log): Activity recovery/summary/seen/history/expiry/rollback, cached expired checks, SQLite FULL/read-only filtering and large applied-journal prefix selection. |
| Integration | **Pass: 18 entries** | [Integration regressions](evidence/2026-10-18-publication-windows/integration.log): Activity 3, store 8, watch-store 7, including selected legacy upgrades. |
| Formatting and lint | **Pass** | [Formatting](evidence/2026-10-18-publication-windows/formatting.log): `cargo fmt --all -- --check`; [Clippy](evidence/2026-10-18-publication-windows/clippy.log): `cargo clippy --workspace --all-targets --offline -- -D warnings`. |

Preliminary oracle failures exposed parameter binding in the test oracle and JSON array representation for selection progress; both were corrected before the final twelve-variant comparison. A new edge fixture initially named nonexistent flag columns and was corrected. Initial lint findings were fixed manually; sandbox restrictions prevented Clippy's optional fix listener. No mutation guard, timeout or existing correctness assertion was relaxed.

No full workspace suite, 500-MiB fixture, broad migration/kill matrix, development app build or app launch was performed. No credentials, live GitHub probes, native delivery, signing, publishing or distribution were enabled.

## Retained applicability and open gates

The preceding schema-13 checkpoint retains **117 distinct scoped entries / 67 actual SIGKILL cases**, and the admission-window checkpoint retains **56 entries / six kills** and its unsigned development build. Those are historical results for their recorded versions, not additional schema-14 runs. The earlier clean **196-test** workspace run and schema-12 **228 SIGKILL / 180 injected-space-failure** migration matrix retain their original scope. The older **166-case** publication qualification remains scoped/retained, explicitly not a clean monolithic run. Selected legacy upgrades here do not qualify every supported migration path. Previous development artifacts do not contain schema 14.

**Pending:** cumulative planning/readiness/admission duration; broader reader/queue/enrollment/sweep queries; variable request/body/key/metadata bytes and whole-request parsing/search/serialization/`json_set` work; total allocator/temp bounds; all-version schema-14 migration and index-build peak space; physical faults/exhaustion/IOERR/power loss; compaction and terminal metadata lifetimes; recovery UX for missing/legacy/corrupt/unreadable stores; native delivery, binary replacement and physical-target responsiveness/performance. Functional test durations do not qualify native performance. Per-query candidate bounds do not bound total traversal work or B-tree/variable-key comparison cost.

Existing live-fixture gaps remain **Blocked**. All **29 grouped release scenarios remain Pending**.
