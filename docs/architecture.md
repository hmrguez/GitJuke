# Architecture and data handling

GitJuke is a GitHub desktop client with explicit PR comment and thread reply submissions. A Tauri shell hosts a React frontend; a Rust runtime owns GitHub reads, comment submissions, scheduling, and a local SQLite store. There is no hosted backend or separate HTTP token client.

## Code layout

| Location | Responsibility |
| --- | --- |
| `src/main.tsx` | Browser bootstrap: fonts, React root, and startup recovery wrapper |
| `src/app/` | App composition, navigation history, native-menu hook, accessibility, and global theme/styles |
| `src/features/` | Activity, changes, conversation, checks, queries, notifications, preferences, and pull-request UI; each owns its models and styles |
| `src/components/` | Shared sidebar, dropdown, Markdown renderer, and startup recovery UI |
| `src/lib/` | Native invocation/events (`ipc.ts`), serialized contracts (`types.ts`), and shared UI copy |
| `src-tauri/src/app/` | Native lifecycle, window/tray/menu setup, startup recovery, singleton ownership, login, and macOS notification adapters |
| `src-tauri/src/commands/` | Frontend command handlers grouped by reader, changes, conversation/activity, checks/logs, queries, notifications, and build diagnostics |
| `crates/gitjuke-model/src/` | Serialized settings, reader, status, and check types; re-exported at the crate root |
| `crates/gitjuke-github/src/protocol/` | GitHub parsing and read plans; `transport.rs` owns bounded `gh` subprocess transport |
| `crates/gitjuke-core/src/runtime/` | Runtime orchestration and feature workers; runtime unit tests are separate from production code |
| `crates/gitjuke-core/src/activity/` | Activity storage, enrollment, and due-work selection |
| `crates/gitjuke-core/src/watch/` | Watch storage, reading, reservations, scheduling, and pending selection |
| `crates/gitjuke-core/src/storage/` | SQLite store, migrations and SQL, accounting, content admission, maintenance, and publication |
| `crates/gitjuke-core/src/features/` | Alert, changes, conversation, logs, and saved-query persistence |
| `crates/gitjuke-core/src/recovery/` | Database compatibility inspection and recovery reporting |
| `tests/frontend/` | React interaction, accessibility, rendering, and model tests |
| `tests/tooling/` | JavaScript release/build-tool tests |
| `scripts/` | Development/release tooling, Python tooling tests, and isolated qualification helpers |
| `docs/archive/implementation/` | Dated implementation records and verification artifacts |

Rust dependencies flow from the native shell through core to GitHub and shared models. Core keeps runtime and storage together because their transaction and publication rules share private implementation details. Crates define test-task and cache boundaries.

The core and GitHub crate roots use explicit module paths to group implementation files without changing existing Rust imports, module visibility, or core test names. This preserves the Nextest fast-suite admission and concurrency filters. Tests that use private implementation details stay beside their owning modules; integration tests remain under each crate's `tests/` directory. Migration SQL stays beside the migration implementation.

The frontend uses direct imports to each feature or shared module. `lib/ipc.ts` exposes invocation/events and re-exports the serialized types; appearance presets belong to `app/theme.ts`. Both IPC files require native CI validation when changed. Bun discovers JavaScript tests only under `tests/`, keeping discovery out of Cargo build artifacts. Scripts remain at their existing entry points for package commands and CI.

The Tauri crate root only exposes `run`. Native startup registers explicitly qualified command handlers; each handler uses the common bounded-response encoder in `commands/mod.rs`. Command names, payloads, serialized model names, database schema, and application identity are unchanged by this organization.

## GitHub access and monitoring

All GitHub requests run through the installed `gh` executable using an existing stored GitHub.com identity. The runtime binds to a stable account and revalidates it. It does not export tokens, alter credentials, change scopes, or switch accounts.

Active watches cover open authored PRs, direct/team review requests, and the selected open PR. Saved-query results and unselected stack neighbors do not automatically become watches. Discovery, head/check snapshots, and Activity hydration run through shared queue, quota, cancellation, and retry controls.

Pause stops automatic discovery, refresh, and alerts. Cached reading remains available; explicit reads and refreshes can still fetch under the normal server and quota rules. First sync establishes a baseline; recovery reconciles missed activity without replaying historical alerts.

## Readers

Changes binds cached patches to the PR comparison revision. All-files mode loads metadata in 100-file batches and renders completed batches while later ones load. Patches load near the viewport; cached files can be revisited without a new GitHub download. Missing, binary, incomplete, denied, and oversized content remains labeled. Single-file and paged reading are also available.

Conversation reads comments, review submissions, and review threads as independently paginated sources. Thread replies have their own pages. These pages do not imply a complete merged timeline. Source bodies use a 128-KiB limit and a shared retained-content budget; long bodies may require segmented reading.

Activity destinations resolve retained source identities into Conversation. Only rendering the explicitly opened matching complete record can mark it seen; segmented bodies require every segment. Ordinary browsing, file reading, and external source navigation leave Activity unseen.

Checks supports explicit GitHub Actions job-log reading with a 128-KiB UTF-8 limit and 200-line pages. Log bodies stay in memory and are released when their reader or authorization context changes. External providers expose validated HTTPS details links.

## Storage and recovery

SQLite holds cached content, configuration, and durable Activity/seen/dedup state. Clear downloaded content preserves durable history and saved-query configuration. Revision and generation barriers reject obsolete reads after changes, denial, cancellation, or cache removal.

Publication uses bounded transactions and, for larger replacements, journals that preserve the old visible snapshot until the replacement is ready. Cleanup, retention, and recovery share a maximum of 128 direct mutations per transaction, including relevant bookkeeping. Indexed candidate windows and maintained accounting reduce repeated full-store scans; they do not establish a total traversal-time or temporary-space bound.

Development version 0.1.0 uses schema 16. Migration supports fresh schema 0 and schemas 1–15, including the two independent schema-15 feature layouts. Upgrade steps check free space and synchronize SQLite. Unknown future, corrupt, or unreadable databases stop in a recovery view with the original files preserved. There is no automatic destructive reset or built-in updater.

Preferences reports database/sidecar sizes separately from disposable content and warns at 1 GiB of measured files. Temporary sort-file storage remains unmeasured. Full physical power-loss, peak-space, and reference-device performance qualification remains open.

## Native integration

Notification permission is requested only after explicit enabling. Alert dispatch commits an attempt marker before handing off to macOS; ambiguous attempts are not replayed. Launch at login is a separate default-off setting. Account/app ownership prevents duplicate writers, and Quit cancels and reaps remote work.

The implementation exists, but signed delivery/click behavior, actual login behavior, VoiceOver, and broader performance remain release checks. See [project status](status.md) and the [documentation index](README.md) for detailed evidence.
