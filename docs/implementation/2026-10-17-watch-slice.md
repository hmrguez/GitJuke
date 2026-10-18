# Watch discovery and head/check scheduling — 2026-10-17

Status: second vertical slice implemented, tested through the actual runtime and native app. **Not release ready.** The settled specification and all 29 required release scenarios remain unchanged. This checkpoint supersedes the watch/scheduler limitations in the first-slice checkpoint; it does not supersede that checkpoint's historical evidence.

## Delivered behavior

- Authored discovery uses the authenticated viewer's cursor-paged GraphQL `pullRequests(states: OPEN)` connection, with 100 records/page and explicit total/cursor/count checks. It has no Search result ceiling.
- Direct requests use `user-review-requested:<validated login>`. Team membership uses all pages of `GET /user/teams`; each stable team ID gets its own `team-review-requested:org/slug` traversal. Search pages are capped at 100; created-time ranges split recursively above GitHub's 1,000-result ceiling, without overlapping range endpoints. Successful enumeration beyond 1,000 is tested. An incomplete response, duplicate/missing record, changing count, failed page or unsupported schema preserves the prior source's membership and successful timestamp. A source that cannot be enumerated honestly remains incomplete; an irreducible >1,000-match single-second Search partition produces an explicit source limitation rather than dropping watches or inventing completeness.
- Watch identity is the PR node ID plus its base-repository locator/ID and number. Durable, independent `authored`, `direct`, `team:<stable ID>` and `selected` reasons deduplicate. A completed traversal replaces only its own source; team membership removal retires only the corresponding team sources. A successful closed/merged head snapshot removes active reasons while retaining historical cache/identity data. Failed reads never prove removal.
- Selecting an authorized open PR adds its selected reason; unselected stack neighbors are not registered. Closing the reader removes that reason while preserving the selection for reopening. Authored/direct/team watches continue. Minimize/unfocus do not remove reasons; visibility controls scheduling priority only. Focus is not an alert suppression input.
- Discovery is eligible every 60 seconds. Outstanding work persists separately from the materialized queue, which holds at most 128 runnable descriptors. Duplicate requests coalesce; continuations expand one page at a time; no missed-tick queue accumulates. After three foreground jobs, the oldest eligible background job runs. Primary budget blocking does not strand eligible work in another bucket behind the first 128 stored descriptors.
- Active heads/checks are eligible every ten seconds, including authored/requested watches with the reader closed. Compact batches contain at most ten PRs, prioritize a visible selection and rotate other due watches by oldest due time. Oversized batches reduce their future batch size; oversized discovery/check pages reduce their page size, with explicit limited states for irreducible responses. No membership cap of 50, 100 or 128 exists. Fifty is still the initial **real capacity validation target**, not a measured guarantee.
- Each batch reads current PR state, draft/title/review summary, head OID, authoritative potential-merge commit and separately paginated check contexts for each known commit. The graph includes Actions workflow run IDs/attempts, suite identity/state/conclusion, individual check states/conclusions and legacy statuses. No branch-name association, completion inferred from absence, log download or alert is introduced. Missing/unknown fields remain explicit rather than becoming success.
- Check contexts stage independently in 25-record remote pages; 100-record/byte-bounded IPC pages expose committed results. Source counts and repeated/changed cursors are checked before complete publication. Changing PR/head/merge generations reject obsolete pages. A newer head hides old contexts from the current-head response; retained old overview text is labeled with its original head and offers an explicit update. Partial GraphQL errors cannot turn denied checks into a successful empty set. Denial purges the denied check content while leaving independent authorized PR content and watch membership intact.
- Every content operation still uses installed gh with the original sanitized stored-auth context. REST reads use before/after stable-ID checks; GraphQL reads use a pre-read stable-ID probe plus the authenticated `viewer.databaseId/login` returned by the content query itself. A query executed using a different identity locks/rejects its content. This reduces probe overhead without trusting login text or a separate token client, and does not claim an impossible race-free external-account boundary.
- Core, Search and GraphQL budgets persist separately, recording request counts, bounded payload bytes, GraphQL cost, remaining quota and resets. Numeric/date Retry-After, secondary-limit cross-resource deadlines and exponential transport/limit backoff survive restart; manual refresh does not bypass them. A small primary-quota reserve leaves room for ordinary gh use. Cache revision events remain post-commit invalidation hints.
- Schema **2** transactionally upgrades schema 1 with normalized watch reasons/locators, persisted pending descriptors, team/source staging and revision-bound check storage. Pause, original identity, settings, seen/deduplication/attempt/progress ledgers and cached history remain intact. Previous-session manual jobs cannot restart automatic reads while paused. Future/corrupt schemas continue to stop without destructive reset.
- The native client now exposes Review requests, head/review/check state and provider/attempt metadata, source completeness, watch reasons/freshness and quota diagnostics. It preserves Stacks, bundled JetBrains Mono and the four-color themes. Every Tauri command response additionally enforces the serialized 1-MiB ceiling without clipping.

## Verification evidence

Environment remains the development host: Apple Silicon, macOS **27.0 / 26A428**, Xcode **27.0**, Rust **1.95.0**, gh **2.92.0**, Tauri **2.12.4**, app development version **0.1.0**, schema **2**, REST version **2026-03-10**. GraphQL fields were checked against the actual GitHub.com schema. This is not the physical M1/8-GiB/macOS-15 performance reference or a supported release OS matrix.

### Deterministic checks

`cargo test --workspace --offline` exercises the actual process runner/worker/SQLite boundary through a test-only executable in `tests/support/gh.py`, plus isolated store/API/IPC tests. Production never imports or falls back to that executable or its data. The suite now comprises 33 core tests and one desktop IPC boundary test.

Passing subcases include:

- Independent authored/direct/team reasons, overlapping reasons, selected close/reopen, request removal, team removal, denied team requests and preserved original identity.
- 151 open watches fully rotate through batches of at most ten; lists page at 100/51. A separate request-discovery test reconciles 1,001 PRs through non-overlapping Search partitions. These are **synthetic capacity/algorithm checks**, not live 50-watch qualification.
- Authored cursor pagination, staged/incomplete traversal preservation, duplicate/count/cursor checks, persistent due work, >128 pending descriptors, foreground/background fairness and primary-bucket isolation.
- 126 contexts on each of head and merge commits, replacement paging, mixed legacy statuses/check runs, workflow attempt changes, and current-head replacement. A generation test explicitly refuses an old-head page after a newer head has committed.
- Denied check fields remove cached check payloads without falsely publishing an empty complete result, dropping membership or globally locking the account.
- Authored/requested snapshot work continues after reader close. Pause stops automatic work, manual sync preserves Pause, and restart cannot replay old manual jobs while paused.
- Schema 1→2 preserves identity/Pause/settings/reasons; future/corrupt database preservation; durable clear semantics; independent budget/reset/exponential-backoff persistence; process timeout/overflow/cancellation/helper cleanup; actual >16-MiB batch overflow and adaptive 10→5→2 reduction without lost membership; inclusive/exceeded serialized 1-MiB IPC response bounds.

`npm run check` verifies TypeScript/frontend production assets and Clippy for the Rust workspace. The arm64 development `.app` is built using `npm run tauri build -- --debug --bundles app --target aarch64-apple-darwin`. These checks do not establish release signing or native performance acceptance.

### Authorized live runtime probe

Command: `cargo run -p gitjuke-core --example watch_probe --offline -- cli/cli 14719`, with network access permitted. The example uses a temporary database, real stored gh authentication and count/timing-only diagnostics; it deletes its temporary state on completion. No GitHub content, credentials, scopes, teams or account configuration are changed.

The latest complete probe recorded:

| Observation | Result |
| --- | --- |
| Discovery | Six sources complete: authored, direct, team membership and three team-request sources; 9,426 ms total |
| Active authored/requested watches | Zero; successful empty results, not inferred access failures |
| Selected public PR | Existing `cli/cli#14719`, already authorized and used by the previous slice |
| Check snapshot | 18 contexts; two commit-resource completeness records (head and potential merge); 17 contexts with Actions run/attempt metadata; 3,279 ms from selection |
| Subsequent automatic snapshot | Observed after 11,693 ms, including serialized gh/network work; eligibility target remains ten seconds, not a ten-second delivery guarantee |
| Selected-only close | Active watch count returned to zero |
| Core metrics | 21 requests, 79,697 response bytes, 4,852 remaining at final sample |
| Search metrics | Four requests, 5,572 response bytes, 26 remaining at final sample |
| GraphQL metrics | Three requests, 22,310 response bytes, measured total cost 3, 4,964 remaining |

The byte/request counters count recorded `gh api` responses; they do not claim wire-level accounting for gh internal redirects or its startup auth-status probe. Budgets are shared with ordinary gh use; remaining values and reset times are observations, not fixed limits. The fixture's 18 contexts were **CheckRun** records: Actions plus a non-workflow check run. No live `StatusContext`, independent external-CI or retry fixture is claimed. The apparent CheckSuite `headSha` field was rejected by the live schema probe before implementation; the implementation uses the supported `commit.oid` and verified `workflowRun.runAttempt` fields.

### Native end-to-end checks

CUA inspected the actual `tauri://localhost` WebView in the built application, not a browser fixture:

- Review requests showed the complete successful-empty set after all sources reconciled.
- Selecting the public PR added one active watch and rendered live head/review metadata and 18 check rows, including distinct SUCCESS/SKIPPED conclusions and run/attempt identifiers. Head and potential-merge completeness were shown separately. Later native inspections showed updated check timestamps while the description retained its explicit fetch timestamp.
- Closing the real native close button, **without reactivating the window**, was followed by read-only SQLite/process inspection: schema 2; zero selected reasons; remembered selection present; exactly one monitor process alive. Reopening restored the same PR/tab. Cmd-Q subsequently quit the verification app.
- The existing local application database actually migrated from schema 1 to 2, preserving the bound stored identity and appearance/Pause choices. Full signed installed-app/login/update qualification remains separate.

### Required evidence still open

Positive live authored, direct-requested and team-requested PRs, authorized private/fork/deleted-head cases, a published native stack, live legacy statuses/external-CI/retry attempts, access-denied source fixtures and actual 50-watch API/quota/latency measurements are **Blocked on suitable authorized fixtures**, not passed by the synthetic suite. No GitHub content or credentials were modified to obtain them.

Minimum/200%-zoom/VoiceOver, physical M1/macOS-15 performance/memory, signed artifact, login, notification delivery/click routing and full installed lifecycle remain Pending. The actual local close/reopen checks do not certify simultaneous login/manual/notification launch or closed-window cadence with 50 real watches.

## Next implementation work and unchanged limits

This slice adds watch discovery and scheduling. Saved queries/open-query scheduling, notification discovery and per-URL validators/X-Poll-Interval, individual activity hydration/classification, baseline/recovery summaries, durable alert intent/attempt dispatch, seen destinations, onboarding/permission controls, bounded diffs/discussions/logs and opt-in login remain future increments under the existing specification. Viewed-PR alerts must remain eligible when alerts are added. Discussion paging remains at **50 messages**, unchanged.

The first checkpoint's full retention/storage/migration-fault/performance/signing gates remain open: a shared budget accounting path and schema upgrade are not proof of the full 500-MiB/30-day/90-day, disk/write/checkpoint recovery and durable-growth contracts. There is no release exception or silent scope revision. All 29 grouped release scenarios stay Pending until their complete required subcases pass.

Primary sources checked: [GitHub team listing](https://docs.github.com/en/rest/teams/teams#list-teams-for-the-authenticated-user), [GraphQL checks](https://docs.github.com/en/graphql/reference/checks), [GraphQL workflow run attempt](https://docs.github.com/en/graphql/reference/actions), and the settled research/contract links in the acceptance matrix. No new external provider or signing credential was provisioned.

## Final local build identity

The final development executable is arm64 with Mach-O `minos 15.0` / SDK 27.0. Its SHA-256 is `3a374540abefc449ae8c1535cbdfbc12f776c41eec348977a4e76f20205ef3ff`. This fingerprints the local `.app` executable for evidence; it is not a signed DMG checksum or a distribution authorization. Final verification: **34 workspace tests pass**, `npm run check` passes, and the arm64 development `.app` build succeeds. The verification app was explicitly quit.
