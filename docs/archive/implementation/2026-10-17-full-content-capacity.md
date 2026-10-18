# Full-size shared content-budget checkpoint — 2026-10-17

Status: full-size functional capacity and fault qualification implemented. Development version **0.1.0**, schema **4**. No Git repository or applicable AGENTS.md exists in the workspace or ancestors. Settled contracts are unchanged.

## Audit and scope

The previous admission/reclamation tests used reduced private limits. They established algorithm behavior but left real **500 × 1024 × 1024 = 524,288,000 payload bytes** unqualified. Other outstanding work includes LRU/broad invalidation/Clear/purge and recovery-completion mutation bounds, query/sort/temp execution, remaining source/metadata lifetimes and terminal compaction, physical exhaustion/readonly/IOERR, power loss, peak migration allocation and missing/legacy/corrupt/unreadable startup UX.

This vertical slice exercises the existing production ceiling and writers at full size, through admission, LRU invalidation/publication, restart, injected failure, process death and worker protection/repair. The existing budget algorithm required no behavior change. `content_capacity.rs` is test-only; the two new barriers in `Store::put` are also compiled only under `cfg(test)`. Production synchronization, cache policy, payload/IPC limits, scheduling and free-space floor are unchanged. No fixture fallback or runtime fault switch is added.

## Fixtures and method

The mixed fixture contains 499 one-MiB UTF-8 JSON strings, one half-MiB string and 128 four-KiB strings: exactly 500 MiB. Allocation spans ordinary content, legacy staging, source staging, staged checks, published checks and Activity staging. Multibyte strings distinguish UTF-8 byte accounting from character accounting. Each seed transaction holds only a bounded payload string and inserts rows into real SQLite; it does not use sparse/zero-filled placeholder files or a lowered admission limit. A checkpoint plus page-count/page-size and filesystem-length assertions establish a database at least 500 MiB on disk. Full SQLite synchronization remains enabled.

These are direct SQLite capacity fixtures, not claims that synthetic filler is valid remote source schema, successfully hydrated Activity, a 50-watch workload, or realistic worst-case row cardinality. A production `Store::put` then serializes a real 384-KiB replacement/new value under the existing one-MiB writer ceiling. A separate real `activity_stage` call tests ingestion when all budget is non-evictable staging.

The crash fixtures convert staging filler to ordinary published content before closing, preserving exactly 500 MiB through normal startup cleanup. Each child enters the actual Store/put path. Only the child test connection uses an eight-page cache; observing WAL bytes beyond its header before SIGKILL establishes that writes have reached a sidecar before the pre-commit kill. Parents verify SIGKILL/reap, inspect raw SQLite before startup cleanup, then reopen through production Store and retry publication.

The full-size fixtures are serialized within the test process to avoid overlapping their disk allocation. The run uses local arm64 macOS **27.0 (26A428)**. This is not the required physical M1/8-GiB/macOS-15 native performance environment or an expansion of the advertised release matrix.

## Evidence

| Subcase | State | Evidence |
| --- | --- | --- |
| Exact full-size inclusive ceiling | Pass, real disk-backed SQLite | At exactly 524,288,000 payload bytes, production admission makes zero row mutations, preserves check completeness and agrees with Preferences disposable-byte reporting. All six payload classes contribute to the same total. |
| Mixed full-size LRU publication | Pass, production Store writer/SQLite | A 384-KiB publication removes 64 older four-KiB content rows and the first 32 four-KiB check rows in stable key order. Remaining checks become explicitly incomplete with `Cached checks evicted`; staging and bulk payloads remain. Total payload bytes remain exactly 500 MiB. The real Store reader returns the complete new value. |
| Restart after successful full-size admission | Pass, production Store | Reopening preserves published payloads and the new value, incomplete check state, identity/Pause, unseen/dedup/progress, terminal attempted/unavailable history, membership/attempt ledgers, recovery boundary and notification deadlines. Normal startup removes abandoned staging; the post-startup total is compared with the exact pre-close published-byte total, not incorrectly expected to retain old staging. |
| Late LRU failure and retry | Pass, SQLite trigger/real Store writer | A content-delete trigger aborts after successful new payload/resource writes, check invalidation and 32 check deletions. SQLite total-changes confirms those preceding mutations executed. Entire publication rolls back: original 500-MiB total, 64/64 old small rows, complete snapshot, revision and lack of new resource remain. Removing the fault permits the normal exact-budget publication. |
| Non-evictable full staging and one-byte excess | Pass, real SQLite and Activity writer | Four staging classes each hold 125 MiB. An extra byte fails production admission; rollback restores exactly 500 MiB. A real Activity staging call also fails with the explicit budget limitation, leaving no new staged source/progress/Activity/intent/revision. No incomplete traversal is discarded to manufacture room. Normal restart discards abandoned staging and preserves durable history. |
| Pre/post-commit full-size process death | Pass, two actual SIGKILL cases | Before commit: original 500-MiB content, complete snapshot, resource absence and old revision survive together. After commit: exact LRU survivors, incomplete snapshot, complete new resource/payload and incremented revision survive together. WAL is nonempty beyond its header before each kill; integrity checks pass. Reopening and repeated real publication remain within the production ceiling and preserve terminal/unseen history. |
| Full-size worker failure/protection/repair | Pass, actual Runtime/process/SQLite with synthetic gh | At the full ceiling, manual selection fails during published-check eviction. Original payload bytes, 64 checks and completeness survive; no new locator or publication revision commits (the earlier remembered-selection revision is retained). Storage protection rejects Refresh. A controlled remote deadline holds the interrupted authorized manual job while repair removes abandoned Activity staging and preserves Pause, identity, interrupted recovery boundary, notification deadline and Activity/replay/progress ledgers. No extra child reads start during that isolated repair. Releasing the fixture deadline and explicitly selecting again publishes successfully, within budget and without pending native delivery. |

## Limits and acceptance

This qualifies **functional capacity/admission and the listed fault boundaries at the actual 500-MiB payload limit**. It does not establish worst-case cardinality, upper bounds on query/sort/temporary space, Rust/SQLite/process memory, elapsed time, LRU mutation count, ingestion throughput or native responsiveness. No time/memory/temp threshold is inferred from successful completion. The database and WAL may exceed the payload ceiling; no whole-database cap, VACUUM/file-shrink or peak-allocation guarantee is added.

Disk exhaustion, read-only filesystems, IOERR and power loss remain Pending. An injected SQL abort and actual process death are distinct from those physical faults. This slice does not implement transient/temp accounting, migration-space estimation, broader mutation bounds, terminal compaction or startup recovery UX.

Native delivery remains absent. Terminal attempted/unavailable records stay terminal; unattempted-intent reconstruction, reservation/revalidation, pre-dispatch attempts, ambiguous OS dispatch and click routing remain unimplemented/unverified. Existing Blocked positive live fixtures remain Blocked. All **29 grouped release scenarios remain Pending**. No live/native or dispatch-readiness claim follows from these storage tests.

## Verification

- `cargo test --workspace --offline`: **108 tests pass** (107 core, one desktop IPC). Six new test entries comprise four full-size store/fault behaviors, the subprocess-only child entry and the full-size worker test. Both new SIGKILL cases run in that suite.
- `cargo test -p gitjuke-core --offline --lib full_ceiling_worker`: Pass after fixture corrections; final workspace run also passes all full-size cases.
- `npm run check`: Pass; TypeScript/frontend production build and workspace Clippy with warnings denied.
- `cargo fmt --all -- --check`: Pass.
- `npm run tauri build -- --debug --bundles app --target aarch64-apple-darwin`: Pass; **31.04 MiB** local Apple Silicon development `.app`, build only.
- Bundled executable SHA-256: `d9183edfc4c5d146b1ab0bf9460831ac3055fc7dc89daacfe73ecee98a736ca8`. All three capacity-test environment-switch strings are absent from this executable.
- Acceptance matrix inspection confirms exactly 29 grouped rows, all Pending.

No native app launch, live GitHub read, credential/content/permission/login change, signing, publication or distribution occurred.

Initial worker-fixture assertions conflated the separately committed remembered selection with failed publication and treated a completed normal cycle as an interrupted recovery. The fixture now accounts for the selection revision/progress, deliberately seeds an incomplete boundary and holds outstanding authorized manual work behind a controlled deadline during repair. Test-only helper visibility/import errors were corrected during compilation. No production behavior failure was accepted or contract changed.
