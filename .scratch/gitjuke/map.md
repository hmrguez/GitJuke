# Find the specification and UI for GitJuke

Labels: wayfinder:map
Status: complete
Completed: 2026-10-17

## Destination

An implementation-ready specification and a human-reviewed UI prototype for a minimal personal GitHub PR reader and activity monitor, using Tauri, React, and a Rust backend invoking the installed gh CLI.

## Notes

- Consult wayfinder, domain-modeling, and frontend-design. Use research for investigations and prototype for the later UI discussion.
- Local Markdown tracker: child issues live in issues/; open, unassigned, unblocked children form the frontier in numeric order. Resolution records an Answer in the ticket and a linked gist here.
- User confirmed: one GitHub.com account through existing gh authentication; read-only stack navigation; menu bar monitoring continues with the window closed; specification and UI prototype before implementation.
- The user's actual stack workflow is `gh stack`; provider compatibility research must target that workflow rather than assuming Graphite or manual chains.
- Required reading surfaces: PR lists, GitHub PR filter presets, changes, conversations/reviews, checks, and stacks.
- Personal activity includes comments/reviews on authored PRs, direct/team review requests, mentions elsewhere, and pipeline completion for authored PRs.
- User accepted ten-second active PR/check refresh targets while awake and online, with personal notification discovery respecting GitHub's returned interval. No universal delivery guarantee.
- User accepted local saved queries for version one rather than synchronization with GitHub saved views.
- User confirmed the latest appearance direction: JetBrains Mono throughout, light/dark presets, and four customizable colors (background, surface, accent, text). Preserve it. Stacks is the confirmed default layout.
- UI: modern, clean, rounded corners, compact purposeful labels; no decorative text or controls.
- No Git repository exists yet. Research assets stay in separate local files rather than introducing throwaway Git branches into an uninitialized project.
- Specification and human-reviewed prototype precede production implementation. UI direction, notification policy and runtime/cache decisions are now confirmed; do not reopen them without a newly identified conflict. Historical proposal/pending statements in ticket comments are superseded by their later confirmations and final Answers.
- Settled handoff assets: [specification](spec.md), [human-reviewed prototype notes](prototype/README.md), [confirmed handoff/release acceptance](acceptance.md). The browser prototype uses fixtures only. The planning destination is complete; future implementation requires a subsequent user instruction.

## Decisions so far

- [Determine whether GitHub PR presets are accessible through gh](issues/01-github-pr-presets.md): no supported saved-view discovery operation identified; the later preset/stack decision accepts local saved queries for v1.
- [Establish personal activity coverage and realistic polling latency](issues/02-personal-activity-polling.md): server-directed notification polling differs from a plausible ten-second known-PR/check refresh target; the ticket holds coverage, quota, and recovery findings.
- [Establish gh data access for the PR reader and stacks](issues/03-reader-and-stack-data.md): reader data and published native gh stack relationships have public read APIs; pagination, permissions, and live validation limits are recorded in the ticket.
- [Choose freshness and personal notification behavior](issues/04-freshness-and-notification-policy.md): user accepted ten-second active PR/check targets and server-directed notification discovery; alert-policy details have moved to their own ticket.
- [Choose preset and stack behavior at API boundaries](issues/05-presets-and-stack-contract.md): local queries accepted; target published native gh stack navigation, with provider compatibility retained as a verification gate.

- [Choose the minimal PR reader interface through a prototype](issues/06-ui-reader-prototype.md): human accepted Stacks as the default and the latest four-color JetBrains Mono appearance; fixture evidence does not validate live/native behavior.

- [Choose notification defaults and the active watch set](issues/08-notification-defaults.md): watch/alert, privacy, seen, onboarding and persistent Pause/Resume policy confirmed; alerts include the current PR and resume uses grouped catch-up.

- [Choose the local runtime and cache boundaries](issues/07-runtime-and-cache-contract.md): Rust/SQLite ownership, identity isolation, bounded transport/queues, revision guards, retention/storage growth and migration/recovery contracts confirmed; live/native verification remains pending.

- [Choose large-diff reading and performance limits](issues/09-large-diff-behavior.md): bounded unified patches, continuous/paged accessible reading, 50-message discussion pages and native performance acceptance confirmed; measurements remain pending.

- [Establish macOS packaging and gh launch constraints](issues/12-macos-distribution-feasibility.md): primary-source signing/DMG, target, updater, GUI gh environment and autostart constraints recorded; later packaging decisions are confirmed, with packaged validation pending.

- [Choose macOS packaging, signing, and distribution](issues/10-packaging-signing-distribution.md): arm64/macOS-15 signed direct DMG, manual schema-aware updates, stored-gh auth context and single-instance login lifecycle confirmed; release/native evidence remains pending.

- [Settle specification handoff and release acceptance](issues/11-final-acceptance.md): handoff/release rules, 960×640 content minimum, required evidence matrix and failure/revision policy confirmed; specification settled and planning complete, with production verification pending.

## Not yet specified

None. All child decisions are resolved; no open frontier or in-scope fog remains. A future finding that changes the contract requires an explicit specification revision/new decision rather than silently changing this completed plan.

## Out of scope

- Posting comments/reviews, merging, branch updates, rebasing, or stack mutation.
- Multiple GitHub accounts, Enterprise hosts, and a separate hosted backend for the first release.
- Production implementation during this planning effort.

## Session checkpoint — 2026-10-17

The user confirmed all four final acceptance recommendations and explicitly requested planning completion. Final acceptance is resolved, all twelve child tickets are resolved, the specification is settled, and this map is complete. Stacks and its JetBrains Mono/light-dark/four-color appearance remain human-approved. The 29-scenario acceptance matrix preserves all production evidence as Pending, alongside historical fixture review. Signing access/release-source/version inputs and API/native/performance evidence are future release gates. No production implementation, signing, auth/API/native execution or publication started; subsequent implementation needs the user's instruction. No next frontier remains within this completed planning effort.

## Production follow-on — 2026-10-17

The user subsequently authorized implementation in this workspace, without reopening resolved decisions. See the [first production slice](../../docs/implementation/2026-10-17-first-slice.md) and updated [acceptance evidence](acceptance.md). This map remains a completed planning record. Production has begun, with partial real native/API/runtime evidence; all grouped release gates remain open. No signing/provisioning/publication authorization is implied.

## Watch/head/check implementation follow-on — 2026-10-17

The next authorized vertical slice implements source-complete authored/direct/team/selected reconciliation and bounded batched head/check scheduling. [Current evidence](../../docs/implementation/2026-10-17-watch-slice.md) separates synthetic membership/partition/fault coverage from live six-source discovery, public head/check reads and native close/reopen. Planning decisions remain settled; the 29 grouped release scenarios remain open.

## Recovery/storage implementation follow-on — 2026-10-17

Authorized implementation continues with [recovery/storage prerequisite hardening](../../docs/implementation/2026-10-17-recovery-hardening.md). Worker storage protection and interrupted recovery boundaries now have additional deterministic evidence; native delivery remains unavailable pending its implementation and prerequisite qualification. This completed decision map is unchanged; no settled choice was reopened.

## Storage availability implementation follow-on — 2026-10-17

Authorized implementation continues with [startup/cache storage availability](../../docs/implementation/2026-10-17-storage-availability.md): protected current-schema startup, cached navigation under denied writes, transactional migration rollback and measured-file growth warnings. Deterministic evidence and remaining physical/temp/crash/native gaps are separated in that checkpoint. Native delivery remains unavailable. The completed decision map and confirmed scope are unchanged.

## Retention maintenance implementation follow-on — 2026-10-17

Authorized implementation continues with [bounded retention maintenance](../../docs/implementation/2026-10-17-retention-maintenance.md): periodic cleanup during Pause, protected-cache lifetime enforcement, transactional cleanup failure/repair and expired-source replay prevention. Durable seen/deduplication and terminal delivery identities/outcomes survive. Evidence is deterministic; native dispatch and broader storage qualification remain open. The completed decisions and contract are unchanged.

## Shared content-budget admission follow-on — 2026-10-17

Authorized production implementation continues with [shared budget admission](../../docs/implementation/2026-10-17-content-budget.md): selected/head cache writers now enforce admission transactionally, LRU selection streams byte metadata without repeated full-cache sums, and publication age cleanup is batched. New deterministic store/worker evidence preserves durable ledgers and Pause through protection/repair. Full reclamation bounds, physical/temp/peak-space qualification and native delivery remain open. Settled decisions are unchanged.

## Published check reclamation follow-on — 2026-10-17

Authorized implementation continues with [published check reclamation](../../docs/implementation/2026-10-17-check-reclamation.md): schema-4 access metadata, shared content/check LRU, bounded check age cleanup, protected expiry and transactional worker repair. Deterministic evidence preserves durable history and Pause. Native dispatch, broader storage qualification and the 29 grouped release gates remain open; settled decisions are unchanged.

## Disposable metadata reclamation follow-on — 2026-10-17

Authorized implementation continues with [disposable metadata reclamation](../../docs/implementation/2026-10-17-metadata-reclamation.md): bounded removal of payload-less validators/API health and expired empty obsolete check states, preserving server deadlines, current watched health and unfinished traversals. Deterministic SQLite/worker evidence covers rollback, paused protection/repair and unconditional explicit rehydration without duplicate Activity. Schema remains 4. Native delivery, broader storage qualification and all 29 grouped release gates remain open; settled decisions are unchanged.

## Bounded startup reclamation follow-on — 2026-10-17

Authorized implementation continues with [bounded startup reclamation](../../docs/implementation/2026-10-17-startup-reclamation.md): restartable transient/session cleanup with at most 128 direct row mutations per transaction and monitoring gated until completion. Real subprocess SIGKILL covers uncommitted and committed cleanup boundaries; deterministic SQLite/worker tests cover late-batch failure, protected startup, paused repair, original-identity validation and preservation of durable history/deadlines. Schema upgrades remain atomic; schema stays 4. Broader query/time/temp/migration/physical/delivery qualification and all 29 grouped release gates remain open. No settled choice changed.

## Bounded recovery initialization follow-on — 2026-10-17

The [recovery initialization checkpoint](../../docs/implementation/2026-10-17-recovery-initialization.md) adds Pass subcases for a global 128-row mutation bound across recovery target/staging/job/provisional-intent resets, late-batch rollback/restart and actual subprocess SIGKILL after boundary commit and before/after batch commit. Worker tests verify protected, identity-locked startup after partial initialization, failed repair without reads, successful repair retaining Pause/boundary/deadline and explicit bound-identity validation. Terminal attempted/unavailable history stays terminal; unattempted membership reconstruction and native delivery remain absent. Schema stays 4. Completion-summary mutations, broad invalidation/LRU bounds, query/time/temp/full-size/physical/migration/ingestion/delivery qualification remain open. No new native/live evidence is claimed. All 29 grouped release scenarios remain Pending and existing Blocked fixtures remain Blocked; the settled contract is unchanged.

## Activity crash durability follow-on — 2026-10-17

The [Activity crash durability checkpoint](../../docs/implementation/2026-10-17-activity-crash-durability.md) qualifies the existing staging/publication/progress/eligibility-intent/recovery-completion transactions with 47 actual subprocess SIGKILL cases. Immediate SQLite inspection precedes startup cleanup; reopening and repeated complete hydration verify no torn ledger/progress state, duplicate source entries or duplicate recovery summaries. Historical baseline, normal arrivals, recovery/post-boundary grouping, persistent-Pause manual reads, summary filtering and terminal-history/deadline preservation have Pass subcases. All crash hooks are test-only; schema stays 4. A production purge correction replaces wildcard/numeric-prefix matching with literal PR path boundaries, with SQLite rollback and actual worker denial/neighbor-cache evidence. Native delivery remains absent, and migration/attempt/dispatch/physical/full-size/query/temp/mutation-bound qualification remains open. No live/native evidence or dispatch readiness is inferred. All 29 grouped release scenarios remain Pending; existing Blocked fixtures and settled contracts are unchanged.

## Migration storage admission and crash follow-on — 2026-10-17

The [migration durability checkpoint](../../docs/implementation/2026-10-17-migration-durability.md) adds full SQLite synchronization before migration writes and repeated space admission before each upgrade step and commit. Schema upgrades still commit atomically; schema remains 4. Pass subcases cover 44 actual SIGKILL cases across schemas 0/1/2/3 and DELETE/WAL modes, 28 late-space failures with whole-schema/data rollback, complete backfills/history preservation and runtime stop followed by paused, identity-locked repair without reader execution. Immediate SQLite snapshots precede normal startup cleanup. Peak migration/temp allocation, physical I/O/power-loss, native schema migration, signed binary replacement, broader mutation/query/full-size bounds and recovery UX remain Pending. Native delivery remains absent, existing Blocked live fixtures remain Blocked, and all 29 grouped release scenarios remain Pending. Settled contracts are unchanged.

## Bounded Activity enrollment checkpoint — 2026-10-17

The [Activity enrollment checkpoint](../../docs/implementation/2026-10-17-activity-enrollment.md) adds Pass subcases for at most 128 new watch-derived hydration targets per transaction/scheduling pass, locator/reason deduplication, preserved existing target deadlines and durable history, and recovery completion blocked until missing enrollment and unfinished hydration resolve. A later-batch SQLite failure rolls back that batch; two actual subprocess SIGKILL cases preserve only committed enrollment and restart to completion. The real worker with synthetic gh verifies paused manual scheduling failure, storage protection and repair without extra child reads or changed ledgers/boundary/deadline. Schema remains 4. SQL/time/temp/full-size/physical/peak-migration, other broad mutations/compaction/recovery UX and native delivery remain Pending. All 29 grouped release scenarios remain Pending, existing Blocked fixtures remain Blocked, and settled contracts are unchanged.

## Full-size shared content-budget checkpoint — 2026-10-17

The [full-size content-budget checkpoint](../../docs/implementation/2026-10-17-full-content-capacity.md) adds functional Pass evidence at the actual **524,288,000-byte** shared payload ceiling: disk-backed exact-fit admission, mixed content/check LRU, incomplete-check invalidation, non-evictable staging refusal at one byte over, real writer rollback/retry and restart preservation. Two actual subprocess SIGKILL cases include observed WAL writes and raw SQLite inspection before startup cleanup. The actual worker with synthetic gh verifies full-size publication failure, protection and isolated paused repair, followed by explicit successful retry. The production algorithm required no behavior change; new fixtures and barriers are test-only, schema stays 4. This does not qualify worst-case row counts, query/sort/temp/time/memory, broad mutation bounds, physical faults, peak migration allocation, compaction/startup UX or native delivery. Existing Blocked live fixtures remain Blocked and all **29 grouped release scenarios remain Pending**; settled contracts are unchanged.

## Bounded global invalidation checkpoint — 2026-10-17

The [bounded invalidation checkpoint](../../docs/implementation/2026-10-17-bounded-invalidation.md) implements a durable generation/revision barrier followed by restartable transactions of at most 128 direct mutations, including final marker removal. Logical check completeness/classification and recovery completion honor the pending marker; startup and storage repair finish cleanup before reads resume. Real SQLite faults and five actual SIGKILL cases preserve only committed batches, published caches, durable/unseen/terminal history and deadlines. The actual worker with synthetic gh verifies partial failure, cached access, paused protection/repair and old-generation rejection. Schema is now **5** so schema-4 binaries cannot ignore unfinished invalidation; expanded migration evidence covers 60 actual SIGKILL and 40 late-space failures across schemas 0–4 and DELETE/WAL modes. Clear/purge/LRU/recovery-completion bounds, query/temp/time/physical/peak-space/compaction/recovery UX and native delivery remain Pending. All 29 grouped release scenarios remain Pending; existing Blocked fixtures and settled contracts are unchanged.

## Bounded Clear downloaded content checkpoint — 2026-10-17

The [bounded clear checkpoint](../../docs/implementation/2026-10-17-bounded-clear.md) adds a durable logical/generation barrier, pending-payload hiding and restartable cleanup transactions of at most 128 direct mutations. Startup/repair integration, SQL faults, five actual SIGKILL cases and paused worker protection/repair preserve durable history and deadlines. Schema 6 prevents older binaries from ignoring partially completed clear; migration qualification expands to 78 kills and 54 space failures across schemas 0–5 in both journal modes. Remaining LRU/purge/publication/recovery-completion, query/temp/physical/peak-space/compaction/startup UX and native delivery gaps stay open. All 29 grouped release scenarios remain Pending and existing Blocked fixtures remain Blocked. Settled decisions are unchanged.

## Bounded check-access denial checkpoint — 2026-10-17

The [check-denial checkpoint](../../docs/implementation/2026-10-17-bounded-check-denial.md) implements a per-PR epoch/logical barrier, payload hiding and restartable 128-mutation cleanup for denied checks, check Activity excerpts/staging, queued check traversals and check completeness. Scope includes head-field and paginated resource denial. SQL faults, five actual SIGKILL cases and actual worker/protected-startup repair preserve minimal Activity/unseen/history, neighbor caches, Pause, identity and server deadlines. Schema 7 migration qualification covers 98 kills and 70 injected space failures across schemas 0–6 in both journal modes. Broader PR purge/LRU/publication/recovery-completion, query/temp/physical/peak-space/compaction/startup UX and native delivery gaps remain open. All 29 grouped release scenarios remain Pending and existing Blocked fixtures remain Blocked. No settled decision changed.

## Bounded full-PR resource purge checkpoint — 2026-10-17

The [full-PR purge checkpoint](../../docs/implementation/2026-10-17-bounded-pr-purge.md) adds a durable generation/visibility barrier and restartable cleanup transactions of at most 128 mutations across PR/list/API/Activity/check/staging/descriptor/health state. Overview, Activity and watched-head denial share it. Watch epoch watermarks and discovery restart guards prevent stale resurrection or false watch removal; durable history and unrelated caches survive. SQL faults, seven actual SIGKILL cases and real worker/protected-startup/discovery continuation evidence have Pass subcases. Schema 8 migration evidence covers 120 kills and 88 injected space failures across schemas 0–7 in both journal modes. LRU/publication/recovery-completion, query/temp/physical/peak-space/compaction/startup UX and native delivery gaps remain open. All 29 grouped release scenarios remain Pending and existing Blocked fixtures remain Blocked. No settled decision changed.
