# Native local reader qualification — 2026-10-18

Development 0.1.0; schema **14** unchanged. This follows [reader keyboard/focus qualification](2026-10-18-reader-navigation.md). The completed Wayfinder decisions, Stacks, bundled JetBrains Mono, four-color appearance and all reader/transport/IPC/queue/cache/mutation bounds remain unchanged. Existing uncommitted work is preserved. No production reader behavior needed correction in this slice.

## What is now qualified

Actual native WebKit, Tauri IPC, Rust worker/subprocess parsing and SQLite were exercised together using **controlled local synthetic fixtures**. These observations close narrow native-local gaps; they do not establish compatibility with live GitHub bodies or coordinates.

| Local fixture | Native observation | Read-only SQLite evidence |
| --- | --- | --- |
| Short complete comment, 36 UTF-8 bytes | Exact destination rendered and focused | Only its matching retained record became seen |
| Segmented comment, 70,571 ASCII bytes | Explicit segments 1/3, 2/3 and 3/3; replacement focused the named body region | Unseen after segment 1, segment 2, and revisiting 1→2; only this record became seen after segment 3 |
| Current review comment, 62 UTF-8 bytes | Initially labeled patch unavailable; selected `file-0.txt` attached to RIGHT line 201, supplied range 199–201, head `aaaaaaaaaa…`, base `bbbbbbbbbb…` | Exact node/digest and line 201 retained; the complete matching body became seen |
| Oversized comment, 131,073 UTF-8 bytes | Explicit >128-KiB placeholder; no clipped prefix | Remained unseen |
| Complete textless APPROVED review | Complete metadata-only event, 0 bytes | Became seen |
| Unopened neighbor | Never opened as an exact destination | Remained unseen throughout |

Keyboard activation of **Return to diff · line 201** selected semantic **Page 2 of 2**, rendered the final supplied lines **200–201**, and focused the stable named diff region. The full eligible patch is **4,733 UTF-8 bytes / 202 rows** (one hunk header plus 201 additions). Keyboard Previous/Next row-page traversal returned through Page 1 and the terminal Page 2 while preserving diff focus. The range starts on the previous semantic page; attachment validates the complete supplied coordinate range, not simultaneous visibility of all three lines.

After Quit/relaunch and explicit local identity revalidation, all six fixture states persisted: four seen, oversized and neighbor unseen. **Mark loaded page seen** and external GitHub source actions were never activated. No SQL update/reset of seen state was used after preparation; the observed transitions came through native reader receipts.

## Isolation and fixture tooling

The [qualification Tauri config](../../scripts/fixtures/tauri.reader-qualification.json) changes only the separate build's product name, app identifier and window title. The app is **GitJuke Qualification — LOCAL SYNTHETIC FIXTURES**, identifier `dev.hmrguez.gitjuke.qualification`. It uses its own OS application-data directory and starts paused with a local fake `gh` executable and empty local configuration directory. The normal app's stored account, executable/configuration paths, appearance and database were not configured or edited by these tools.

The explicit [Rust preparer](../../crates/gitjuke-core/examples/reader_qualification.rs) is an example executable, never linked into the application. It creates six initially unseen records and payload labels through the existing schema/Store APIs plus scoped fixture-only ledger inserts. Activity preview bodies are **null**: exact destination reads must fetch their own matching bodies, including the >8-KiB segmented case. The preparer refuses existing directories and every app-data namespace except the qualification identifier. [Existing-data refusal](evidence/2026-10-18-local-reader/refuse-existing.log) and [production-namespace refusal](evidence/2026-10-18-local-reader/refuse-production.log) were exercised before any write.

The existing [synthetic gh helper](../../crates/gitjuke-core/tests/support/gh.py) now accepts a per-node destination map and matching fullDatabaseId coordinate records. Legacy fixtures keep their defaults. The helper performs local file/stdin/stdout operations, not GitHub requests, credential/keyring access or remote writes. There is no application fixture fallback, test IPC handler or production feature flag. The native local app uses the same application code and commands as the normal app, with this explicitly configured subprocess fixture.

The qualification database is `/Users/hmrguez/Library/Application Support/dev.hmrguez.gitjuke.qualification/gitjuke.sqlite3`; source/scenario files are under `target/reader-qualification-fixture/`. The qualification app was stopped after capture; its fixtures and database remain separate, disposable test data for inspection. No remote content was needed or created.

For a fresh setup from the repository root:

```sh
cargo run -p gitjuke-core --offline --example reader_qualification -- \
  "$PWD/target/reader-qualification-fixture" \
  "$HOME/Library/Application Support/dev.hmrguez.gitjuke.qualification"
bun run tauri build --debug --bundles app --target aarch64-apple-darwin --no-sign \
  --config scripts/fixtures/tauri.reader-qualification.json
```

The recorded directories already exist, so the preparer deliberately refuses a second preparation. A clean repeat requires first stopping the qualification app and archiving its two dedicated fixture directories; no automatic destructive reset exists. Launch the distinct qualification `.app`, use **Refresh** to validate its local identity while paused, then open **Activity**. The normal `bun run tauri build …` command still produces the regular identifier and bundle.

## Native evidence

[Evidence directory](evidence/2026-10-18-local-reader/), [qualification artifact/fixture hashes](evidence/2026-10-18-local-reader/artifact.json), [native observations](evidence/2026-10-18-local-reader/native.md).

- [Initial six unseen records](evidence/2026-10-18-local-reader/native-initial.png) and [initial SQLite snapshot](evidence/2026-10-18-local-reader/initial.json).
- Partial-segment snapshots: [segment 1](evidence/2026-10-18-local-reader/segment-1.json), [segment 2](evidence/2026-10-18-local-reader/segment-2.json), [repeated visits](evidence/2026-10-18-local-reader/repeated-segments.json). [Completed segment 3 screenshot](evidence/2026-10-18-local-reader/native-segment-3.png) and [matching SQLite transition](evidence/2026-10-18-local-reader/segment-3.json).
- [Short-body transition](evidence/2026-10-18-local-reader/short.json), [oversized unseen state](evidence/2026-10-18-local-reader/oversized.json), [empty-review transition](evidence/2026-10-18-local-reader/empty-review.json).
- [Current exact source metadata](evidence/2026-10-18-local-reader/current-anchor.json), [native attachment/jump AX](evidence/2026-10-18-local-reader/native-current-anchor-ax.txt), [page-two screenshot](evidence/2026-10-18-local-reader/native-current-anchor.png), [terminal page focus](evidence/2026-10-18-local-reader/native-row-page-return-ax.txt).
- [Before restart](evidence/2026-10-18-local-reader/before-restart.json), [after restart](evidence/2026-10-18-local-reader/after-restart.json), [native persisted states](evidence/2026-10-18-local-reader/native-restart-activity.png).

The [snapshot tool](../../scripts/reader-qualification-snapshot.py) accepts only the separate qualification DB name/namespace, opens SQLite read-only with `query_only=ON`, and records metadata/seen state rather than body content. Initial/segment-1 source lists are empty because the initial metadata filter used the wrong cache-key prefix; their seen maps remain valid. The corrected filter is used from segment 2 onward. No source-availability inference is drawn from those earlier empty metadata lists.

The [observation verifier](../../scripts/verify-reader-qualification.py) checks all eleven captured SQLite stages, relevant native AX text and helper-request traces: [22 observation checks pass](evidence/2026-10-18-local-reader/observation-verification.json). This verifies captured evidence; it is not an unattended fresh native UI test. [Helper request trace](evidence/2026-10-18-local-reader/native-gh-reads.jsonl) shows **one** matching `pulls/comments/103` point read and **zero Conversation history scans**. Direct-node requests have no `first` or `after` traversal variables.

Explicit revalidation after restart also produced two synthetic historical check Activity entries already marked seen. They are outside the six-reader-record oracle and are not counted as reader-induced transitions. The fixture's baseline/recovery state remains incomplete; no recovery completion or native delivery claim follows.

## Deterministic verification and builds

**87 distinct tests pass**: 23 frontend + 59 integration + four Conversation core + one IPC. Repeated focused runs are not additional tests.

- [23 frontend tests](evidence/2026-10-18-local-reader/frontend.log), including two new mounted tests: a segmented exact destination retains traversal while its matching current-anchor patch loads, Return to diff selects semantic page two/focus, and the final distinct segment alone submits the receipt; leaving a partial destination releases authorization and reopening starts fresh segment coverage. These use test-only simulated IPC/worker responses, independently of the native local evidence.
- [35 reader integration tests](evidence/2026-10-18-local-reader/reader-regressions.log): Activity 3, Changes 5, Conversation 9, GitHub 6, transport 5, watch store 7. The new per-node fixture test checks retained body identity, current range/commit/path, fullDatabaseId point lookup, oversized rejection and exact seen-neighbor isolation. The opt-in live test stays ignored.
- [24 runtime/store regressions](evidence/2026-10-18-local-reader/runtime-store-regressions.log), [four core boundary tests](evidence/2026-10-18-local-reader/core-conversation.log), [IPC ceiling](evidence/2026-10-18-local-reader/ipc.log), [formatting](evidence/2026-10-18-local-reader/formatting.log), [workspace Clippy](evidence/2026-10-18-local-reader/clippy.log).
- TypeScript/Vite and arm64 compilation pass in both the [qualification build](evidence/2026-10-18-local-reader/qualification-app-build.log) and final [normal production app build](evidence/2026-10-18-local-reader/production-app-build.log). The [normal bundle metadata](evidence/2026-10-18-local-reader/production-artifact.json) confirms the regular identifier; the normal app was not relaunched or live-tested in this slice.

These scoped reader regressions do not rerun the full storage/500-MiB/migration/crash matrix. Functional run times do not qualify native performance.

## Limits and next slice

Native-local coverage now exists for an initially unseen short/segmented/empty-review transition, a >128-KiB placeholder, exact current RIGHT range attachment, semantic row paging and restart persistence. The combined segmented-body/current-anchor interaction is mounted-test evidence; native fixtures exercise those sources separately.

Live GitHub segmented/oversized/new-unseen/current-anchor fixtures remain unavailable. Native inclusive 128-KiB/non-ASCII segment boundaries, current LEFT-side/range variants, outdated/ambiguous race injection, native multi-page discussions/replies, VoiceOver, actual 200% text zoom, measured 960×640 content sizing, system reduced motion, maximum DOM/process memory and physical M1/8-GiB/macOS-15 performance remain open. Signing/distribution/native alerts remain unqualified. All **29 grouped release scenarios remain Pending**.

Next: extend the isolated local harness to native boundary/negative-coordinate and discussion-pagination cases, then exercise VoiceOver, actual text zoom and reduced motion. Live compatibility will require pre-existing suitable owned fixtures or explicit approval of a named remote fixture and exact content before creating anything. No remote creation is implied by this local qualification.
