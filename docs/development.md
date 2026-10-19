# Development

## Requirements

- An Apple Silicon Mac running macOS 15 or later.
- Xcode command-line tools and Rust installed through `rustup`. The repository pins its Rust toolchain in `rust-toolchain.toml`.
- Bun 1.4.2 or later.
- Python 3.9 or later for the Rust test runner and its cache.
- The GitHub CLI (`gh`), already signed in to GitHub.com with the account you want to use.

GitJuke uses the stored CLI identity. It does not install `gh`, log in, switch accounts, or change scopes. The default configuration directory is `~/.config/gh`, independently of shell token and XDG overrides. Preferences accepts explicit absolute executable and configuration paths, then revalidates the originally bound account.

## Run locally

From the repository root:

```sh
bun install --frozen-lockfile
bun run tauri dev
```

Use the native app to read real GitHub data. `bun run dev` starts only the browser frontend; it has no native runtime and does not supply sample PRs.

The historical fixture prototype is separate:

```sh
bun run prototype
bun run prototype:build
```

It is useful for inspecting the original design, but it is not the production reader or live-data evidence.

## Verify changes

Install the checksum-pinned Nextest binary once. It lives in the ignored `target/test-tools/` directory, without changing your global Rust installation:

```sh
bun run test:setup
```

| Command | Purpose |
| --- | --- |
| `bun run build` | TypeScript checks and frontend build |
| `bun test` | Frontend tests, including reader interactions and diff models |
| `bun run test:rust` | Fast Rust workspace tests; reuse successful tasks with unchanged inputs |
| `bun run test:rust --no-cache` | Execute the fast suite, bypassing test-result caching |
| `bun run test:affected` | Fast tests for working-tree changes relative to `HEAD`, plus dependent crates |
| `bun run test:affected --since origin/master` | Fast tests affected by changes relative to another ref |
| `bun run test:qualification` | Uncached storage, crash-recovery, and runtime qualification |
| `bun run test:full` | Uncached full Rust workspace suite, including doctests |
| `bun run test:all` | Frontend tests and the uncached full Rust suite |
| `bun run test:partition` | Verify fast and qualification are disjoint and cover every normally enabled Rust test |
| `bun run test:runner` | Check cache invalidation, affected selection, and failure handling |
| `cargo test --workspace --locked --offline` | Original full-suite runner, using locally available dependencies |
| `bun run check` | Frontend build and workspace Clippy with warnings treated as errors |

`bun run test` is a Rust-only alias for the fast suite. Both Rust runners use locally installed, locked dependencies; run `cargo fetch --locked` if they are missing. Dependencies are locked in `bun.lock` and `Cargo.lock`. Node/npm are needed only to reproduce the legacy JavaScript benchmark; see the [Bun migration record](archive/implementation/2026-10-18-bun.md).

The fast suite includes GitHub parsing/transport, selected inexpensive storage and query tests, and native shell tests. Large disk fixtures, process-kill recovery, and runtime integration remain in qualification. The two suites partition the original tests; pre-existing ignored cases still need their documented opt-in setup. Admission to the fast core suite is explicit in `.config/nextest.toml`, so a new expensive test does not silently make everyday checks slow. Nextest test groups preserve the concurrency limits formerly provided by in-process fixture locks. Doctests run with the fast and full commands because Nextest does not execute them.

Successful task results are stored under `target/gitjuke-test-cache/`. Keys cover the crate and its transitive workspace dependencies, tracked and new source/fixtures, executable modes, workspace manifests and lockfile, runner/configuration, compiler and Nextest versions, OS/architecture, Cargo configuration, and relevant build/test environment variables. Only complete successful runs with matching before/after input hashes are cached. Native desktop tests always execute because build metadata also depends on Git state and build time. `--no-cache` bypasses pass entries; deleting `target/gitjuke-test-cache/` clears them.

Use `-p gitjuke-github` (or another crate) to narrow a task manually. `--since` selects changed crates and their dependents, not merely previous failures. It includes staged, unstaged, deleted, and new files. Unknown/shared input paths select the whole workspace; an unavailable base ref also falls back to the whole workspace. A clean tree compared to `HEAD` selects nothing, so use the ordinary fast command to validate a newly checked-out commit. These are deterministic task caches, not coverage-based selection or evidence of full qualification.

GitHub Actions always runs frontend tests and the frontend build. For frontend-only PRs and pushes, it skips Rust tests (including partition discovery), Clippy, and desktop builds. Frontend scope covers `src/` except the shared IPC bridge `src/api.ts`, `public/`, `index.html`, frontend `.test.tsx`/`.test.mjs` files in `scripts/`, and documentation/prototype changes. Rust files, Tauri files, shared/root configuration and lockfiles, CI/build/test tooling, and unrecognized paths require native validation. PR comparison uses the merge base; pushes compare their before/after commits. Deleted and renamed paths are included. An unavailable comparison defaults to native validation.

When native validation is required, CI runs cached fast Rust checks, uncached full workspace qualification, Clippy, and a desktop build. Release workflows explicitly set `force_full: true`; reusable CI defaults to full validation regardless of changed paths. The existing `macOS checks` gate requires frontend and scope detection to succeed, and accepts skipped native jobs only when scope detection explicitly selected that outcome. It restores Cargo artifacts and successful fast task results, and uploads per-crate JUnit reports for executed Rust tasks; cache hits are reported in the job log. Superseded PR runs are cancelled. Run `python3 scripts/ci-scope.test.py` when changing classification or gate behavior.

Opt-in live and filesystem qualification cases have their own setup in the relevant implementation records. Passing simulated tests does not establish native UI or live-provider compatibility.

## Build a local app

```sh
bun run desktop:build
```

This produces an optimized, ad-hoc signed app at `target/aarch64-apple-darwin/release/bundle/macos/GitJuke.app`, without Apple credentials or notarization. It does not launch or install the app. The build uses the pinned Rust toolchain and locked dependencies.

For a DMG signed with your own certificate, use `bun run desktop:sign`. For a signed and notarized DMG, use `bun run desktop:release`. See [Releasing and signing](releasing.md) for credentials, GitHub setup, and verification. Personal builds need no signing secrets.

## Local behavior

The native app stores its database in the Tauri application-data directory for `dev.hmrguez.gitjuke`. Closing the window hides the reader; Cmd-Q or tray Quit exits and cancels background work. Pause and appearance settings persist. Notifications and launch at login have separate opt-in controls.

Launch-at-login registration for a normal bundled copy requires `/Applications/GitJuke.app`. Moved or translocated copies show installation guidance. Development and qualification builds should retain the isolation described in their implementation records.
