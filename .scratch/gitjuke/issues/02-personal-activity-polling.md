# Establish personal activity coverage and realistic polling latency

Labels: wayfinder:research
Type: research
Status: resolved
Assignee: polling_research
Parent: ../map.md
Blocked by: none

## Question

Can a local Rust backend invoking gh deliver personal PR activity and current PR state within 10 seconds at sustainable cost? Investigate notifications, authored-PR comments and reviews, direct/team review requests, mentions on other PRs, and relevant check/workflow completion. Detail endpoint coverage and gaps, GitHub server polling guidance, ETag/Last-Modified semantics through gh, REST and GraphQL limits, payload shaping, pagination, process costs, and catch-up after sleep. Quantify illustrative request budgets and distinguish polling interval from guaranteed event latency. No hosted service or webhook requirement may be silently introduced. Use primary sources; avoid remote writes and aggressive polling.

## Answer

Resolved 2026-10-17 by polling_research. gh can provide personal PR discovery and targeted state reconciliation. Universal ten-second notification delivery conflicts with Notifications API X-Poll-Interval guidance and has no publication-latency guarantee. Use server-directed notification polling plus bounded ten-second known-PR/check refresh targets, conditional REST requests, measured GraphQL batches, persistent deduplication, and catch-up after sleep. Notification threads are not an event log; hydrate comments/reviews and independently reconcile checks/workflows. Product acceptance of this two-speed contract remains a later decision.

Asset: [Personal activity coverage and realistic polling latency](../research/personal-activity-polling.md).
