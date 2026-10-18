# macOS distribution and installed-gh constraints

Research for [Establish macOS packaging and gh launch constraints](../issues/12-macos-distribution-feasibility.md), 2026-10-17. Primary-source documentation review only. Recommendations below are **not confirmed product decisions**. No production builds, credentials, account changes, installation or signing occurred.

## Platform matrix

**Fact:** Tauri v2 documents `aarch64-apple-darwin` (Apple Silicon) and `x86_64-apple-darwin` (Intel) builds. Its app bundle supports configuring `minimumSystemVersion`; the documented default is macOS 10.13. That default does not prove compatibility of a particular dependency/plugin set or Apple Silicon hardware. [Tauri signing targets](https://v2.tauri.app/distribute/sign/macos/), [minimum system version](https://v2.tauri.app/distribute/macos-application-bundle/#minimum-system-version).

**Fact:** Apple's Sequoia compatibility list includes MacBook Air (M1, 2020). [Apple compatibility](https://support.apple.com/en-ae/120282).

**Recommendation/inference:** Apple Silicon only, macOS 15 minimum, is feasible as a deliberately narrow v1 support matrix. macOS 15 is a conservative product floor, not a Tauri technical requirement or a claim about Apple's future security-support policy. Benchmark on physical M1/8-GiB with a maintained macOS 15 patch and record the exact version. Qualify each additionally supported macOS major with packaged tests; “15 or later” must not promise untested future majors.

**Local observation:** Safe version-only checks returned macOS 27.0, arm64, gh 2.92.0 (2026-04-28). This is the local host's reported version, not verification of a public macOS release or support matrix. This machine does not supply evidence for the macOS 15 reference. No authentication or API payload was inspected.

## Direct DMG and signing

**Facts:** Tauri produces a macOS `.app` and supports DMG distribution. Developer ID Application is the certificate for outside-store apps; a free developer account cannot notarize. Certificate creation/access and notarization credentials are prerequisites whose availability here is unknown. [Tauri DMG](https://v2.tauri.app/distribute/dmg/), [Tauri signing](https://v2.tauri.app/distribute/sign/macos/).

Apple's notarization rules require Developer ID signatures for distributed executables, hardened runtime, and secure timestamps. Notarization is an automated security check, not App Review. Use supported `notarytool` tooling; `altool` uploads are no longer accepted. Stapling the ticket avoids relying on an online lookup during Gatekeeper evaluation. [Apple notarization](https://developer.apple.com/documentation/security/notarizing-macos-software-before-distribution), [Apple packaging/offline behavior](https://developer.apple.com/documentation/xcode/packaging-mac-software-for-distribution).

Mac App Store distribution requires App Sandbox. This is a separate constraint from hardened runtime; an app that runs installed external gh and consumes existing gh credentials would need a specific sandbox feasibility design and validation. Documentation does not establish that integration works in the store. [Apple App Sandbox](https://developer.apple.com/documentation/xcode/configuring-the-macos-app-sandbox).

**Recommendation/inference:** One Developer ID signed, notarized, stapled arm64 DMG containing GitJuke.app, copied into `/Applications`, with a stable bundle identity. Direct personal distribution avoids introducing store sandbox/review decisions. Signing access is a future release prerequisite, not permission to enroll or purchase now. Gatekeeper acceptance of the downloaded/quarantined artifact, offline first launch, and continued external-gh access remain unverified release gates. No security bypass should be the supported installation flow.

## Manual updates and database recovery

**Facts:** Tauri's updater requires update signatures that cannot be disabled, an embedded public key and secured private signing key; losing the private key prevents updates to installed clients trusting it. Production endpoints enforce TLS by default. The default comparator accepts newer versions; downgrade support can be configured. These are updater trust/version controls, not SQLite migration guarantees. [Tauri updater](https://v2.tauri.app/plugin/updater/).

**Recommendation/inference:** Manual replacement after explicit Quit suits personal v1: no automatic download/install, feed, updater key or automatic binary rollback. Distribute only through a documented trusted release location; checksums help identify artifacts but do not replace Developer ID/Gatekeeper verification. Preserve app data across replacement. Before migration, check schema compatibility; a future-schema app must stop without modifying the database. Prefer a forward corrective release. Reinstalling an older binary is safe only if its supported schema range includes the existing database. Restoring old ledgers can replay previously delivered alerts, so a pre-upgrade database restore must not be described as transparent rollback. Reconcile any restore with the confirmed alert-attempt ledger policy. Migration/recovery tests remain future acceptance gates.

## Installed gh from Finder and login

**Facts:** macOS GUI apps do not inherit PATH from shell dotfiles. [Tauri app bundle](https://v2.tauri.app/distribute/macos-application-bundle/). `GH_TOKEN`/`GITHUB_TOKEN` override stored credentials; `GH_HOST` selects a default host, `GH_REPO` can select a repository, and `GH_CONFIG_DIR`/`XDG_CONFIG_HOME` change configuration location. `GH_PROMPT_DISABLED` disables prompts. [gh environment](https://cli.github.com/manual/gh_help_environment).

`gh auth status --json` can exit zero despite authentication issues; parse its fields. `--show-token` exposes credentials and is unnecessary. `gh api --hostname` selects the request host. [gh auth status](https://cli.github.com/manual/gh_auth_status), [gh api](https://cli.github.com/manual/gh_api).

**Recommendation/inference:** Resolve a user-selected absolute executable path or known installation candidates (e.g. `/opt/homebrew/bin/gh`, `/usr/local/bin/gh`); validate and display the selected executable/version. Spawn with argv directly from Rust, no shell startup script or interpolated command. Persist the selected path, revalidate on launch and after executable changes. Use explicit github.com/repository targeting and a documented child environment. To use stored gh login consistently, remove inherited token/host/repository overrides; either use the default configuration context deliberately or offer an explicit configuration-directory choice. Do not silently discard an intentionally custom config and then report “logged out.” Disable interactive prompts and pagers. Do not call `gh auth token`, install/upgrade gh, switch accounts, or change credentials automatically.

An exact minimum gh version must follow the commands/fields actually used and a tested compatibility matrix, not a guessed current-version floor. The installed version is merely an observation. Official gh support does not prove stack-provider or custom-extension compatibility. Distinguish missing executable, unsupported command/version, config-context mismatch, unauthenticated account, permissions failure and unavailable provider; supply actionable setup guidance without changing them.

## Launch at login

**Facts:** Tauri's autostart plugin exposes enable/disable/status and a macOS LaunchAgent mode. Current source describes a plist under `~/Library/LaunchAgents`, started at login; it resolves the executable path. The alternative AppleScript mode manipulates System Events login items. [Plugin docs](https://v2.tauri.app/plugin/autostart/), [first-party implementation](https://raw.githubusercontent.com/tauri-apps/plugins-workspace/v2/plugins/autostart/src/lib.rs).

**Recommendation/inference:** LaunchAgent is a plausible implementation of the confirmed optional/default-off login setting, launching the same app with a background-start argument. It does not justify promising a daemon, wake from sleep, post-Quit monitoring, or automatic relaunch after a crash. Register only the installed stable app path, not a mounted DMG or translocated/download copy; show registration errors and reconcile externally disabled state. Verify on packaged supported macOS versions: login without a terminal, disable/re-enable, moved/missing app, manual replacement, single instance, menu-bar access, and explicit Quit stopping monitoring. Plugin documentation alone is not evidence those behaviors pass.
