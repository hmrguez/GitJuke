# Choose notification defaults and the active watch set

Labels: wayfinder:grilling
Type: grilling
Status: resolved
Assignee: hmrguez
Parent: ../map.md
Blocked by: 02

## Question

Which PRs belong to the active ten-second watch set, what capacity/overload behavior is acceptable, and which personal actions should generate native alerts? Review the concrete defaults in the specification: direct/team requests, comment/review/mention coverage, self/bot suppression, grouping terminal checks, first-sync baseline, wake catch-up, preview privacy, and local seen state distinct from GitHub read state.

Freshness cadence and the coverage decisions below are approved. Remaining defaults require human review.

## Comments

### Review round prepared — 2026-10-17

The user explicitly prioritized this ticket. Previously approved ten-second PR/check refresh and X-Poll-Interval notification discovery stay fixed. No detailed defaults are approved by the continuation request.

Keep separate: active watch membership (refresh policy), personal activity (reader feed), and native alerts (OS interruption policy). Membership alone does not authorize an alert. Mention discovery is not a ten-second promise, and selecting a PR does not automatically subscribe to every future event.

First round proposals awaiting the human:

1. Watch all open authored PRs, open direct/team review requests, and the currently selected open PR; deduplicate identities. Saved-query results and unselected stack neighbors do not automatically join. Team requests are the explicit breadth trade-off. Window close preserves authored/request watches; selected-only membership lifetime and capacity handling need the next round.
2. Enable alerts for other people's comments/reviews on authored PRs, direct/team review requests, mentions elsewhere, and terminal checks on authored PRs; suppress own actions and routine bot comments, with per-category controls. Alternative: direct requests and mentions only. Human must choose interruption level; grouping, baseline/catch-up, previews, local seen semantics, and exact capacity/overload rules remain pending afterward.
3. Separately ask whether Stacks should be the default layout. Record its answer in the prototype ticket; do not silently resolve that ticket while working this one.

No resolution yet. Runtime/cache choices follow the settled policy. Do not begin production implementation.

### Confirmed coverage — 2026-10-17

The user accepted all three recommendations in the preceding review round:

- Active watch membership: open authored PRs, open direct/team review requests, and the selected open PR, deduplicated. Saved-query results and unselected stack neighbors do not automatically join.
- Full personal-activity native alert coverage: other people's comments/reviews on authored PRs, direct/team review requests, mentions elsewhere, and terminal checks on authored PRs. Suppress own actions and routine bot comments; provide category toggles. Suppression governs alerts, not visibility in the reader.
- Stacks is the default layout; that decision is recorded in the prototype ticket.

This is a partial decision record, not resolution of the entire ticket. Capacity, background membership lifetime, grouping, recovery, previews, and seen-state details remain open.

### Capacity and background proposals — awaiting human answer

- Treat 50 simultaneous active PRs as an initial validation target, not a hard product cap or proven capacity. Retain all eligible watches. Under budget pressure prioritize the visible selected PR, rotate the remaining watches fairly, and show delayed freshness/last refresh instead of silently dropping entries or implying every PR still meets ten seconds. No automatic eviction solely for age.
- Keep authored/request watches on the same ten-second target with the window hidden, unfocused, or closed while the app is awake, online, and within budget. Selected-only membership lasts while a reader window is open, including minimized/unfocused; close removes that reason without removing authored/request reasons. Preserve selection locally for reopening, but do not keep it watched solely because it was last selected. Quit stops monitoring. Permission denial for OS alerts does not stop monitoring.
- When successful reconciliation confirms a PR closed/merged or its review request removed, remove that membership reason and retain cached/activity history; overlapping reasons still apply. Never infer removal from failed or partial discovery. A transition observed in the final snapshot can still create eligible activity; removal does not fabricate terminal check completion.

Discovery cadence, retention duration, scheduler budgets, and exact degraded-refresh mechanics remain runtime/cache questions after these product decisions.

### Confirmed capacity and background behavior — 2026-10-17

The user explicitly accepted all three capacity/background recommendations:

- No hard PR cap. Fifty simultaneous active watches is the initial validation target, not demonstrated capacity. Retain eligible watches; under budget pressure prioritize the visible selection, rotate others fairly, and expose delayed freshness. No silent dropping or automatic age eviction.
- Authored and direct/team-request watches continue at the same refresh target with the window closed. Selected-only membership ends on close, but survives minimizing or switching apps. Remember selection for reopening. OS notification permission denial does not stop monitoring; explicit Quit does.
- Remove a membership reason after successful reconciliation confirms closure/merge or removal of that review request; other reasons still apply. Retain cached/activity history. Failed or incomplete discovery never proves removal.

Ticket remains claimed pending the remaining notification policies.

### Grouping, recovery, and privacy review — proposals only

1. Normal alerts: group newly eligible activity for the same PR within a completed reconciliation batch into one native alert; retain individual activity records. Do not add a deliberate batching delay beyond normal polling. Coalesce job results into one terminal alert per workflow/check-suite attempt; keep independent providers/runs distinct, while grouping those that finish in the same PR batch. A retry attempt can alert again; the same attempt cannot. Superseded-head completions do not generate new native alerts. Do not wait for every workflow on a PR to finish before reporting a completed run, and never infer completion from missing checks.
2. First complete sync establishes a baseline with no historical OS alerts. After wake, offline recovery, or restart with an existing baseline, reconcile missed activity into one catch-up summary across PRs instead of replaying individual alerts. Clicking the summary opens Activity. Include only newly discovered eligible activity since persisted successful progress; keep partial recovery visibly incomplete and avoid a summary claiming completeness until the reconciliation succeeds. Use a recovery-start boundary to separate recovered older activity from activity that occurs afterward; post-boundary activity resumes ordinary grouping once discovered. GitHub discovery intervals still apply; no universal instant catch-up promise. Exact persistent recovery mechanics belong in runtime/cache review.
3. Default native alert contents: generic category/count only, with no repository names, PR numbers/titles, actor names, branch names, comment bodies, or log excerpts. Applies to every PR, even when its visibility is unknown. Offer explicit opt-in metadata previews (repo/PR title/actor); leave body/log excerpts out of v1 native alerts. Clicking opens the appropriate local reader or Activity summary. These are OS-preview choices, not a claim that the local cache is encrypted or that macOS ignores its own preview settings.

Still to settle after this round: local seen semantics, alerts while already reading, permission/onboarding behavior, and Pause/Resume semantics. No new ticket is resolved by these proposals.

### Confirmed grouping, catch-up, and privacy — 2026-10-17

The user accepted all three recommendations from the preceding round:

- Group normal eligible activity per PR within the same refresh batch, without an added waiting period; retain individual activity records. Checks alert once per workflow/check-suite attempt rather than per job; retries may alert again, superseded-head completions stay quiet, and reporting a completed run does not wait for all workflows.
- First complete sync establishes a baseline without historical native alerts. Wake/offline/restart recovery with an existing baseline produces one cross-PR catch-up summary opening Activity, not individual replay. Show partial recovery honestly; post-recovery-start activity follows normal grouping when discovered.
- Default native alert text is generic category/count only, with opt-in repo/PR-title/actor previews. No comment bodies or log excerpts in version-one native alerts.

The preceding proposal section supplies the detailed interpretation of these recommendations. Local seen state, foreground alerts, permission/onboarding, and Pause/Resume remain unresolved.

### Local seen state, foreground alerts, and onboarding — proposals only

1. Seen state belongs to individual activity records and persists locally. Explicitly opening an activity marks that item seen only after its destination renders successfully. Merely selecting its PR, refreshing, delivering/dismissing an OS alert, or reading on GitHub does not clear it. New arrivals remain unseen until opened or explicitly marked seen. Provide Mark all seen for the currently loaded Activity set, using a snapshot so later arrivals are unaffected; label partial scope honestly. First-sync historical activity starts seen. Notification clicks follow the same successful-destination rule; catch-up summary clicks open Activity without clearing all items. Never change GitHub read/done state.
2. Suppress native alerts for the selected PR while the reader window is focused and visible, including when a different tab of that PR is open. Keep new activity unseen and show in-app count/tab indicators. Continue native alerts for other PRs; restore normal policy when minimized, unfocused, or closed. Do not replay foreground-suppressed alerts when focus changes. Aggregate/catch-up alerts include only still-eligible unsuppressed activity.
3. Offer Enable notifications during first-run setup, with a plain explanation of coverage, generic previews, and close-to-menu-bar behavior; allow skipping. Request macOS permission only after explicit enabling. If denied, keep monitoring/Activity working, expose status and an Open System Settings action, and do not repeatedly request permission. Enabling or re-enabling alerts starts future delivery without replaying previously accumulated activity; local unseen state remains intact. Launch at login is a separate opt-in, default off. Check installed gh/account/read access and show actionable instructions, without changing credentials, scopes, account, or installing software automatically. Record onboarding/alert choices locally.

These are product recommendations awaiting the user; no native behavior has been implemented or verified. Pause/Resume semantics follow after this round.

### Confirmed seen state, foreground alerts, and onboarding — 2026-10-17

The user accepted the local seen-state and permission/onboarding recommendations from the preceding round, and explicitly rejected foreground suppression:

- Explicitly opening activity marks that individual item seen after its destination renders. PR selection and OS delivery/dismissal do not clear activity. Mark all seen applies to the currently loaded snapshot; later arrivals stay unseen. First-sync historical items start seen. GitHub read state is never changed.
- Alert for all eligible PRs, INCLUDING the selected/current PR while the reader is focused and visible. Window focus, selection and reader tab do not suppress eligible native alerts. Existing own-action/bot/category/deduplication/privacy rules still apply. This supersedes the foreground-suppression proposal above.
- Skippable Enable notifications onboarding explains coverage, generic previews and background monitoring. Request macOS permission only after enabling; denial keeps monitoring working and offers status/System Settings without repeated prompts. Later enablement does not replay the backlog or clear local unseen state. Launch at login is a separate opt-in, default off. The preceding proposal supplies the detailed accepted seen/onboarding interpretation.

Only Pause/Resume remains in this policy ticket. Runtime/cache review follows its resolution; no production implementation is authorized.

### Pause/Resume review — proposals only

1. Label the control Pause monitoring. It stops automatic GitHub discovery/refresh and native alerts, rather than merely muting alerts. Show a persistent Paused state and cached timestamps in both window and menu bar. Local cached reading/settings/seen changes remain available. Explicit user-driven detail reads and manual refresh remain available and are clearly manual; they respect server deadlines, quotas and backoff, do not restart automatic monitoring, and produce no native alerts while paused. Stop scheduling immediately; cancel in-flight work where safe. A completed valid result may be cached, but never alert during pause or advance recovery progress for work not actually completed. Disable notifications separately in Preferences to silence alerts while keeping monitoring active.
2. Pause lasts until explicit Resume and persists across window close, sleep and app restart. No timed snooze presets in version one. Resume while offline records monitoring enabled but shows offline; actual requests wait for connectivity and existing API/backoff deadlines. Resume never silently changes notification permission or category settings.
3. Resume uses the accepted bounded catch-up policy: one summary of newly discovered eligible missed activity instead of individual replay, then normal alerts for post-resume activity. No baseline reset, fake historical events, or clearing unseen state. Deduplicate activity already discovered manually; previously seen items do not appear in the catch-up summary. If native alerts are disabled/denied, recovery still updates Activity without OS delivery. Exact persisted progress/recovery markers are part of the next runtime/cache ticket.

## Answer

Resolved 2026-10-17 through the recorded human review rounds. The user accepted all three Pause/Resume recommendations: Pause monitoring stops automatic discovery/refresh and native alerts, preserves manual fetches and cached reading, persists until explicit Resume across close/sleep/restart, and Resume performs bounded grouped catch-up without clearing unseen state or resetting baselines. Offline resume respects connectivity and server/backoff deadlines; denied/disabled alerts do not prevent Activity recovery.

Together with the confirmed rounds above, this settles watch membership, capacity/overload policy, background lifetime/removal, alert coverage/suppression, grouping/retry/head behavior, first-sync and recovery behavior, generic-by-default native privacy, local seen state, foreground alerts including the current PR, permission/onboarding and opt-in login launch. Earlier proposal sections are conversation history; later confirmations supersede them, especially the rejected foreground suppression. The spec's confirmed policy section is the consolidated production contract.

Assets: [specification draft](../spec.md), [domain vocabulary](../../../GLOSSARY.md). API/native validation remains pending. Exact persistence, recovery mechanics, retention and discovery cadence belong to [Choose the local runtime and cache boundaries](07-runtime-and-cache-contract.md). No production implementation is authorized by this resolution.
