# Determine whether GitHub PR presets are accessible through gh

Labels: wayfinder:research
Type: research
Status: resolved
Assignee: presets_research
Parent: ../map.md
Blocked by: none

## Question

What are GitHub's saved PR filter presets, and can one authenticated GitHub.com user discover and read their real saved presets through supported gh commands, REST, or GraphQL? Distinguish PR dashboard presets from saved issue searches and projects views. Establish supported versus undocumented access, required auth, and the smallest viable fallback if public API access is missing. Check installed gh capabilities and primary documentation/source. Do not mutate remote account state or disclose private query contents.

## Answer

No supported saved PR dashboard view discovery operation was identified in the reviewed CLI and public API documentation. Automatic sync remains unverified; use local saved query copies as the smallest proposed fallback, pending a product decision. Live API/schema probing failed to connect, so this is not an absolute absence claim.

Answer asset: [GitHub PR saved views through gh](../research/pr-presets.md).
