# Choose the minimal PR reader interface through a prototype

Labels: wayfinder:prototype
Type: prototype
Status: resolved
Assignee: hmrguez
Parent: ../map.md
Blocked by: 04, 05

## Question

Which layout and interactions let the user triage PRs, inspect changes/conversations/checks, navigate stacks, and understand personal activity with minimal chrome? Use frontend-design and prototype; create a disposable interactive UI asset and obtain the user's reaction before resolving. Cover loading, empty, stale, error, and notification entry states as well as the normal reader.

## Comments

Three interactive React directions are available: Reader, Inbox, and Stacks. See [prototype notes](../prototype/README.md) and [spec draft](../spec.md). They share sample data and read-only detail panels, with loading/empty/offline/rate-limit previews, query editing, and a menu-bar simulation.

First user reaction: “I like it but it lacks a bit of personality.” They requested roughly four customizable theme colors, light/dark presets, and JetBrains Mono. The revision implements these; no final layout has been selected, so this ticket remains claimed rather than resolved.

### Feedback reconciliation — 2026-10-17

The user likes the latest appearance and explicitly confirms JetBrains Mono throughout, light/dark presets, and four customizable colors: background, surface, accent, text. Preserve this direction. The last viewed layout was Stacks; that is evidence of what was viewed, not approval of the default. Reader and Inbox remain alternatives. Ask explicitly whether Stacks should be the initial/default layout before settling this ticket. Appearance acceptance does not resolve the full UI question.

The fixture prototype was started with `npm run prototype` and opened at `/prototype?variant=stacks` (port 5174 because 5173 was occupied). Prior build/browser evidence remains in the prototype notes; this session's opening does not revalidate those checks or live/native behavior.

## Answer

Resolved 2026-10-17 through human review. The user explicitly accepted Stacks as the default, with the latest JetBrains Mono appearance, light/dark presets, and four customizable colors (background, surface, accent, text). Reader and Inbox remain prototype comparison alternatives; shipping a layout switcher is not required by this decision.

Assets: [prototype notes](../prototype/README.md), [spec draft](../spec.md). Prior fixture/browser/build evidence is recorded in the notes. This resolves the prototype direction, not authenticated API behavior, native desktop lifecycle, or complete production accessibility/edge-state validation. Those remain acceptance gates.
