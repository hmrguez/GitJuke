# Native observations — 2026-10-18

Actual unsigned arm64 Tauri app at `tauri://localhost`, existing stored gh identity, pre-existing owned `hmrguez/FantasyEditor#4`. No remote fixture, credential, account or scope change. Background generic gh failure/recovery-incomplete status remained; successful reading does not qualify recovery or delivery.

## Initial packaged artifact

Built with `bun run tauri build --debug --bundles app --target aarch64-apple-darwin --no-sign`; quit the earlier running app and launched the rebuilt bundle.

- Activity retained 18 already-seen baseline records. Opened a PR #4 review comment: exact singleton, **2,042 UTF-8 bytes**, current head **2fefba2b95…**, outdated original **b764bc3ef8:22**. Focus initially targeted its article. Tab reached the named **Message body** region. Page Down visibly scrolled the body with focus retained; see [AX](native-message-body-ax.txt) and [screenshot](native-message-body.png). No new seen-state transition is implied.
- Shift-Tab back to Browse, Return: ordinary Conversation focused its stable panel, with three comments. [AX](native-browse-ax.txt). Four Tabs selected the Review threads control; Return loaded four resolved/outdated threads. Tab/Return opened the first thread (2,178-byte reply). Two Tabs from the panel reached Back to thread list; Return restored the exact originating path/original-line thread button. [AX](native-thread-return-ax.txt).
- Home selected/focused Overview; End selected/focused Checks; two Left presses selected/focused Changes.
- Resized the outer native window to the **960×640 screenshot**. The native title bar is included: this is compact-window smoke evidence, not measured 960×640 *reader content* qualification. Layout used explicit Back/detail, with the Activity list hidden. [Compact screenshot](native-compact.png), [AX](native-compact-ax.txt). Attempts to enlarge the height did not produce an observed new size; do not infer content sizing from drag targets.
- The established PR supplied **102 changed files**, head **2fefba2b95…**, base **e78ed97833…**. From the Changes tab, 22 Tabs reached Next files. Return replaced files 1–20 with **21–40 / 102**, retained the named file navigator focus through queue/loading and completion, and kept its pager reachable. [Page-two AX](native-files-page-2-ax.txt), [screenshot](native-files-page-2.png). This is native file pagination, not discussion pagination.
- Tab/Return selected page two's first file, `apps/desktop/src/modules/project/components/welcome/WorldVolume.tsx`: **3,128 bytes / 73 rows / five hunks**. The literal code table exposed old/new coordinates and change types. Initial hunk entry accidentally appended 2 to 1; native form validation correctly rejected 12 above the five-hunk maximum. Setting the field to 2 and pressing Return focused the stable named diff region. Page Down visibly scrolled the code; horizontal code scrolling remained contained. [AX](native-compact-patch-ax.txt), [screenshot](native-compact-patch.png). No current attached discussion anchor was available for this selected file; all supplied threads remained labeled outdated.
- Cmd-plus produced no observed text-size change. No claim of 200% zoom follows. No system reduced-motion setting or VoiceOver review was performed. No external source link was activated.

## Final artifact recheck

After moving the focus-restoration layout effect below its source-data declaration and recording list-return focus as a stable element ID (rather than retaining a detached DOM subtree), rebuilt the app using the [final build log](development-app-build-final.log), quit and relaunched. [artifact.json](artifact.json) identifies this final executable and host, not the required physical release reference.

- Reopened the same exact 2,042-byte Activity review comment; matching singleton/outdated label and article focus rendered again.
- Three Shift-Tabs and Return invoked Back to Activity. The exact originating row regained focus; the loaded 18-record page remained. [Final Activity-return AX](native-final-activity-return-ax.txt).
- Return reopened that row. Two Shift-Tabs and Return invoked Browse: Conversation panel regained focus. Four Tabs/Return selected threads; Tab/Return opened the first thread; two Tabs/Return restored the exact originating thread button again. [Final thread-return AX](native-final-thread-return-ax.txt), [screenshot](native-final-thread-return.png). Left the final app open at default width on this ordinary thread list.

Earlier captures document the immediately preceding implementation artifact; compact/patch/body observations were not all repeated after the final declaration-order and focus-metadata cleanup. The final recheck specifically qualifies exact destination, Back/Browse and thread-return behavior. No entire grouped gate is promoted.

## Remaining gates

Already-seen baseline only; no native unseen-to-seen transition. No >32,768-unit segmented or >128-KiB oversized live fixture, new current attached anchor, native discussion/reply page beyond the normal source limits, account/Clear race injection, actual VoiceOver, exact minimum content viewport, 200% text zoom, system reduced motion, native maximum DOM/process memory or physical M1/8-GiB/macOS-15 latency/frame-time/100-cycle stress qualification. No signing, notarization, distribution or native alerts. All **29 grouped release scenarios remain Pending**.
