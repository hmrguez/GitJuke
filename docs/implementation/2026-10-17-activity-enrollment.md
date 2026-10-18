# Bounded Activity target enrollment checkpoint — 2026-10-17

Status: production enrollment and recovery-readiness slice implemented. Development version **0.1.0**, schema **4**. No Git repository or applicable AGENTS.md exists in the workspace or ancestors. Settled contracts are unchanged.

## Audit and implementation

The remaining storage audit found that `activity_targets` inserted every missing non-selected watch into the hydration target table in one autocommit statement. Startup/recovery cleanup was bounded, but ordinary scheduling could immediately create an unbounded enrollment write. LRU/broad invalidation/purge, recovery completion, query/temp/full-size capacity, remaining lifetimes/compaction, physical faults, migration peak allocation and startup recovery UX remain separate gaps.

`activity_enrollment.rs` now enrolls at most **128 new targets per transaction and scheduling pass**. A space check precedes each pass. It selects missing repository/PR locators in deterministic repository/number order, deduplicating membership reasons and duplicate historical stable IDs at the same locator. Selected-only PRs remain excluded, as before. Existing targets retain due/completion state, including targets first discovered through notifications. Watches are neither capped nor removed; later passes enroll the remainder without a cursor or a Rust victim/target collection. The existing 128-descriptor hydration scheduling bound remains unchanged.

Recovery completion now independently rejects any non-selected watch without an enrolled target. This prevents partial enrollment from falsely establishing a historical baseline or completing catch-up when every *currently enrolled* target happens to be done. Enrolled unfinished targets continue to block completion through the existing readiness check. Notification/source/check completeness, Pause, manual-read behavior, boundary and summary filtering remain authoritative. Internal enrollment does not bump the presentation revision, matching prior behavior; published Activity/progress still uses the existing transactional revision path.

Each enrollment transaction is atomic. Earlier batches survive later faults or interruption; the uncommitted batch rolls back. Existing runtime error handling stops ingestion after enrollment storage failure, preserving validated cached access and Pause. No new migration, production fault switch, delivery state or native alert implementation is introduced.

## Evidence

All inputs are synthetic. SQLite, worker execution, filesystem persistence and killed subprocesses are real. No live GitHub read or native launch occurs.

| Subcase | State | Evidence |
| --- | --- | --- |
| Mutation bound and eventual enrollment | Pass, real SQLite | 257 watches, each with three reasons, plus a duplicate historical locator and a selected-only locator. Passes insert exactly 128/128/1/0 rows; each successful transaction's SQLite total-changes delta equals its returned count. Every eligible locator enrolls once; selected-only remains absent. No watch cap or dropped membership. |
| Existing target and history preservation | Pass, real SQLite/reopen | Existing completed target retains its future due time. Identity, recovery boundary, baseline/catch-up progress, unseen/dedup state, attempted/unavailable outcomes, membership/attempt history, published content and notification validator/deadline survive successful passes, SQL failure, process death and restart. |
| Recovery readiness during partial enrollment | Pass, real Store | Completing all currently enrolled targets after each full batch still leaves recovery incomplete, with no new summary. After enrollment finishes, one unfinished target independently blocks completion. Completing that target permits normal mode; repeated completion adds no intent. An expired catch-up record does not manufacture a summary. |
| Later-batch rollback | Pass, SQLite trigger | First 128 targets commit; insertion of target 200 aborts the second batch, including its preceding inserts. Reopening while faulted preserves exactly the earlier 128. Removing the fault finishes the remaining 128/1, without resetting original targets/history. |
| Interrupted later batch | Pass, **two actual subprocess SIGKILL cases** | Child enters production Store/enrollment after the first 128 commit. Kill before second commit retains 128; kill immediately after commit retains 256. Parent verifies SIGKILL/reaps and inspects raw SQLite before startup cleanup: integrity passes, all enrolled new targets are unfinished/due, and no partial batch survives. Production reopening finishes all 257 and preserves durable history. |
| Worker protection and paused repair | Pass, Runtime/SQLite with synthetic gh subprocess fixture | Establish a baseline through the actual worker, then Pause. Under a future remote deadline, manual scheduling commits the first batch and faults the second. Storage protection blocks further refresh and starts no new child reads. Removing the fault permits normal storage repair, retaining identity, Pause, boundary, notification deadline and ledger counts. Repair does not enroll through Pause; subsequent explicit passes finish enrollment under the same remote deadline without reads or new intent history. |

The two crash barriers and environment switches compile only in tests. No production fixture fallback is present.

## Limits and acceptance

The 128-row bound covers **watch-derived Activity target enrollment transactions**, not the entire scheduling pass, hydration publication, recovery summary mutation count or every target writer. Notification enrollment remains bounded by fetched pages and explicit single-PR enrollment remains a separate path. Source/watch discovery publication, LRU admission, broad invalidation/Clear/purge and recovery completion still need broader mutation qualification. SQL scanning, DISTINCT/sorting/index/page work, execution duration, temporary space, full 500-MiB capacity and whole-database size are not bounded by this change. No VACUUM/file-shrink guarantee exists.

Physical exhaustion/readonly/IOERR, power loss, transient/temp accounting, sufficient peak migration allocation and missing/legacy/corrupt/unreadable startup UX remain Pending. Native delivery is absent; existing attempted/unavailable intents stay terminal. Unattempted membership reconstruction, reservation/revalidation, pre-dispatch markers, ambiguous dispatch and click routing remain unimplemented/unverified. No dispatch readiness or native/live capacity is inferred.

All **29 grouped release scenarios remain Pending**; existing Blocked positive live fixtures remain Blocked. This checkpoint adds only the listed enrollment/readiness/fault Pass subcases.

## Verification

- `cargo test -p gitjuke-core --offline enrollment`: Pass; five new entries, including the subprocess-only child entry, the two real SIGKILL cases and runtime protection/repair.
- `cargo test --workspace --offline`: **102 tests pass** (101 core, one desktop IPC). No test failures.
- `npm run check`: Pass; TypeScript/frontend production build and workspace Clippy with warnings denied.
- `cargo fmt --all -- --check`: Pass.
- `npm run tauri build -- --debug --bundles app --target aarch64-apple-darwin`: Pass; **31.04 MiB** local Apple Silicon development `.app`, build only.
- Bundled executable SHA-256: `ef376503d6d7b034014a058919f684513d782b7d52f7a857b1a5fd036ae4a1ef`. All three enrollment test environment-switch strings are absent from this executable.

No native app launch, live GitHub read, credential/content/permission/login change, signing, publication or distribution occurred.
