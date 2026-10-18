# Shared content-budget admission checkpoint — 2026-10-17

Status: production storage prerequisite slice implemented through cache writers, SQLite transactions, the actual worker/process path and storage-protection repair. Development version remains **0.1.0**, schema **3**. The confirmed specification is unchanged. No Git repository or applicable AGENTS.md was found in the workspace or its ancestors.

## Audit and changes

The audit found two cache writers that bypassed the shared 500-MiB admission check: selected-PR registration and successful head snapshots both upserted `list:` payloads without eviction. Both now invoke the shared admission path before committing their metadata, membership, head generation, check invalidation and revision changes. Failure rolls back the entire operation; it cannot publish a successful closure/removal or a newer head with incomplete cache publication.

`content_budget.rs` now owns the common byte accounting used by admission and Preferences storage reporting. It counts UTF-8/BLOB bytes, including `content`, legacy staging, source staging, published and staged check contexts, and Activity staging. Production still uses the settled **500 × 1024 × 1024 bytes**, inclusive. No schema migration, settings change or hard watch cap was introduced.

The old eviction loop recomputed all five tables' byte totals after deleting each LRU row. The new path computes the shared total once, streams only access-time/key/byte-length metadata until the necessary LRU prefix is known, drops the cursor and deletes that prefix in one SQL statement. Ties use key order. Rust retains at most one victim boundary identity and does not deserialize payloads or collect victim IDs. Admission uses at most four SQL statements: one age cleanup, one shared sum, one ordered victim scan and one prefix delete. When the non-evictable staging/check portion alone exceeds the budget, the operation returns an explicit content-budget limitation and its caller rolls back publication/progress. It does not discard incomplete traversals or durable membership to manufacture capacity.

Publication's opportunistic age cleanup now removes at most **128** expired content rows, rather than the entire expired history. Independent periodic maintenance continues reclamation. Existing read-time age filters enforce visibility for their covered resource classes. The exact 30-day boundary remains inclusive. LRU pressure may require removing additional expired or recent payload rows in the same transaction.

**Limits:** this is a bound on Rust victim memory, SQL statement count and opportunistic age-cleanup mutations. It is **not** a bound on LRU mutation count, SQLite sort/temp space, query execution time or total database size. Published check payloads count toward the ceiling but still lack general age/LRU reclamation; a staging/check-only budget failure therefore remains an explicit limitation. Broader payload lifetimes, startup cleanup, all-row mutation bounds, physical fault/peak-space qualification and temporary-file accounting remain required work. No full storage qualification or dispatch readiness is inferred.

## Evidence

All new evidence is deterministic, with real SQLite. The worker test runs the actual bounded process runner against the existing synthetic gh executable. No live GitHub read, native app launch, OS permission request or delivery occurred. Reduced byte ceilings are passed only to the private algorithm helper in tests; production remains fixed at 500 MiB. These tests do not establish a full-size 500-MiB performance/capacity pass.

| Subcase | State | Evidence |
| --- | --- | --- |
| Inclusive byte ceiling, UTF-8 and deterministic LRU | Pass, deterministic SQLite | Exactly 12 bytes survives; one-byte excess reclaims the oldest sufficient prefix, including an older zero-byte row; tied access times resolve by key. Multibyte text is counted as bytes. |
| Many victims and durable history | Pass, deterministic SQLite/reopen | A payload plus 200 small cache rows are evicted through the streamed prefix; 57 newer rows remain. Minimal Activity, unseen value, dedup, baseline, terminal intent membership and attempt history survive reopening. |
| Shared allocation classes and reporting | Pass, deterministic SQLite | Each legacy/source/Activity staging class and both staged/published check contexts consume the same scaled ceiling. Storage reporting agrees. An impossible admission rolls back new content, progress and revision. |
| Bounded publication expiration | Pass, deterministic SQLite | 129 expired content rows leave one after publication's 128-row cleanup; periodic maintenance removes the remainder. Exact-age content survives. |
| Selected-registration atomicity | Pass, SQLite trigger fault | Failed eviction leaves no new locator, membership, list payload or revision. Removing the fault permits normal registration. |
| Head-publication atomicity | Pass, SQLite trigger fault | Failed eviction rolls back head epoch, confirmed closure/removal, staged-check deletion, check completeness, cached list title and revision. Removing the fault permits publication. |
| Process → worker → publication fault → protection/repair | Pass, actual worker/process/SQLite with synthetic gh | Manual selection of a previously unknown PR fails inside registration/eviction. No new PR locator or overview commits. Existing cached overview remains readable; explicit Refresh is rejected. Repair cleans up and completes the health probe/checkpoint while preserving Pause and starting no additional child reads. A subsequent explicit selection succeeds without creating pending native delivery. |
| Full 500-MiB and physical storage qualification | Pending | Scaled functional tests do not prove full-size memory/temp/time behavior, read-only/exhausted filesystem or IOERR, process-kill commits, peak migration space, published-check reclamation or all-row mutation bounds. |
| Native notification implementation and positive live fixtures | Pending / previously Blocked | Native dispatch/onboarding/categories/reservation/revalidation/click routing remain absent. Existing terminal unavailable/attempted history stays terminal. Positive live fixture gaps remain unchanged. |

The initial full test run exposed an existing cancellation-test race: file existence was treated as proof that the helper PID had been written. The fixture now waits for a newline-terminated parseable PID before cancelling, preserving the process-reaping assertion. The new worker test initially selected an already-discovered fixture PR; it now uses an unknown PR to establish rollback of registration. Neither correction changes production transport behavior.

## Verification

- `cargo test --workspace --offline`: **69 tests pass** (68 core, one desktop IPC), including six new budget/store tests and one new worker test. The final newline-terminated PID fixture was also verified by a separate passing `cargo test -p gitjuke-core --offline --test transport` run.
- `npm run build`: TypeScript and frontend production build pass.
- `cargo clippy --workspace --all-targets --offline -- -D warnings`: Pass.
- `cargo fmt --all -- --check`: Pass.
- `npm run tauri build -- --debug --bundles app --target aarch64-apple-darwin`: Pass; **31.00 MiB** local development `.app`. Build only; not launched in this slice.
- Bundled executable SHA-256: `cdd9a5a36faa3546b4503899771dba48a19281f786eddae61917cc126e5830fc`.

No signing, native performance, full-size storage-capacity or native-delivery pass is inferred from the local development artifact. All **29 grouped release scenarios remain Pending**. No GitHub content, credentials, permission, login, signing, publication or distribution settings changed.
