# Activity storage integrity and crash durability checkpoint — 2026-10-17

Status: the next storage prerequisite is qualified through real staging, Activity publication, reconciliation and recovery completion under actual process death. Development version **0.1.0**, schema **4**. No Git repository or applicable AGENTS.md was found in the workspace or its ancestors. Settled contracts are unchanged.

## Audit and scope

The remaining storage work includes LRU admission and broad invalidation/purge mutation bounds, recovery-completion mutations, full-size 500-MiB/query/temp qualification, source/metadata lifetimes, terminal compaction, startup recovery UX, physical I/O faults and migration/ingestion/delivery crash boundaries. Startup cleanup and recovery initialization now have real SIGKILL evidence, while Activity/progress/eligibility-intent atomicity previously relied on SQL-error rollback tests.

This slice closes the **Activity staging/publication and recovery-summary process-crash evidence gap**. It exercises the existing production transaction boundaries, rather than replacing transactional publication with partial commits. The crash results required no change to transactional publication. The separate purge correction below fixes a production resource boundary; schema stays 4. `activity_crash.rs` and all barriers at the real store operations compile only under `cfg(test)`; the development app contains no new fault switch. No production fixture fallback is added.

The parent launches the actual core Rust test executable, enters `Store::open`, stages individual source records through `activity_stage` and publishes through `activity_publish` or completes reconciliation through `activity_finish_recovery`. Test-only barriers stop execution at named boundaries. The parent sends actual **SIGKILL**, verifies the signal and reaps the child. It then opens SQLite directly to inspect the immediate committed state **before normal startup cleanup can alter the evidence**. Finally it reopens the production Store, enters recovery and reconciles source IDs again through the real publication path and Activity reader.

All input is synthetic, SQLite and process death are real. No gh subprocess, live GitHub read, native UI, permission or OS notification dispatch occurs in these new cases. This is not power-loss or filesystem-IOERR qualification.

## Denied-resource purge correction

The audit also found that Activity denial used `LIKE 'api:repos/{repo}/%/{number}%'` to purge source payloads. Denying PR 1 could delete PR 10/100 payloads. Underscores in legal repository names acted as SQL wildcards and could match another repository. This violated the existing resource-specific boundary.

Purge now matches literal pull/issue roots case-insensitively, followed only by end-of-key, `/` or `?`. It removes all cached source pages for the affected PR and its retained Activity excerpts, without matching a longer number or another repository. The existing transaction still removes abandoned Activity staging, invalidates target completion and bumps the revision; minimal Activity, unseen/dedup/progress/intent/attempt history and deadlines survive. This does not bound purge mutations or change general invalidation behavior.

A real SQLite test covers uppercase roots, underscores, all implemented source families, pagination, direct/query/path boundaries, longer PR numbers, another repository and malformed suffixes. A target-update trigger fails after payload/staging deletion, proving the transaction restores deleted payloads/staging and the revision; removing the trigger permits the exact purge while durable history survives. The actual worker/process/SQLite denial fixture now seeds PR 10's source payload before denying PR 1: the denied excerpts disappear, PR 10's cache survives, Pause persists and the account remains unlocked. The worker uses synthetic gh; no live denial is claimed.

## Evidence

Two behavioral tests cover **47 separate killed child processes**, plus one subprocess-only test entry. Every case verifies SQLite integrity, durable state and restart behavior.

| Subcase | State | Evidence |
| --- | --- | --- |
| Staging before/after commit | Pass, actual SIGKILL | Four modes—first historical baseline, ordinary arrivals, incomplete recovery and persistent-Pause manual reads—are killed before and after staging commit. Uncommitted staging is absent; committed staging alone creates no Activity/seen/dedup/publication progress/intent. Normal startup discards abandoned staging and requires fresh complete hydration. |
| Partial per-record publication | Pass, actual SIGKILL | Each of those four modes is killed after the first dedup insertion, after the first minimal Activity/seen insertion, after its eligibility/catch-up decision, and after all payload writes. Two additional kills stop ordinary/recovery publication immediately after PR intent creation and before member insertion. The entire publication rolls back: no partial minimal rows, seen/dedup identities, excerpts, catch-up membership or new eligibility intents survive. Previously committed staged rows remain for inspection. |
| Successful progress and pre-commit | Pass, actual SIGKILL | Kills after staging deletion/target completion/successful PR progress and immediately before transaction commit leave the old progress value, incomplete target, original revision and committed staging. No partial progress or intent commits. |
| Complete publication commit | Pass, actual SIGKILL | After commit, all three minimal Activity/seen/dedup/payload rows, target completion, successful progress, staging removal and revision increment survive together. Normal arrivals have one unavailable per-PR intent with three members; recovery has two pre-boundary catch-up members and one post-boundary unavailable per-PR member. Historical baseline and paused manual reads create no new eligibility intents. |
| Restart and repeated source hydration | Pass, real Store/SQLite/Activity reader | Each killed case restarts recovery and republishes the same complete source IDs twice. All three entries remain unique; baseline entries remain seen and subsequent eligible entries remain unseen. Pre-commit ordinary arrivals reconcile into one catch-up summary; already committed unavailable ordinary intents are not replayed into a summary. Recovery retains one summary plus the post-boundary PR group. No pending delivery state is created. |
| Cross-PR summary atomicity | Pass, actual SIGKILL | Five kills: after summary intent insertion, after member insertion, after catch-up progress deletion, before commit and after commit. Before commit, the old recovery mode/boundary, catch-up set, revision and lack of a new summary survive together. After commit, one two-member cross-PR unavailable summary, removed catch-up keys, normal mode and revision survive together. |
| First baseline completion atomicity | Pass, actual SIGKILL | Four kills around membership/no-members, catch-up deletion, pre-commit and post-commit. Before commit, no baseline progress or normal-mode claim survives. After commit, the historical baseline and normal mode survive together without a new summary intent. |
| Summary filtering and restart idempotence | Pass, real Store/SQLite | Summary candidates include a seen entry and an expired minimal entry, alongside eligible unseen records from two PRs. Seen/expired/suppressed/previously manually discovered entries do not join the summary. Reopening, full target reconciliation and repeated completion retain exactly one two-member summary and leave its members unseen. A completed cycle gets a new restart boundary; an interrupted cycle retains its original boundary. |
| Durable history and unrelated data | Pass, checked across every restart | Original stable identity, stored Pause, an existing unseen/dedup sentinel, direct watch membership, unrelated cached payload, explicit attempted/unavailable outcomes, original intent membership, attempt ledger and notification validator/server deadline survive. No existing terminal outcome becomes pending or recovery work. |

The publication matrix comprises nine common barriers × four modes, plus two immediate PR-intent-creation barriers (**38 kills**). Recovery completion comprises five summary and four first-baseline barriers (**nine kills**). SQL state is checked both immediately after death and after production Store recovery; the barriers do not manufacture SQL failures or catch them with a normal rollback/destructor.

## Remaining limits

Native delivery remains absent. Existing unavailable eligibility records remain terminal. These tests do not implement or qualify unattempted-intent membership reconstruction, reservation/revalidation, durable pre-dispatch attempts, ambiguous OS dispatch or notification click routing. Preserving an existing attempted sentinel is not an attempted-delivery pass. No dispatch readiness is inferred.

The slice does not bound publication/recovery-completion row mutations, total query/runtime duration, sorting/temp/index/page work, whole-database size or peak migration space. LRU/broad invalidation/purge bounds, other lifetimes/compaction, full-size budget and temporary-file accounting, physical exhaustion/readonly/IOERR, migration kills and startup recovery UX remain Pending. It does not qualify remote schema/classification compatibility, actual native schema-4 migration or positive live arrival/recovery/capacity fixtures.

All **29 grouped release scenarios remain Pending**. Previously Blocked live fixtures remain Blocked. The narrower staging/Activity/progress/eligibility-intent/recovery-completion SIGKILL subcases now have Pass evidence; migration/native attempt/dispatch and the rest of their grouped gate remain open.

## Verification

- `cargo test -p gitjuke-core --offline --lib activity_crash`: Pass; all 47 crash/restart cases, two behavioral entries and the child entry.
- `cargo test --workspace --offline`: **93 tests pass** (92 core, one desktop IPC). Four new entries comprise two crash behavioral tests, their subprocess-only entry and the purge rollback/scope test; the existing worker denial fixture is strengthened.
- `npm run check`: Pass; TypeScript/frontend build and workspace Clippy with warnings denied.
- `cargo fmt --all -- --check`: Pass.
- `npm run tauri build -- --debug --bundles app --target aarch64-apple-darwin`: Pass; **31.02 MiB** local development `.app`, build only.
- Bundled executable SHA-256: `fa63933f484d85d392df9aa1ff5936e1a0e453e5aa44cb3962357a90836f9034`. The Activity crash environment-switch strings are absent from this executable.

The first purge-test compile used an unsupported `usize` SQLite row conversion; it was corrected to an integer conversion before the passing targeted checks. No behavior failure was accepted as an exception.

No credentials, GitHub content, permission, login, signing, publication or distribution settings changed.
