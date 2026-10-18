# GitJuke UI prototype

Disposable React/Vite exploration, not the production Tauri app. Question: which reader structure makes personal PR triage feel effortless without unnecessary chrome?

Planning handoff settled 2026-10-17: Stacks and the appearance are human-approved, the [specification](../spec.md) is settled, the [acceptance policy](../acceptance.md) is confirmed and the [wayfinder map](../map.md) complete. Native/API/performance/signing evidence remains pending. No production implementation has started; a subsequent user instruction is required to begin.

From the repository root:

```sh
npm install
npm run prototype
```

Continue review at `/prototype?variant=stacks` using the localhost URL printed by Vite (normally http://127.0.0.1:5173; it selects another port if occupied). Build check: `npm run prototype:build`. The development-only review bar is hidden in the production build.

## Directions

| Direction | Structure and emphasis | Best fit | Trade-off |
| --- | --- | --- | --- |
| Reader | Navigation, compact PR list, full detail; list scanning comes first | Moving among authored PRs, review requests, and saved queries | Three panes need desktop width; compact mode uses explicit back navigation |
| Inbox | Wide activity cards with excerpts and a contextual reader; events come first | Catching up after time away | Less dense for scanning many PRs; activity is not equivalent to GitHub's unread state |
| Stacks | Branch trunk and ordered native-stack members beside the reader | Inspecting dependencies in published gh stack work | Dependency emphasis can distract from unrelated requests; native metadata is required |

Each keeps the same fixture dataset, read-only panels, rounded corners, reduced-motion behavior, and keyboard focus. Layout is shareable through `?variant=reader`, `?variant=inbox`, or `?variant=stacks`; use the bottom arrows or Left/Right outside text fields and reader tabs.

## Appearance revision

The user's first reaction requested more personality, JetBrains Mono, light/dark presets, and about four custom colors. All UI now uses locally bundled JetBrains Mono. Carbon (dark) and Parchment (light) presets change background, surface, accent, and text together. The Appearance dialog allows native pickers and editable hex inputs for those four colors, derives neutral/interactive tones, and shows text/surface contrast. Reach it through Theme in the review bar or Preferences → Appearance. Changes are session-only in this disposable prototype.

## Try

- Search and select a PR, switch reader tabs, select a changed file, expand sample check output and a resolved conversation.
- Navigate published stack members without a checkout.
- Create, edit, and remove a local saved query. The sample parser supports only the qualifiers stated in its editor; production delegates to GitHub Search.
- Inspect loading, empty, offline and rate-limited states using the review bar.
- Open the menu-bar simulation, select personal activity, and inspect local seen state.
- Change light/dark presets or customize the four palette colors. The preview never asks for macOS notification permission or changes OS appearance.

## Boundaries

All PRs, comments, logs, timestamps and the shown notification interval are fictional fixtures. No polling, live gh calls, native tray, durable settings, Markdown renderer, or native notification delivery is implemented. Open-in-GitHub explains the fixture boundary and links to the real PR dashboard. Copy link copies an explicitly sample PR URL.

No Git repository was present. The prototype remains isolated here rather than being promoted into production or captured on a nonexistent throwaway branch. Stacks is human-approved; production work requires a settled specification handoff and a subsequent user instruction to begin. Prototype approval alone does not authorize initializing/promoting/rewriting it as the production app.

## Design references

21st metadata-only discovery informed comparable navigation/notification patterns: [Sidebar by wensity](https://21st.dev/@wensity/components/sidebar), [Dashboard Sidebar by arunjdass](https://21st.dev/@arunjdass/components/dashboard-sidebar), and [Notification Panel by uvain](https://21st.dev/@uvain/components/notification-panel). No catalog code or hosted generation was used; these previews are original code. Each layout varies hierarchy and interaction emphasis, not only color.

## Review evidence

Browser checks covered search typing/focus, file selection, check expansion, local query create/edit/delete, empty/offline previews, native-stack navigation, layout switching, and all four theme values plus light/dark preset reset. Production build succeeds. Desktop and compact layouts are inspected separately. Stacks is now the human-approved UI direction. Notification policy is resolved, including alerts for the currently viewed PR; authenticated API and native desktop behavior remain unverified.

Screenshots in `review/` are primary prototype evidence, not evidence of live GitHub or native desktop functionality.

## Current human feedback — 2026-10-17

The latest appearance direction is confirmed: JetBrains Mono throughout, light/dark presets, and four customizable colors (background, surface, accent, text). The user subsequently confirmed Stacks as the default. Reader and Inbox remain comparison candidates; a shipping layout switcher is not implied.

Continuation session opened the Stacks route at http://127.0.0.1:5174/prototype?variant=stacks. This is an opening check only; the earlier browser/build checks above were not repeated. Authenticated API and native macOS behavior remain unverified.
