# Releasing and signing

GitJuke builds for Apple Silicon on macOS 15 or later. The public repository uses the standard `macos-15` GitHub-hosted runner. Official releases use the maintainer's private Apple credentials; personal builds and forks use the public build scripts with their own signing choices.

## One-time maintainer setup

1. Enroll in the Apple Developer Program. In Certificates, Identifiers & Profiles, create a **Developer ID Application** certificate for distribution outside the App Store, using a certificate signing request from your Mac. Install it in Keychain Access. It must include its private key.
2. In Keychain Access, export that identity and private key as a password-protected `.p12`. Keep the file and password outside the repository. Encode it into a file outside the checkout:

   ```sh
   openssl base64 -A -in /path/to/GitJuke.p12 -out /path/to/certificate-base64.txt
   security find-identity -v -p codesigning
   ```

3. At Apple's account site, generate an **app-specific password** for notarization. Find your Team ID in your developer membership details. Use the Apple ID belonging to that team; never use your normal account password here.
4. In this GitHub repository, open **Settings → Environments → New environment**, named exactly **`release`**. Under deployment branches and tags, choose selected branches and tags and add the **tag** pattern `v*`. Restrict tag creation/update/deletion with a repository ruleset to maintainers. Add yourself as a required environment reviewer if you want a manual gate before credentials are used.
5. Add these six **environment secrets** in `release`:

   | Secret | Value to enter |
   | --- | --- |
   | `APPLE_CERTIFICATE` | Entire contents of `certificate-base64.txt`; base64 is encoding, so treat this as a private key |
   | `APPLE_CERTIFICATE_PASSWORD` | Password chosen when exporting the `.p12` |
   | `APPLE_SIGNING_IDENTITY` | Full identity from `security find-identity`, e.g. `Developer ID Application: Your Name (ABCDEFGHIJ)`; omit the surrounding quotes |
   | `APPLE_ID` | Apple account email used for notarization |
   | `APPLE_PASSWORD` | Apple's generated app-specific password |
   | `APPLE_TEAM_ID` | Developer membership Team ID, e.g. `ABCDEFGHIJ` |

6. Enable GitHub Actions and allow the pinned `actions/checkout`, `oven-sh/setup-bun`, and `actions/attest-build-provenance` actions if your repository has an allowlist. Repository or organization policy must permit the release job's `contents: write`, `id-token: write`, and `attestations: write` permissions. The built-in `GITHUB_TOKEN` is supplied automatically; no personal access token is needed.
7. Protect `master` with the **macOS checks** status check once it has run. Review workflow and build-script changes as carefully as application changes. If the default branch changes, update the CI push filter too.

Never put credentials in a pull request, issue, shell history, tracked file, or chat. Contributor CI has read-only repository access and receives no environment secrets. The certificate is imported into a temporary runner keychain and removed in an always-run cleanup step; the runner is ephemeral. Signing credentials are available only to the relevant signing steps.

## Cut a release

1. Update the version consistently in `package.json`, `src-tauri/tauri.conf.json`, and `src-tauri/Cargo.toml`. Run `cargo check --workspace` to refresh the workspace package version in `Cargo.lock`, and commit that lockfile too. Check with `bun scripts/check-release.ts`.
2. Merge the release commit into `master` and wait for CI. From that commit, create and push the matching tag:

   ```sh
   git tag -a v0.1.0 -m "GitJuke 0.1.0"
   git push origin v0.1.0
   ```

   Use the actual version. Prerelease versions such as `0.2.0-beta.1` produce prerelease drafts.
3. The Release workflow checks that the tag matches all version files and compares the current version with every published GitHub release using semantic version ordering, including prereleases. Equal or lower versions skip CI and the release build. Drafts are excluded so failed draft builds can be retried; API failures or unrecognized published version tags stop the workflow. If eligible, it reruns the full CI workflow, checks that the tag is reachable from the default branch, imports the certificate, and builds the DMG. Tauri signs, notarizes, and staples the app. The shared build script checks the signature, Team ID, stapled ticket, and Gatekeeper assessment before any release upload.
4. After verification, the workflow computes SHA-256 checksums, generates GitHub build-provenance attestations for the DMG, and uploads the DMG plus `SHA256SUMS.txt` to a **draft** release with generated notes. Re-running the workflow can replace assets in that draft; it refuses to replace assets in a published release. For a published release, fix forward with a new version and tag.
5. Download the draft DMG, verify it, install the app in `/Applications`, and test launch, GitHub CLI account discovery, notifications and notification clicks, launch at login, quit, and relaunch on a normal account. Credentials and automated packaging do not establish native behavior. Keep the remaining [release qualification work](status.md) visible.
6. Edit the generated release notes to describe changes and known limitations, then click **Publish release**. No workflow automatically publishes the draft. GitHub supplies source archives for the tag.

If a build fails, inspect its Actions log, correct the cause, and re-run the failed workflow. First-time Apple notarization can exceed the job's 60-minute timeout; inspect Apple's notarization status before retrying. Never bypass verification or publish an unfinished draft to work around notarization failures.

## Build your own copy

Install the [development prerequisites](development.md), clone the repository, and check out the version tag you want. All scripts run from the checkout and use `Cargo.lock`; Bun dependencies must be installed with the frozen lockfile:

```sh
bun install --frozen-lockfile
bun run desktop:build
```

`desktop:build` uses ad-hoc signing (`APPLE_SIGNING_IDENTITY=-`) and explicitly clears Apple notarization and certificate-import variables in its subprocess. It needs no Apple account and produces `target/aarch64-apple-darwin/release/bundle/macos/GitJuke.app`. This signature does not establish a verified publisher or bypass Gatekeeper for downloaded copies. The command does not launch, install, or register the app at login.

The build script selects the pinned compiler even if Homebrew's Rust comes first in your shell. The release build also leaves host build dependencies unstripped to avoid the [Rust/LLVM dylib alignment issue on macOS 27](https://github.com/rust-lang/rust/issues/157750); this applies to compiler plugins, independently of the application signature.

To use your own Keychain certificate, find its identity and supply it for that command:

```sh
APPLE_SIGNING_IDENTITY="Developer ID Application: Your Name (ABCDEFGHIJ)" \
  bun run desktop:sign
```

This produces a DMG in `target/aarch64-apple-darwin/release/bundle/dmg/`. With notarization credentials present, Tauri also notarizes it. For a distribution build that requires notarization, set `APPLE_SIGNING_IDENTITY`, `APPLE_ID`, `APPLE_PASSWORD` (app-specific), and `APPLE_TEAM_ID` in your local environment using a secure credential manager, then run:

```sh
bun run desktop:release
```

Local signing uses the identity installed in your Keychain; the CI-only `.p12` secrets are unnecessary. Forks can configure the same workflow with their own six environment secrets. For an independently maintained app, change the bundle identifier and product name and update the scripts' `GitJuke.app` paths to avoid sharing application data and login registration with the official app.

## Verify downloads

From the directory containing the DMG and checksum file:

```sh
shasum -a 256 -c SHA256SUMS.txt
gh attestation verify ./GitJuke_0.1.0_aarch64.dmg --repo hmrguez/GitJuke
```

Use the actual downloaded filename and the fork's repository if applicable. Checksums detect corruption; attestations bind the asset to a GitHub build and source commit. Neither guarantees a byte-for-byte reproducible build. The Rust and Bun versions and dependencies are pinned; the GitHub macOS runner image and Xcode can still change.

After mounting the DMG, verify the enclosed app (substitute its actual volume path):

```sh
codesign --verify --deep --strict --verbose=2 /Volumes/GitJuke/GitJuke.app
xcrun stapler validate /Volumes/GitJuke/GitJuke.app
spctl --assess --type execute --verbose=2 /Volumes/GitJuke/GitJuke.app
```

Apple code signing and GitHub provenance are separate from Tauri updater signatures. GitJuke currently has no automatic updater; adding one later requires a separate updater key pair and an explicit fork/update-channel policy.

References: [Tauri macOS signing](https://v2.tauri.app/distribute/sign/macos/), [GitHub artifact attestations](https://docs.github.com/en/actions/concepts/security/artifact-attestations), [GitHub runner reference](https://docs.github.com/en/actions/reference/runners/github-hosted-runners).
