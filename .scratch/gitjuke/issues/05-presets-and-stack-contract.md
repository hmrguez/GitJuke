# Choose preset and stack behavior at API boundaries

Labels: wayfinder:grilling
Type: grilling
Status: resolved
Assignee: hmrguez
Parent: ../map.md
Blocked by: 01, 03

## Question

Given supported access and inference limits, what preset behavior and stack recognition meet the user's needs? Establish whether native-preset fallback is acceptable and clarify the actual stack workflow, including how ambiguous relationships are shown. Define canonical terms for preset, stack, PR activity, and GitHub notification only as decisions are settled.

## Answer

The user accepted local saved queries “for now” and confirmed their stack workflow as `gh stack`, with read-only navigation already agreed. Local queries remain independent of dashboard saved views; the prototype labels them Local queries and supports session-only create/edit/remove. The production spec persists them locally.

Published native-stack metadata is the supported reading contract identified by the research. Exact official extension/provider compatibility remains an implementation verification gate; no legacy extension or inferred relationship support is promised. Canonical domain terms are recorded in [the glossary](../../../GLOSSARY.md).
