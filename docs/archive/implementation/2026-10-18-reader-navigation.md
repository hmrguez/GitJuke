# Reader keyboard and focus qualification — 2026-10-18

Development 0.1.0; schema **14** unchanged. The completed Wayfinder map remains settled. This continues the authorized implementation and qualification handoff, preserving existing uncommitted work, Stacks, bundled JetBrains Mono, four-color appearance and all source/transport/IPC/queue/storage/mutation limits.

## Corrections

Native inspection reproduced focus dropping to the web document after **Browse Conversation sources**, and returning from replies failed to restore the originating thread button. Browse now focuses the newly committed ordinary Conversation panel. Returning from replies keeps focus on a stable region while loading, then restores the exact thread button once the replacement list commits. If the user moves elsewhere while awaiting the list, the response does not take focus back. Ref maps contain only mounted thread buttons; list return stores a stable element ID rather than retaining a detached DOM subtree.

Message bodies are named, keyboard-focusable scroll regions. Explicit segment navigation resets the replaced text's scroll position and focuses the new segment, including when the navigation button becomes disabled at a boundary. Initial exact destination focus and committed-body receipts remain separate: ordinary messages never gain seen authorization, all distinct segments are still required, changed generation/digest remounts coverage, and stale heads/placeholders remain unseen. Polling unchanged content does not refocus it. No body, excerpt or source limit changes.

Conversation's First page action now focuses its stable panel like Next/Previous. Changes file-page replacement focuses the file navigator; semantic row-page replacement focuses the stable diff viewport, including terminal pages. An inline **Go to thread** jump preserves the intended diff focus rather than immediately overriding it with discussion focus. Reader tabs support Home/End alongside Left/Right. Compact Back releases the destination and restores the originating Activity/list control, with a focusable list fallback if the original control disappeared. Pending detail requests are invalidated when backing out.

Navigation and pagination can wrap. The sidebar can scroll when available height shrinks, rather than relying on a fixed 640-pixel app frame. Narrow-screen spacing preserves primary navigation labels. These are layout safeguards, not evidence of completed 200% text-zoom or reduced-motion qualification.

## Deterministic verification

See [evidence](evidence/2026-10-18-reader-navigation/). `happy-dom` **21.0.3** is a pinned development dependency for mounted React interaction tests; its fake IPC exists only in test code. Production has no fixture fallback. DOM simulation does not establish WebKit layout, real keyboard traversal, VoiceOver or native memory/performance.

- [Frontend suite](evidence/2026-10-18-reader-navigation/frontend.log): **21 passing tests**, including ten new mounted-component tests. The tests cover ordinary/exact segmented focus and scroll reset; distinct committed segment coverage; empty-review and oversized-placeholder receipts; asynchronous exact-thread focus restoration and user-moved-focus protection; Next/First replacement with old-body release; superseded source results and cancellation; generation restart/stale-head rejection; terminal 200-row patch pages and file-page focus; and exact inline diff-jump focus. The [pre-fix run](evidence/2026-10-18-reader-navigation/interaction-before.log) has four reproducible failures among the original six tests. Those four pass after correction in the [final ten-test interaction run](evidence/2026-10-18-reader-navigation/interaction-after.log), repeated within the 21-test suite and not additional distinct tests. The initial standalone experiment also exposed test-harness setup/diagnostic issues; it is not product evidence.
- [Reader integration regressions](evidence/2026-10-18-reader-navigation/regressions.log): **58 passing tests** across Activity (3), Changes (5), Conversation (8), GitHub (6), runtime (16), store (8), transport (5) and watch-store (7), including synthetic-gh identity/comparison/cancellation/Clear/denial/eviction/receipt barriers. The opt-in live Conversation test remains ignored in this deterministic run.
- [Four Conversation core boundary tests](evidence/2026-10-18-reader-navigation/core-conversation.log) and [inclusive serialized IPC ceiling](evidence/2026-10-18-reader-navigation/ipc.log).
- [Rust formatting](evidence/2026-10-18-reader-navigation/formatting.log), [workspace Clippy](evidence/2026-10-18-reader-navigation/clippy.log), and TypeScript/Vite through the [final unsigned arm64 build](evidence/2026-10-18-reader-navigation/development-app-build-final.log).

There are **84 distinct passing deterministic tests**: 21 frontend + 58 integration + four core + one IPC. Repeated interaction runs are not additional tests. This is scoped reader verification. No full storage/500-MiB/migration/crash matrix or physical-target performance rerun is implied.

## Live and native scope

[Native observations](evidence/2026-10-18-reader-navigation/native.md) and [artifact metadata](evidence/2026-10-18-reader-navigation/artifact.json) separate actual WebKit/gh reading from simulated tests. Existing stored identity and pre-existing **hmrguez/FantasyEditor PR #4** supplied already-seen Activity, exact review comments, four review threads and a **102-file** comparison. No remote content, credentials or scopes were changed. No separate opt-in live test was rerun; network evidence here comes from the actual native app.

At default width, Tab reached the new Message body region; Page Down visibly scrolled its text while retaining focus. Keyboard Browse focused the ordinary Conversation panel. Keyboard source selection/open/reply return restored the exact thread button. Home/End and Left/Right selected/focused the intended reader tabs.

A **960×640 outer-window capture**, including native title chrome, exercised the compact list/detail layout. It does not establish the required 960×640 *content* viewport or the entire compact accessibility matrix. Keyboard traversal reached Next files; page replacement displayed **Files 21–40 / 102**, retained navigator focus, and released the first page. Selecting `apps/desktop/src/modules/project/components/welcome/WorldVolume.tsx` on page two rendered **3,128 UTF-8 bytes / 73 rows / five hunks**. A valid hunk-2 submission focused the named diff viewport; Page Down scrolled it. Long paths wrapped and the viewport scrolled code horizontally. This adds native file-page evidence, not native multi-page discussion/reply evidence.

Existing baseline records remain already seen, so no native unseen-to-seen transition is claimed. Existing bodies do not provide segmented/oversized sources or current attached anchors; those remain deterministic-only. The standard zoom shortcut produced no visible text-size change; 200% text zoom remains unqualified. System reduced-motion behavior and VoiceOver were not exercised. Accessibility-tree inspection is not VoiceOver qualification. Background generic gh errors/incomplete recovery remain visible independently of successful reading; no recovery/delivery qualification follows.

## Next qualification slice

Use an explicitly authorized disposable owned-repository fixture for a new unseen record, a body above 32,768 UTF-16 units but within 128 KiB, a current revision/file/side/range anchor, and more than 50 comments/replies or 20 threads. Specify the destination and exact remote writes before creating any fixture. Existing read-only fixtures and deterministic tests do not authorize remote creation. Then exercise the same native flow with VoiceOver, measured content sizing, actual 200% text zoom and system reduced motion. Keep unavailable/unsupported sources labeled; avoid expanding rich formatting or infrastructure for this slice.

Full keyboard coverage, maximum DOM/process memory, physical M1/8-GiB/macOS-15 performance, signed distribution and native alerts remain open. All **29 grouped release scenarios remain Pending**.


## Follow-on: isolated native local qualification

The [native local reader checkpoint](2026-10-18-local-reader.md) now supplies controlled synthetic-source evidence in a separate native app for unseen-to-seen short/segmented/textless-review records, oversized/unopened-neighbor unseen preservation, current RIGHT range attachment/semantic return-to-diff and restart persistence. This does not convert the earlier owned live fixtures into unseen/segmented/current-anchor fixtures or establish live provider compatibility. All grouped release gates remain Pending.
