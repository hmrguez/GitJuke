# Project status

GitJuke 0.1.0 is an incomplete development build. The repository is public; a signed installer and release-ready monitoring guarantees are not available yet.

## Implemented

The integrated app includes PR and review-request lists, personal Activity, saved searches, overview/stack reading, diffs, conversations, checks and job logs. Preferences includes appearance, text enlargement, monitoring, GitHub configuration, storage, notification controls, and launch at login. Recovery UI preserves local history when storage cannot safely start.

Implementation and qualification are separate. In particular, notification and login controls being present does not establish signed delivery, notification-click activation, or actual login behavior.

## Current verification

The [2026-10-18 integrated daily-use assessment](archive/implementation/2026-10-18-daily-qualification.md) records:

- 58 frontend and 343 Rust test entries passing, plus two opt-in mounted-filesystem cases.
- Static checks and both development app builds passing.
- Isolated native observations of reading, persistence, recovery Retry, minimum-size layouts, 200% text enlargement, and reduced motion.

These are recorded results for the examined build, not a claim that every future commit has passed them. Earlier checkpoints keep their original source, schema, fixture, and platform scope. Controlled synthetic-source tests and native qualification captures are distinct from real GitHub reads.

## Before release

The following remain unqualified or blocked in the integrated assessment:

- A clean installed-app smoke test using the normal account owner.
- VoiceOver and broader live GitHub/provider coverage.
- Performance on the specified physical reference device, including cumulative query work and peak/temporary storage.
- Signed notification permission, delivery, and click activation; actual login lifecycle.
- Developer ID signing, notarization, stapling, and distribution packaging. CI and a tagged draft-release pipeline are configured; the first credentialed GitHub run and downloaded-installer smoke test remain outstanding. See [release setup](releasing.md).

All 29 grouped scenarios in the [acceptance matrix](../.scratch/gitjuke/acceptance.md) remain Pending. Consult the dated engineering records for exact test scope and remaining work rather than treating individual passing checks as release certification.
