# GitJuke interface

GitJuke is a personal review desk. Its visual reference is the quiet, well-spaced Codex desktop shell: a warm sidebar around an inset reading surface. Personality comes from JetBrains Mono, a small branch-shaped J mark, warm brass accents, and deliberate restraint.

## Foundations

- JetBrains Mono throughout. Body 13px, controls 12px, metadata 11px, auxiliary metadata 10px. PR titles 23px, page titles 16px. Weights 400 and 500.
- Spacing follows a 4px rhythm: 4 / 8 / 12 / 16 / 20 / 24 / 28 / 32 / 48. Reader padding is 32px, falling to 24px and 16px on compact screens.
- Radii: controls 7px, rows and reader cards 8px, shell 12px, dialogs 14px.
- Lucide outline icons with a shared 1.65 stroke. Navigation 16px, reader tabs 14px. Icon-only controls have accessible labels or titles.
- Graphite: canvas #171716, reading surface #111110, ink #eeeeec, accent #d6b98c. Paper: canvas #efeeea, surface #fdfcf9, ink #272725, accent #8b6840.
- Paper metadata uses a 70% ink mix; thread metadata uses the semantic muted tone without additional opacity so small text retains contrast.
- Semantic variables in `src/theme.css` derive borders, muted text, hover surfaces and accent treatments from those four colors. Success, warning and failure have distinct semantic colors and text/icon labels.

The generated transparent mark lives in `public/brand/gitjuke-mark.png`. The sidebar masks it with the theme accent; desktop icon sizes in `src-tauri/icons/` share the same silhouette on a warm graphite rounded-square tile, sourced from `public/brand/gitjuke-app-icon.png`. Generation prompts and reproduction notes live in `public/brand/README.md`.

## Components and behavior

The sidebar, list navigator, reader toolbar, tabs, buttons, inputs, segmented controls, pagination, status banners and dialogs share styles in `src/app.css`. Changes and Conversation own only reader-specific layout.

Selected navigation uses a neutral surface; the reader tab has a brass underline. Color never carries selection or status alone. All controls have visible keyboard focus. Dialogs use the native modal focus boundary and Escape dismissal, with bespoke content and controls. The two-option reading mode is a segmented control, avoiding an OS-styled dropdown.

Preferences exposes Graphite and Paper rather than arbitrary color customization. Existing stored colors are left intact; the stored mode selects the corresponding new palette. The typeface and default typography remain fixed. Session accessibility enlargement scales text through 200% without changing source or retention limits. Diagnostics are available through Sync status or the relevant Preferences section instead of dominating the reading flow.

Lists filter only the currently loaded page and say so. Pagination and bounded loading remain authoritative. Opening files, reading messages, Activity receipts and native GitHub actions retain their existing runtime behavior.

At the 960px content minimum, and whenever text is enlarged, the list and detail become separate views with a Back action. Below 640px the sidebar becomes an icon rail with named controls. Text wraps, reader content scrolls, and code retains horizontal scrolling. Reduced-motion preferences disable transitions. On the tested native title bar, a 672px outer minimum reserves 32px chrome and supplies the confirmed 640px content height; each advertised OS still needs native measurement.

## Verification

Run `bun run build` and `bun test`. Visually inspect My PRs, Review requests, Activity, all four reader tabs, all Conversation sources, reading mode, and all four Preferences sections in both themes. Check compact layout and keyboard navigation. Production has no sample-data fallback; use real cached native data or explicitly isolated qualification fixtures.
