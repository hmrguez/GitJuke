# Establish gh data access for the PR reader and stacks

Labels: wayfinder:research
Type: research
Status: resolved
Assignee: reader_stack_research
Parent: ../map.md
Blocked by: none

## Question

Which gh commands and public GitHub APIs cover PR lists, details, diffs, inline and general comments, reviews, check summaries and logs, and read-only stack navigation? The user confirmed their workflow is `gh stack`; investigate that specific command/provider and distinguish native commands from extensions. Identify truncation/pagination and fork/permission edge cases. Compare public stack metadata, chained base/head branches, and tool-specific metadata. State what can be inferred reliably, what remains ambiguous, and how confidence/provenance should appear in the domain model. Use primary sources and avoid remote writes.

## Answer

Resolved 2026-10-17. The gh-only reader is feasible with app-owned API pagination. Native GitHub stacks are generally available on GitHub.com as of 2026-10-06 and have REST/GraphQL read APIs; use these before visibly inferred branch relationships. Preserve completeness, permissions, head-SHA and unavailable-log states.

Asset: [PR reader and stack data through gh](../research/reader-and-stacks.md). Documentation research only; live API and installed-CLI verification remain implementation acceptance checks.

Provider clarification: user uses `gh stack`. Official `github/gh-stack` submits native GitHub stacks. Local `gh` reports the official extension installation hint, but no stack extension is presently installed; exact user provider/version remains unverified. Published native stacks can be read through gh api without installing an extension or checking out a repository. See the asset's provider section.
