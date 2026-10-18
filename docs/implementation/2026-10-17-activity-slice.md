# GitJuke — notification discovery and individual Activity checkpoint

Production work authorized after the completed decision map. Development version 0.1.0, schema **3**, Apple Silicon/macOS 15 minimum, `dev.hmrguez.gitjuke`. This checkpoint implements an additional vertical slice; it does not certify the full notification feature or a distributable release. All 29 grouped release scenarios remain **Pending**. Confirmed policy is unchanged.

## Implemented behavior

The single Rust runtime now discovers changed PR notification threads using installed `gh api`, including read threads (`all=true`). Fixed GitHub.com endpoints, existing stored stable identity checks, sanitized subprocess environment, serialized process bounds and quota/backoff handling remain in force. Thread reasons never create individual events. Discovery records exact-URL validators and persists server `X-Poll-Interval` deadlines immediately after receiving headers, before subsequent identity validation. Resume/restart/manual work cannot reset that deadline. Notification traversal uses a fixed `since`/`before` window with a one-second overlap and one page per runnable descriptor; GitHub notification pages contain at most 50 threads.

Authored/requested watches and discovered notification PRs hydrate authoritative PR descriptions, general comments, submitted reviews, review comments and timeline review-request records. Each source pages at 50, reducing pages after bounded-output failures. Conditional 304 reads reuse the corresponding exact-URL cached payload; after eviction they require a fresh unconditional read under the existing deadlines. Missing rendered mention data, malformed sources, duplicate individual IDs across pages, denied resources and unfinished pagination do not publish successful individual progress. Ordinary comments on unrelated PRs remain outside personal Activity. Personal mentions require a GitHub-rendered user/team mention link; literal text in code does not qualify. Requests match stable user/team identities. Textless submitted reviews are retained.

A completed PR batch atomically publishes individual Activity, compact dedup/seen state, successful hydration progress and grouped eligibility intents. Own actions and routine bot comments remain readable and do not become seen merely because they are suppressed. Actions terminal outcomes use authoritative workflow-run/attempt plus commit identity; supplied terminal legacy statuses retain their source timestamp and commit. Current head identity is retained with the record and must match authoritative hydration. Missing contexts never imply completion. **External check-suite retry classification is unavailable**, rather than guessing an attempt from a timestamp. Full provider/retry qualification remains required.

The first completed reconciliation of implemented sources establishes a historical, seen baseline. A later-discovered source record that predates that baseline remains historical, preventing sticky-thread changes from replaying old mentions. Recovery persists its start boundary, waits for discovery/hydration, and produces one cross-PR summary intent for newly discovered, eligible, still-unseen pre-boundary records. Post-boundary records group normally per PR. Manual paused reads populate Activity without alert eligibility intents; manually discovered duplicates and seen records cannot enter a later summary. Resume restarts incomplete traversals while preserving successful progress and notification deadlines. Startup, a long scheduler gap and transport recovery enter reconciliation; partial recovery stays labeled incomplete.

Activity payloads and source/staging responses participate in the existing shared 500-MiB disposable budget. Minimal Activity/seen entries remain readable after Clear downloaded content or payload eviction; 90-day expiration removes Activity rather than marking it seen. Identity-only dedup/progress/seen ledgers survive. Resource denial purges associated excerpts/source payloads without claiming deletion or losing watch membership. Schema 3 transactionally upgrades schema 1/2; existing identity, appearance, Pause and ledgers are preserved.

The React Activity view uses the approved Stacks shell, bundled JetBrains Mono and existing theme tokens. It shows historical baseline/recovery/error state, individually seen/unseen records, retention and unavailable functionality. Explicitly opened source detail acknowledges that item after its React destination renders; evicted/limited text does not acknowledge it. Mark loaded page seen sends only the loaded ID snapshot (maximum 100); subsequent arrivals are unaffected. Activity IPC also measures serialized rows to stay below the 1-MiB response ceiling. Browser text is rendered as plain text.

**Native delivery remains unavailable.** Eligibility intents are durably terminal `unavailable` records, so a future enabling path cannot replay this backlog. No permission request, OS dispatch, delivery retry or login registration occurs. The existing attempt ledger is preserved, but pre-dispatch attempt reservation, native revalidation/dispatch/click routing, onboarding/permission and category controls still require implementation and fault verification before delivery can be enabled. No attempted-delivery/no-replay pass is claimed by an implementation that does not dispatch.

## Verification evidence

Environment: development host Apple Silicon, macOS 27.0/26A428, Rust 1.95.0, gh 2.92.0, Tauri 2.12.4, REST version 2026-03-10. This remains outside the physical M1/8-GiB/macOS-15 performance reference.

### Deterministic evidence

The real worker/process/SQLite tests use only the test executable in `tests/support/gh.py`; production has no fixture fallback. New passing subcases cover:

- Sticky-reason rejection, rendered-versus-code mentions, stable team request matching, own/bot suppression and textless reviews.
- A 51-comment, multiple-page hydration; historical baseline; persistent Pause with explicit hydration; seen acknowledgements; Resume waiting for the original 120-second fixture polling deadline; honest partial recovery.
- Conditional payload reuse without replay; denied individual sources retain prior progress, discard incomplete staging and purge excerpts.
- Atomic rollback of Activity/dedup/seen/progress/intents on a SQLite write fault; recovery boundary preservation across reopening; one summary excluding manual duplicates/already-seen records; post-boundary grouping; Clear preserving durable replay prevention.
- Actions attempts, head/merge identities and independent supplied legacy statuses. A request-only source fixture does not establish pipeline authorship; authoritative PR authorship does.

Hydration exposed an existing scheduler expansion limitation: one reserved head batch could sit behind a large number of individual source jobs. The scheduler now reserves disjoint due head batches before expanding hydration, while still materializing at most 128 runnable descriptors. An oversized response resizes all reserved batches, preserving watch due state. The existing **151-watch rotation and 10→5→2 overflow tests pass with their original 25-second timeout**. This remains synthetic scheduling evidence, not real 50-watch capacity or ten-second latency qualification.

### Authorized real reads and native app

`cargo run -p gitjuke-core --example activity_probe --offline` uses a temporary database and existing gh authentication. It emits counts/deadlines only and removes temporary data after shutdown. It does not mutate GitHub or credentials.

The completed public probe hydrated all five resources of the previously authorized `cli/cli#14719`: **5 cached source pages**, no reported source errors, **42,334 ms** elapsed. At that sample, recovery remained incomplete with five pending discovered PRs and no retained Activity yet; it is not a live recovery pass. The recorded budgets were Core 76 requests/468,571 response bytes/4,774 remaining, Search 4/5,572/26, GraphQL 1/1,439/4,999. Counts measure recorded CLI responses, not transport wire bytes or a capacity promise.

The real development application subsequently migrated its existing schema-2 database, completed discovery/hydration and displayed an established historical baseline with **18 retained real Activity records**, all seen. It showed own-action and routine bot-comment suppression and submitted reviews. CUA inspected `tauri://localhost`, opened a real individual source destination as safe plain text, closed it, and explicitly quit using Cmd-Q. Evidence records counts and behavior only; repository names, actors and source bodies from private/personal data are not reproduced here. This establishes a real historical authored-comment/review path discovered from notifications, including closed PR history. It does not establish positive live direct/team request, personal mention, retry or new-unseen-arrival behavior.

UI changes reuse the project's existing components/tokens. 21st metadata search informed comparison; no catalog code or hosted generation was used. The 21st CLI was unavailable locally and its initialization attempt could not reach npm, so no CLI review pass is claimed. TypeScript/Clippy and native inspection provide the verification recorded here.

### Remaining required evidence

- **Pending implementation:** native permission/category controls, generic preview delivery, pre-dispatch attempt reservation/revalidation/click routing and crash dispatch matrix; external suite retry classification; saved queries, complete diffs/discussions/logs and login integration.
- **Pending verification:** live post-baseline unseen arrivals and completed recovery/Resume summaries, real workflow/status/retry Activity, mutable notification traversal/race matrix, 304-after-eviction, kill-at-every-commit-boundary, full disk/write/checkpoint/retention/1-GiB warning and migration-failure matrix, exact-URL validator behavior against real 304 responses, minimum viewport/200%/VoiceOver and physical performance measurements.
- **Blocked on suitable authorized fixtures:** positive live direct/team review requests, mention elsewhere, published native stack, private/fork/deleted-head watch cases, external CI/retries and a real 50-watch set. Existing historical authored Activity does not prove positive open-authored discovery/capacity.
- **Pending release prerequisites:** qualified OS matrix, signing/notarization/stapling and exact DMG lifecycle. No membership/signing credentials were purchased/provisioned; no artifact was published or distributed.

Primary API references used: [Notifications and server-directed polling](https://docs.github.com/en/rest/activity/notifications?apiVersion=2026-03-10), [individual timeline records](https://docs.github.com/en/rest/issues/timeline?apiVersion=2026-03-10), [submitted reviews and media types](https://docs.github.com/en/rest/pulls/reviews?apiVersion=2026-03-10). The settled specification remains authoritative.

## Final verification and applicability

`cargo test --workspace --offline`: **44 tests pass** (43 core, one desktop IPC test). `npm run check` passes TypeScript, frontend production build and Clippy with warnings denied. The final arm64 development `.app` builds successfully. The verification app was explicitly quit; no notification permission or login setting was changed.

Native observation preceded the final scheduler reservation/overflow refinements, authoritative head guard and source-head label. The real historical comment/review source path, SQLite migration and individual text destination are unaffected by those refinements; automated regression checks cover the changed scheduling/head paths. A complete native replay against the final release artifact remains Pending. No native retry, post-baseline arrival, summary dispatch, external-suite or full accessibility pass is inferred from the historical baseline observation.

Final local arm64 development executable SHA-256: `b73efca1deb1f6dfa44d0609b8798cad97a3eaf159643e183168edf5046e3049`. This fingerprints a local development executable, not a signed DMG or an authorized distribution.
