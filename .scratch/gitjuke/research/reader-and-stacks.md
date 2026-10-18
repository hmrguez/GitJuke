# PR reader and stack data through gh

Researched 2026-10-17. Primary sources only. This is a documentation feasibility investigation; no authenticated account data or live private repositories were queried. Recommendations below are design inferences, not implementation validation.

## Conclusion

The read-only PR client is feasible entirely through the `gh` executable, using `gh api` for a precise, paginated transport and selected high-level commands for convenience. Native GitHub stacks should be the primary source of stack relationships. Branch chaining remains useful as a visibly inferred fallback; it must not be presented as authoritative stack membership. Logs and large diffs need explicit unavailable/partial states.

`gh api` supports authenticated REST and GraphQL, request headers, explicit methods, response headers, and pagination. Fields normally switch its method to POST, so REST reads with fields must explicitly set GET. GraphQL requests use POST but can contain read-only queries. REST pagination follows pages; GraphQL pagination needs an `$endCursor` variable plus `pageInfo`. Nested connections need their own independent pagination, rather than assuming one global cursor completes them all. [gh api manual](https://cli.github.com/manual/gh_api)

## Reading surface

| UI need | Recommended gh surface | Important behavior |
| --- | --- | --- |
| Account-wide PR lists | `gh search prs '<query>' --json ... --limit N`, or a GraphQL search query through `gh api` | Search supports author, mentions, reviewer/team, repository, state, and update filters. Set an explicit limit; default is 30. |
| Repository PR lists | `gh pr list -R OWNER/REPO --json ... --limit N`, or REST `GET /repos/OWNER/REPO/pulls` | Default list is open PRs and 30 entries. Keep list fields small. |
| Selected PR header/body | `gh pr view N -R OWNER/REPO --json ...`, or GraphQL `repository.pullRequest` | JSON covers author, body, refs, draft/state, labels, review requests, review decision and checks. Native stack fields require direct API queries. |
| Changes | `gh pr diff N -R OWNER/REPO --color never`; paginated REST `/pulls/N/files` for structured filenames/patches | Raw diff is text, not JSON. `--name-only` and `--patch` are also available. |
| General conversation | REST `/repos/OWNER/REPO/issues/N/comments` | PRs are also issues for general comments; don't confuse these with line review comments. |
| Review summaries | REST `/repos/OWNER/REPO/pulls/N/reviews` | Keep each submission's author, body, state, timestamp and reviewed commit. |
| Inline discussion | GraphQL `reviewThreads`, with independently paginated thread comments | Retain resolved/outdated status and original/current coordinates. |
| Checks | `gh pr checks N --json ...`, or head status rollup with all contexts paginated | Preserve individual check identities/statuses and link to provider details. |
| Actions jobs/logs | `gh run view RUN_ID -R OWNER/REPO --json ...`; `--log` or `--log-failed` on demand | Don't download logs in background polling. Non-Actions providers can require opening their details URL. |

Sources for command surfaces: [gh search prs](https://cli.github.com/manual/gh_search_prs), [gh pr list](https://cli.github.com/manual/gh_pr_list), [gh pr view](https://cli.github.com/manual/gh_pr_view), [gh pr diff](https://cli.github.com/manual/gh_pr_diff), [gh pr checks](https://cli.github.com/manual/gh_pr_checks), [gh run view](https://cli.github.com/manual/gh_run_view).

General comments have IDs, bodies, authors and update timestamps; list comments accepts `since` and pagination (default 30, maximum 100 per page). Use an overlap when incrementally reading and deduplicate by ID/update timestamp; deletion requires periodic reconciliation. [Issue comments REST](https://docs.github.com/en/rest/issues/comments)

Review submissions are separate resources from comments and must be read separately to include approvals, changes requested and dismissed review state. [Reviews REST](https://docs.github.com/en/rest/pulls/reviews)

REST review comments include `pull_request_review_id`, `in_reply_to_id`, path, diff hunk, original/current commit IDs, line/start-line and side. Keep this context rather than attempting to place an outdated discussion against today's line number. [Review comments REST](https://docs.github.com/en/rest/pulls/comments)

GraphQL provides `reviewThreads`, thread `isResolved`, `isOutdated`, current/original line context, and connection `pageInfo`/`totalCount`. It also exposes `stack`/`stackEntry`; a stack has id, repository-local number, size, base ref and paginated entries, each with an authoritative position starting at 1 nearest the base. `statusCheckRollup` is tied to the PR head ref. Query only needed fields and follow every connection, including comments inside each thread. [GraphQL pull request types](https://docs.github.com/en/graphql/reference/pulls)

## Pagination, large changes, and permission boundaries

- The current CLI source constructs `first:100` fields for files, commits, comments, reviews, review requests and check contexts. Some contain pagination metadata and other CLI paths may paginate further; these JSON fields should **not** become the application's completeness contract. Use app-controlled API pagination for complete reader panels. Installed CLI behavior can differ from trunk. [CLI query builder source](https://github.com/cli/cli/blob/trunk/api/query_builder.go)
- Search is capped at 1,000 results per query and may return `incomplete_results` after a timeout. Surface incomplete data; narrower date/repo partitions can recover larger result sets. Search is not an event stream. [REST search](https://docs.github.com/en/rest/search/search)
- REST PR files are paginated, default 30 per page, and capped at 3,000 files. PR GET supports raw diff/patch representations, and retrieval can return 406/500/503. Distinguish successful empty diff from missing/unavailable content; preserve GitHub links and compare listed count with PR `changed_files`. The endpoints document Pull requests read permission for fine-grained tokens. [PR REST](https://docs.github.com/en/rest/pulls/pulls)
- Check runs and legacy commit statuses are different kinds of context; don't model only Actions workflow runs. On forks, the Checks API can return an empty `pull_requests` association array. Join using the PR's actual head SHA and available workflow/check relationships, never branch name alone. Check-run listing also has special handling beyond 1,000 suites. [Check runs REST](https://docs.github.com/en/rest/checks/runs)
- Actions job metadata/logs require repository read access; private-repository classic/OAuth tokens need `repo`, and fine-grained tokens need Actions read. Job-log download redirects expire after one minute. Metadata and log permissions must be handled independently from PR access. [Workflow jobs REST](https://docs.github.com/en/rest/actions/workflow-jobs)
- CLI log extraction can fail when more than 25 job logs are missing from its main ZIP-based method; some lines appear as `UNKNOWN STEP`. Store useful partial output with an explanatory state and open-in-GitHub action. [gh run view limitations](https://cli.github.com/manual/gh_run_view)

Design inference: represent `complete`, `hasMore`, `truncated`, `unavailable`, and `stale` separately, alongside `fetchedAt`, head SHA/revision and endpoint error. A successful request returning no data is not equivalent to missing permission. Treat deleted fork/head repository fields as nullable; PR identity is host + base repository + number (and durable node ID when available).

## Native stacks: supported now

GitHub announced stacked PR general availability on **2026-10-06**, available on all GitHub.com plans. The announcement says Enterprise Server follows in an upcoming release. Auto-merge has a separate gradual rollout; that does not prevent read-only stack navigation. Older preview references should not override this newer announcement. [GA announcement](https://github.blog/changelog/2026-10-06-stacked-pull-requests-generally-available/)

GitHub documents read support through REST and GraphQL: PR REST resources carry `stack` membership, while the GraphQL PullRequest has read-only `stack` and `stackEntry`. No extension installation is necessary to invoke these APIs with `gh api`; the `gh stack` extension serves the local development workflow. [Stack API overview](https://docs.github.com/en/pull-requests/reference/stacked-pull-requests-apis-and-webhooks)

Read endpoints:

```text
GET /repos/{owner}/{repo}/stacks?pull_request={number}&per_page=100
GET /repos/{owner}/{repo}/stacks/{stack_number}
```

The list filter locates the stack containing a PR. Responses identify stack, base and members; GET provides detailed member PRs. Fine-grained access requires Pull requests read; public resources can be read without authentication. Defaults are 30 stacks/page, max 100. Current examples specify `X-GitHub-Api-Version: 2026-03-10`; pin a documented supported version and test it rather than using older version examples by habit. This research has not live-probed these endpoints, so implementation acceptance must include one public or authorized fixture and graceful capability/error handling. [Stacks REST reference](https://docs.github.com/en/rest/pulls/stacks)

Native stacks require branches in one repository; cross-fork stacks are unsupported. Trunk can be a release or other branch, not just default/main. Required reviews/checks and Actions evaluate against stack trunk. A branch chain alone should not be used to reproduce GitHub's mergeability decision. [Stack requirements](https://docs.github.com/en/pull-requests/reference/stacked-pull-requests)

## Inferred and tool-specific relationships

GitHub itself distinguishes an existing base/head chain from a native stack: it offers to turn a chain into a stack. Therefore equal base/head refs are evidence for a relationship, not proof of native membership. [Creating stacked PRs](https://docs.github.com/en/pull-requests/how-tos/create-pull-requests/creating-stacked-pull-requests)

Recommended fallback algorithm (design inference): from accessible PRs in the base repository, connect child.base repository ID + ref to parent.head repository ID + ref; require a unique candidate, reject self-edges/cycles, and keep branching graphs as graphs. Do not join by unqualified ref names or assume all authored PRs are the complete repository graph. A partial/open-only snapshot can miss merged/closed parents. Do not infer topology from overlapping diffs alone.

Tool-generated PR-body links can be shown as related PRs, but are mutable content and may be stale. This research did not establish a universal public Graphite/other-tool stack metadata contract. Avoid silently treating arbitrary footer text as authoritative. Add adapters only against a documented tool contract or explicit supported fixtures later.

Recommended model (design inference): `Relationship { source: native | inferred_branch | tool_metadata, parentPr, childPr, evidence, fetchedAt, completeness }`; native stack object has separate identity, trunk, positions and member state. Keep unknown membership distinct from confirmed none. Present native navigation simply as “Stack”; fallback uses “Related branches” or a small “Inferred” label with evidence available on demand. Never upgrade inference to native status in cache.

## Next acceptance evidence

Before production implementation, exercise public fixtures with >100 files/comments/checks, renamed/deleted heads, fork PRs, resolved and outdated review threads, native stacks and an unconverted branch chain; test unavailable Actions logs and denied access. Verify exact installed `gh` JSON behavior, the pinned API version and native stack schema. These are verification tasks, not claims this report has already tested.

## User's stack provider and local environment

The user clarified their tool is `gh stack`. A safe local inspection on 2026-10-17 found `gh version 2.92.0 (2026-04-28)`. `gh stack --help` says it is an official extension installed from `github/gh-stack`; the local extension inventory does **not** currently contain a stack extension. Thus the current workstation does not establish the user's exact installed stack implementation/version on another machine. No installation or authentication changes were made.

The official `github/gh-stack` extension submits PRs into native GitHub stacks. It keeps additional unpublished local tracking in `<git-common-dir>/gh-stack`, shared between worktrees. `gh stack view --json` exposes current local stack data. This is useful for local/unsubmitted branches but unnecessary for account-wide remote PR navigation, which should read the native Stack API. Local navigation commands (`up`, `down`, checkout, etc.) switch branches and are outside this client's read-only scope. Name collisions with historical third-party `gh-stack` extensions must be handled by checking extension repository identity; the command spelling alone does not prove official-provider semantics. [Official extension repository](https://github.com/github/gh-stack)

Recommendation: scope v1 to **published native stacks created through official gh stack**, using `gh api`; no local clone, extension dependency, or branch checkout is necessary. If the user's existing stacks were produced by a legacy third-party extension, verify a sample PR's native membership before claiming equivalent support. A future explicit local-project integration may read official `gh stack view --json` behind a version/identity check, without mutating catalogs or running sync.
