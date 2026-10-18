# Personal activity coverage and realistic polling latency

Research date: 2026-10-17. Scope: one GitHub.com account; local Rust backend; all GitHub access through installed `gh`; read-only operation. This is a feasibility finding and proposed architecture, not a user-approved freshness contract. No authenticated network requests or private account data were needed.

## Result

A useful personal PR monitor is feasible through `gh api`. Universal delivery within ten seconds is **not** supportable under the documented API contract. GitHub's notifications endpoint explicitly supplies a minimum polling interval; a local ten-second timer must not override it. There is no documented maximum event-publication latency for notifications or search. Ten-second **refresh targets** are plausible for bounded, already-known PR state and running checks, while awake, online, and within quota. They do not guarantee ten-second event detection.

The CLI itself offers `gh pr checks --watch` with a default ten-second interval. This supports the narrower checks-refresh target, rather than a promise about every personal event. [CLI checks manual](https://cli.github.com/manual/gh_pr_checks)

## Notification discovery: broad but thread based

`GET /notifications` returns updated discussion threads, not individual events. `review_requested` covers a user or one of their teams; `mention` and `team_mention` identify mention subscriptions; `author` represents threads the user created. Reasons can change and remain sticky, so a reason cannot identify the newest action reliably. Hydrate a changed PR before saying “new review” or naming a commenter. Authentication needs `notifications` or `repo`; fine-grained PATs and GitHub App tokens are unsupported here. Existing `gh` credentials need a capability check. [Notification API](https://docs.github.com/en/rest/activity/notifications)

Polling must obey `X-Poll-Interval` (the documentation illustrates 60 seconds and allows increases). Preserve `Last-Modified` exactly and send `If-Modified-Since`; unchanged responses are 304. Parameters include `all`, `participating`, `since`, and `before`; pages contain at most 50 threads. Proposed discovery uses `all=true` so activity read elsewhere is still considered, with local relevance filtering rather than relying solely on `participating=true`. [Notification API](https://docs.github.com/en/rest/activity/notifications)

Recommendation: do not change GitHub notification read/done state. Local seen state and OS notification deduplication are separate from GitHub's unread flag. Do not label a sticky `mention` reason as a new mention without inspecting content.

## Coverage by requested event

| Need | Discovery and verification | Important gap |
| --- | --- | --- |
| Comments on my PR | Notification discovery, then issue comments and inline review comments | Notification thread updates can coalesce several actions. Read actual comments to recover author, body, timestamp, link, and IDs. |
| Reviews on my PR | Notification discovery, then reviews and their comments | A review is a collection of inline comments plus state/body; approval without a textual comment is still a review. |
| Direct/team review request | Notifications plus searches and current requested-reviewer state | Request removal/re-request and reassignment need state reconciliation, not only notification reasons. |
| Mentions on another PR | Notifications plus changed thread hydration | Watching/subscription settings and retained thread reasons limit interpretation; no complete durable event feed is promised. |
| Checks related to my PR | Poll current head SHA's checks and statuses; reconcile on head changes | CI activity notifications alone are insufficient. Third-party CI may report a status/check without being a GitHub Actions workflow. |
| Workflow completion | Match workflow runs using repository, SHA, run ID/attempt, and PR association | A run need not map to the PR head, especially PR merge refs; do not match only by branch name. |

General discussion comments are provided by the issues comments endpoint; PR inline review comments and reviews have separate endpoints. [Issue comments](https://docs.github.com/en/rest/issues/comments), [Pull request API index](https://docs.github.com/en/rest/pulls), [Reviews](https://docs.github.com/en/rest/pulls/reviews)

Search provides `author:`, `mentions:`, `involves:`, direct `user-review-requested:@me`, and `team-review-requested:org/team` qualifiers. These help reconcile sets of PRs; they do not constitute a real-time change stream. Discover team membership and reconcile team requests explicitly if needed rather than assuming a direct-only query includes them. [Search/filter documentation](https://docs.github.com/en/issues/tracking-your-work-with-issues/using-issues/filtering-and-searching-issues-and-pull-requests?tool=webui)

GitHub Actions completion notifications depend on notification preferences and runs the user triggered, with special scheduled-workflow rules; they do not guarantee every pipeline attached to an authored PR. [Workflow run notifications](https://docs.github.com/en/actions/concepts/workflows-and-actions/notifications-for-workflow-runs), [Actions notification settings](https://docs.github.com/en/subscriptions-and-notifications/how-tos/managing-github-actions-notifications)

Read current check runs and combined commit status independently. Workflow-run APIs add run-level identity, attempts, status, conclusion, and filtering. [Check runs](https://docs.github.com/en/rest/checks/runs#list-check-runs-for-a-git-reference), [Combined status](https://docs.github.com/en/rest/commits/statuses#get-the-combined-status-for-a-specific-reference), [Workflow runs](https://docs.github.com/en/rest/actions/workflow-runs#list-workflow-runs-for-a-repository)

Do not substitute the Events API: GitHub explicitly documents latency from 30 seconds to 6 hours, a maximum 300-event timeline covering only the last 30 days, and its own polling header. Received events concern watched repositories/followed users, which is different from personal PR involvement. [Events API](https://docs.github.com/en/rest/activity/events)

## Request budgets and payload size

Ordinary authenticated REST access generally has a 5,000-request hourly primary budget shared with other requests on behalf of the user. Search has separate restrictive buckets; read response `x-ratelimit-resource`, remaining, and reset headers instead of spending the core budget as if all endpoints were equivalent. Secondary controls include concurrency, approximately 900 REST points/minute and 2,000 GraphQL points/minute, CPU usage, and undisclosed controls. Published thresholds are ceilings, not permission to poll at maximum intensity. [REST limits](https://docs.github.com/en/rest/using-the-rest-api/rate-limits-for-the-rest-api)

Authenticated conditional GET responses returning 304 do not consume the **primary** limit. This is not a promise of exemption from secondary controls. Keep exact URL/parameter variants stable and cache validators per variant. Serialize requests; honor `Retry-After`, primary reset, and exponential retry backoff. [REST best practices](https://docs.github.com/en/rest/using-the-rest-api/best-practices-for-using-the-rest-api)

GraphQL has a separate ordinary user budget of 5,000 points/hour, minimum query cost 1; include `rateLimit { cost remaining resetAt }` when evaluating a proposed batch. Connections must request 1–100 items and a query cannot request over 500,000 nodes. Deep comments × reviews × checks fan-outs can become expensive; small scalar snapshots plus lazy detail loading are preferable. GraphQL HTTP 200 can contain rate-limit/query errors, so parse the response body as well as status. [GraphQL limits](https://docs.github.com/en/graphql/overview/rate-limits-and-query-limits-for-the-graphql-api)

Illustrative arithmetic, not measured query costs:

| Pattern | Hourly requests or points, assuming continuous operation |
| --- | ---: |
| One request every 10s | 360 |
| One request every 60s | 60 |
| 20 PRs × 3 REST endpoints × every 10s | 21,600 requests before conditional savings |
| One GraphQL batch every 10s at measured cost 1 / 5 / 15 | 360 / 1,800 / 5,400 points |
| Notifications every 60s + one cost-1 GraphQL batch every 10s | Up to 60 REST requests + 360 GraphQL points, before detail/pagination work |

The last row assumes the server actually permits 60 seconds, the batch cost is measured as 1, and the PR set is complete in its pages. It is a design target to validate, not an assertion about the final implementation. A global all-PR ten-second REST fan-out is unsuitable even if quiet-time 304 rates are high.

## Proposed `gh` transport and scheduler

`gh api` supports REST/GraphQL, custom headers, response headers/status via `--include`, explicit method, stdin request bodies, and pagination. `--jq` and `--template` select values from the response on the client; they do **not** reduce downloaded REST response bytes. GraphQL selection sets do shape remote response content. `--cache` is a duration cache, not a replacement for app-owned freshness/validator policy. [CLI API manual](https://cli.github.com/manual/gh_api)

Implementation recommendations (inferences from the API contracts):

1. One backend scheduler owns a serialized request queue. React and tray UI subscribe to the same cached state; closing the window leaves that backend alive.
2. Invoke `gh` with argument arrays, never a shell command composed from a PR title, query, or URL. Use explicit `--hostname github.com`, explicit repo identities, bounded output/timeouts, and no interactive prompting. Keep tokens inside `gh`; do not print authorization/debug output.
3. Use `gh api --include` with app-owned REST validators. Parse HTTP headers separately from JSON and handle 304 as a valid unchanged outcome even if a CLI version reports unusual exit behavior. Verify this adapter with a small fixture/probe before implementation; CLI manual does not specify all 304 exit semantics.
4. A ten-second scheduling tick refreshes bounded active/known PR scalar state and nonterminal checks if due, header-permitted, and budget-safe. Notifications run on their own deadline at `max(local target, X-Poll-Interval)`. Slow or postpone lower-value reconciliation when budget shrinks. Never overlap a slow previous poll.
5. Changed snapshots schedule detail jobs. Fetch diffs/comments/review threads only when opened or when needed to classify a notification. Paginate nested collections fully when claiming completeness; a page size limit is never a completeness guarantee.
6. Discovery searches reconcile authored, requested, team-requested, mentioned/involved PR sets on a separate bounded schedule. Exact cadence remains a product decision; ten-second search does not guarantee ten-second index visibility.
7. Persist successful watermarks, validators, known PR IDs/head SHAs, and event keys. For notification pagination, keep a fixed catch-up window and advance its watermark only after every page succeeds. Overlap windows to handle timestamp boundaries; deduplicate by stable source IDs plus versions/timestamps. A moving `since` URL needs its own validators, so do not reuse validators across changed queries.
8. After sleep/offline, perform one bounded catch-up and state reconciliation, not thousands of missed ticks. Present backlog in-app and coalesce old OS alerts. Initial sync should establish baseline rather than notify for every historical item. Recover retryable failures with backoff and preserve previous data with an accurate stale indicator.

Process launch/auth/network overhead must be measured on the target Mac; no benchmark was run here. At six small subprocesses per minute a single bounded batch appears practical, but one process per endpoint per PR scales poorly. `gh` browser login normally stores credentials in the OS credential store and also supports environment-supplied credentials; validate the active account/capabilities rather than assuming the user's environment matches the configured login. [CLI authentication manual](https://cli.github.com/manual/gh_auth_login)

## Decisions still needed

- Accept a two-speed contract: ten-second target for bounded known PR state, server-directed discovery latency for notifications, no universal guarantee.
- Decide background watched-set size and overload behavior. “Every PR involving me forever at ten seconds” is materially different from active authored/requested PRs plus notification discovery.
- Specify local seen/badge semantics, initial sync and wake notification behavior, self/bot filtering, and OS alert grouping.
- Validate token compatibility and current `gh` 304 behavior using a minimal explicit probe during implementation, without copying private payloads into logs.
