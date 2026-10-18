# GitHub PR saved views through gh

Research date: 2026-10-17. Scope: one GitHub.com account; read-only client; GitHub access only through `gh`.

## Finding

**Do not promise automatic synchronization of the user's real GitHub PR dashboard presets yet.** GitHub calls these **saved views**. They are a real, generally available dashboard feature. This investigation found **no documented gh command or public REST/GraphQL discovery operation** for that personal saved-view collection. This is a supported-interface finding, not proof that no internal endpoint exists. Live authenticated API validation was unavailable in this environment.

The smallest viable fallback is **local saved queries**, with user-supplied names and query text copied from GitHub. Label these as local views, not synchronized GitHub presets. Preserve an optional original dashboard URL for “Open in GitHub”; a durable saved-view URL must not be assumed to encode its query.

## What the user means

GitHub's [July 9, 2026 PR dashboard announcement](https://github.blog/changelog/2026-07-09-new-pull-requests-dashboard-is-now-generally-available/) says that the refreshed `github.com/pulls` dashboard supports organizing named saved queries. It also describes inbox sections and built-in personal filters. These are the relevant presets.

Do not confuse them with:

- **Issues dashboard saved views**, which can also contain PR searches using `is:pr`, and have their own creation/editing workflow. [Issues dashboard documentation](https://docs.github.com/en/issues/tracking-your-work-with-issues/using-issues/viewing-all-of-your-issues-and-pull-requests).
- **Project views**, which are tabs over a particular project's items with filters, layout, grouping, and ordering. They are not a personal cross-repository PR dashboard. [Projects view documentation](https://docs.github.com/en/issues/planning-and-tracking-with-projects/customizing-views-in-your-project/managing-your-views).
- Browser bookmarks or local `gh` aliases, which may hold a query but are not the GitHub saved-view collection.

GitHub explicitly made saved PR view URLs durable across edits in its [April 23 dashboard announcement](https://github.blog/changelog/2026-04-23-global-pull-requests-dashboard-moves-to-opt-out-public-preview/). Consequently a pasted URL alone is insufficient evidence that the app can extract the saved filter.

## Supported surface checked

| Surface | Evidence and conclusion |
| --- | --- |
| Installed CLI | `gh version 2.92.0 (2026-04-28)`. `gh pr --help` has list, status, view, checks, and diff commands; no saved-view/preset command. `gh search prs --help` exposes query execution and JSON fields, not preset discovery. |
| Current CLI manual | [`gh search prs`](https://cli.github.com/manual/gh_search_prs) documents raw search qualifiers, flags, and JSON output. It does not document saved-view discovery. |
| Generic API transport | [`gh api`](https://cli.github.com/manual/gh_api) can call REST and GraphQL using the active CLI credentials. That transports an existing supported API; it does not itself provide access to the website's internal saved-view store. |
| REST | Public [pull request](https://docs.github.com/en/rest/pulls) and [search](https://docs.github.com/en/rest/search/search#search-issues-and-pull-requests) references document PR data and query execution. No saved PR dashboard view discovery operation was identified in this research. |
| GraphQL | The current public reference is grouped by category. The reviewed [pull request](https://docs.github.com/en/graphql/reference/pulls), [search](https://docs.github.com/en/graphql/reference/search), and [user](https://docs.github.com/en/graphql/reference/users) references expose no identified saved PR dashboard view connection. `savedReplies` on User is unrelated. This was a targeted documentation review, not an exhaustive downloaded-schema proof. |

## Query execution and auth

A local view can execute a query through `gh search prs` or `gh api --method GET search/issues -f q=...`, then hydrate selected PRs separately. Use an argument array in Rust, never shell interpolation. Distinguish an unsupported or invalid query from an empty result set.

The [CLI API manual](https://cli.github.com/manual/gh_api) documents authentication from `GH_TOKEN`/`GITHUB_TOKEN` or the configured account. Repository access still depends on that account/token. No additional OAuth scope or permission that unlocks personal PR saved-view discovery was established; do not request extra permissions speculatively.

Dashboard/API query parity needs a fixture-based check before claiming full fidelity. GitHub's July announcement introduces `team-review-requested-user:@me` and `review-involves:@me` in the dashboard. Their presence in that announcement does **not** verify that the installed CLI and public Search API evaluate them identically. The installed CLI accepts raw query text but cannot guarantee backend support. The current manual also has advanced-query handling details newer than this installed binary, so test the exact version or use explicit REST transport.

## Local validation and limitations

Read-only commands run: `gh --version`, `gh pr --help`, `gh search prs --help`, `gh api --help`, a credential status check with identifying output suppressed, and this schema probe:

```sh
gh api graphql -f 'query={__schema{types{name}}}' \
  --jq '[.data.__schema.types[].name | select(test("Saved|Dashboard|View"))]'
```

The schema probe failed to connect to `api.github.com`; it returned **no schema evidence**. The local auth status check indicated a problem, so active account access was not validated. No private PRs, saved queries, tokens, account names, or remote state were output or changed. GitHub's public downloadable schema was located via [Public schema](https://docs.github.com/en/graphql/overview/public-schema), but the browser tool could not consume its octet-stream download. No browser cookies, HTML scraping, undocumented requests, or internal GraphQL operations were attempted.

## Decision implication

Keep native saved-view synchronization as an explicit feasibility gate. A later successful supported API probe can enable it; absent that, offer a small local query list and an honest explanation during view setup. Local query creation is local configuration and fits a read-only GitHub client. It does not fulfill automatic retrieval of existing GitHub presets, so this tradeoff requires the product owner's decision before implementation scope is locked.
