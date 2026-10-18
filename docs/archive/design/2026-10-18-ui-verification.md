# UI revamp verification · 2026-10-18

Verified in the real Tauri/WebKit app, using the existing local account and cached PRs. No fixture fallback was added to production.

| Surface | Native visual observation |
| --- | --- |
| My PRs | Empty authored queue, filter, collapsed and expanded known-PR form, restored reader |
| Review requests | Empty review queue, correct heading and selected navigation |
| Activity | 18 retained entries, recovery/error banners, source navigation |
| Overview | Empty description on FantasyEditor #4; populated plain text on cli/cli #14719 |
| Changes | 20-file and 3-file lists; cached Go patch with additions, deletions, line numbers and horizontal scrolling |
| Reading modes | Continuous and Paged selection; persisted setting; semantic page controls; hunk jump focuses diff |
| Conversation | Comments, review loading and populated metadata-only review, review threads, populated thread replies, exact Activity destination, empty source |
| Checks | Pending state and 18 populated contexts; uppercase SUCCESS normalized to green success; skipped remains neutral |
| Preferences | Appearance, Monitoring, GitHub and Storage; Escape dismisses modal |
| Palettes | Paper and Graphite viewed in native app |
| Compact window | 960 × 640; reader switches to single-pane presentation; Back returns focus to list |

The first pass found and corrected a vertical filter layout, undersized text, uppercase status interpretation, thread heading spacing and outer document scrolling. Shared root scroll containment now leaves the shell stationary while the reader scrolls.

`bun run build` passes. `bun test` passes: 23 tests, 90 expectations, covering reader focus, source replacement, destination receipts, diff bounds and safe text rendering. The local debug `.app` bundle builds successfully.

Limits: authored and requested queues contain no open PRs on this account, so their populated row appearance is not qualified with live rows. Some GitHub sources report pre-existing recovery/fetch errors or queued reads; these were displayed honestly. Native VoiceOver, increased system text size, and every failure/status permutation were not exhaustively qualified. Backend behavior and release qualification were outside this visual change.
