# Development

## Requirements

- An Apple Silicon Mac running macOS 15 or later.
- Xcode command-line tools and Rust installed through `rustup`. The repository pins its Rust toolchain in `rust-toolchain.toml`.
- Bun 1.4.2 or later.
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

| Command | Purpose |
| --- | --- |
| `bun run build` | TypeScript checks and frontend build |
| `bun test` | Frontend tests, including reader interactions and diff models |
| `bun run test:rust` | Rust core tests |
| `bun run test:all` | Frontend and Rust core suites |
| `cargo test --workspace --offline` | Workspace tests using locally available dependencies |
| `bun run check` | Frontend build and workspace Clippy with warnings treated as errors |

`bun run test` remains a Rust-only compatibility alias. Dependencies are locked in `bun.lock` and `Cargo.lock`. Node/npm are needed only to reproduce the legacy JavaScript benchmark; see the [Bun migration record](archive/implementation/2026-10-18-bun.md).

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
