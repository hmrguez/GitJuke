# Project status

GitJuke 0.1.0 is an early development release. The repository is public and a Developer ID signed, notarized installer is available from [GitHub Releases](https://github.com/hmrguez/GitJuke/releases/tag/v0.1.0). Monitoring and installed-app behavior still need the qualification described below.

## Implemented

The integrated app includes PR and review-request lists, personal Activity, saved searches, overview/stack reading, diffs, conversations, checks and job logs. Preferences includes appearance, text enlargement, monitoring, GitHub configuration, storage, notification controls, and launch at login. Recovery UI preserves local history when storage cannot safely start.

Implementation and qualification are separate. In particular, notification and login controls being present does not establish signed delivery, notification-click activation, or actual login behavior.

Preferences → About and startup recovery now show the running binary's package version, build ID, Git revision/source state, target/profile and data-format version. These come from Rust at build time and remain available without an authenticated runtime or open database. See the [2026-10-19 build-information checkpoint](archive/implementation/2026-10-19-build-information.md) for scoped verification. The existing chronological Conversation timeline also supersedes the older assessment's independent-source-panel limitation.

## Current verification

The [2026-10-19 build-information checkpoint](archive/implementation/2026-10-19-build-information.md) records 89 passing frontend tests, 17 passing desktop Rust tests, real Tauri IPC-to-render checks, build-stamp invalidation, strict Clippy and local app builds. An isolated native recovery window displayed the binary metadata with monitoring stopped; the normal installed About/update flow remains unqualified. These scoped results do not replace the broader assessment below.

The [2026-10-18 integrated daily-use assessment](archive/implementation/2026-10-18-daily-qualification.md) records:

- 58 frontend and 343 Rust test entries passing, plus two opt-in mounted-filesystem cases.
- Static checks and both development app builds passing.
- Isolated native observations of reading, persistence, recovery Retry, minimum-size layouts, 200% text enlargement, and reduced motion.

These are recorded results for the examined build, not a claim that every future commit has passed them. Earlier checkpoints keep their original source, schema, fixture, and platform scope. Controlled synthetic-source tests and native qualification captures are distinct from real GitHub reads.

The [v0.1.0 release pipeline](https://github.com/hmrguez/GitJuke/actions/runs/39171077232) passed. The downloaded DMG's checksum and GitHub artifact attestation were verified, along with the bundled app's Developer ID signature, notarization staple, and Gatekeeper acceptance. This verification did not launch the installed app.

## Remaining qualification

The following remain unqualified or blocked in the integrated assessment:

- A clean installed-app smoke test using the normal account owner.
- Broader live GitHub/provider coverage.
- Performance on the specified physical reference device, including cumulative query work and peak/temporary storage.
- Signed notification permission, delivery, and click activation; actual login lifecycle.

All 29 grouped scenarios in the [acceptance matrix](../.scratch/gitjuke/acceptance.md) remain Pending. Consult the dated engineering records for exact test scope and remaining work rather than treating individual passing checks as release certification.

On 2026-10-19, the user removed accessibility work and qualification from the [wayfinder map](../.scratch/gitjuke/map.md). Historical accessibility blockers no longer gate this effort; existing app behavior is preserved.
