# Choose freshness and personal notification behavior

Labels: wayfinder:grilling
Type: grilling
Status: resolved
Assignee: hmrguez
Parent: ../map.md
Blocked by: 02

## Question

Given the researched API coverage and latency limits, what freshness promise is acceptable for active PR/check state and personal notification discovery?

## Answer

The user explicitly accepted: “Use 10s refresh for active PRs and checks, respect GitHub’s interval for notifications.” These are refresh targets subject to sleep, connectivity, quotas and source availability, not universal event-delivery guarantees.

The original broader question also contained unresolved notification-policy preferences. Those were split into [Choose notification defaults and the active watch set](08-notification-defaults.md), so this resolution does not claim approval of suppression, grouping, capacity, or read-state defaults.
