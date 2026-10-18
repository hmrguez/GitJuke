# GitJuke

A read-only macOS GitHub PR reader and personal activity monitor built with Tauri, React, Rust, SQLite and the installed `gh` CLI.

Production implementation started on 2026-10-17 under the settled specification. The runtime reads the existing stored gh identity, reconciles open authored/direct/team/selected watches, schedules batched head/check snapshots reads PR overviews/published native stacks, discovers notification threads and hydrates individual personal Activity with durable baseline/recovery and seen/dedup state. The production app has no fixture fallback. This is an incomplete development build, not a release-ready monitor: native alerts/permission/category controls, external suite retry classification, saved queries, diffs, discussions, logs and login remain to be implemented; full recovery/release qualification is pending.

## Develop

Apple Silicon, macOS 15+, Rust, Xcode command-line tools, Node/npm and an existing stored GitHub.com `gh` login are required. GitJuke neither installs gh nor changes its credentials. The default config directory is the OS account's `~/.config/gh`, independently of shell token/XDG overrides. Preferences accepts explicit absolute executable/configuration paths and revalidates the originally bound stable account.

```sh
npm ci
npm run tauri dev
```

`npm run dev` alone opens a browser frontend without a native runtime; it reports that limitation and supplies no sample PRs. The native app stores its database in the Tauri application-data directory for `dev.hmrguez.gitjuke`. Close hides the reader; Cmd-Q or tray Quit cancels/reaps remote work and exits. Pause and four-color appearance settings persist. Startup transient cleanup and recovery initialization each commit at most 128 direct row mutations per cleanup transaction and resume safely after interruption before monitoring starts. Recovery commits its boundary before batched resets, preserving it through partial failure. Watch-derived Activity enrollment adds at most 128 missing targets per scheduling pass; recovery cannot complete while eligible watches await enrollment. Full-PR denial unifies overview/Activity/watch purging behind a durable generation/visibility barrier and cleanup transactions of at most 128 mutations; affected discovery restarts before publishing, while durable history and unrelated caches survive. Check-access denial commits a per-PR epoch barrier, hides denied contexts and check Activity excerpts, and resumes cleanup in transactions of at most 128 direct mutations during startup/repair. Unrelated PR content and durable history survive. Clear downloaded content commits a logical/generation barrier, hides pending payloads and resumes cleanup in transactions of at most 128 direct mutations during startup/repair, preserving durable history and deadlines. Global invalidation commits a generation barrier before restartable 128-row cleanup; pending invalidation makes cached checks incomplete and resumes during startup/repair. Existing current-schema databases can start in storage protection; validated sessions retain cached navigation when writes fail. First-page PR discovery initialization commits a source restart guard before deleting old staging in transactions of at most 128 direct mutations; unfinished initialization blocks staging/publication and recovery completion while cached membership remains readable. Source/watch/check publication now uses complete transactions capped at 128 direct mutations including admission; small operations commit once, while larger replacements use a durable old-snapshot journal, bounded application and one visibility commit. Startup/repair finish interrupted publication before eligibility; exact retries preserve membership and epochs. Activity staging initialization also uses bounded cleanup and staging commits; Activity publication and recovery completion share the 128-mutation maximum, retaining one transaction for small sets and journaled atomic visibility for larger histories/summaries. Protected Activity/seen/dedup/progress/cycle/intent/member reads prevent premature completion; startup/repair finish interrupted publication before abandonment cleanup or remote eligibility. All eight payload writers use shared transactional admission, reserving publication writes and revision from the same 128 allowance. Admission adds at most 128 mutations per candidate; larger reclamation first rolls back publication, then commits independent LRU batches capped at 128 total mutations including check invalidation and revision, before retrying atomic publication. Staging and durable history remain protected; earlier disposable reclamation may survive a later publication failure. Full-size synthetic fixtures now qualify functional admission and publication/fault behavior at the actual 500-MiB shared limit; worst-case query, temp-space and performance bounds remain pending. Check snapshots enforce 30-day access expiry and report incomplete state after eviction. Periodic retention also runs while paused. One global allowance caps each retention transaction at 128 direct mutations, including dependent Activity/payload/catch-up deletion, check invalidation, validator/API-health/obsolete-check reclamation and revision. Logical expiration applies while physical cleanup waits; expired loaded snapshots cannot mark Activity seen. Head-reservation retry cleanup, abandoned Activity retry staging and manual watch-due resets now use durable scoped sweeps capped at 128 mutations including final bookkeeping; startup/repair finish them before scheduling, preserving cached payloads, deadlines and durable history. Retention selection now uses indexed ranges and fenced 128-candidate keyset windows with atomic cursor/deletion commits; zero-sort/automatic-index plans are qualified against large protected prefixes. Publication application uses a partial unapplied-row index. Shared admission now totals bytes through snapshot-consistent 128-row windows and merges indexed content/check windows into an exact bounded LRU prefix, with no global SQLite victim sort. Compact counter/fault/restart fixtures qualify these statements; constant-time totals, sublinear check selection, cumulative admission time, publication planning, deep reader/queue queries, variable metadata bytes and total temp space remain unqualified. Preferences reports database/sidecar sizes separately from disposable content, with a 1-GiB measured-file warning; transient sort-file accounting remains pending. Activity denial purges use literal PR path boundaries so neighboring PRs retain their caches. Notification permission is never requested and login registration is never created in this slice.

```sh
npm run build
npm test
cargo test --workspace --offline
npm run check
npm run tauri build -- --debug --bundles app --target aarch64-apple-darwin
```

The last command produces a **local development app**, not the signed/notarized/stapled release DMG. No signing credentials, membership, publication or distribution have been provisioned. Development version 0.1.0 now reads schema 12 and transactionally upgrades schema 1/2/3/4/5/6/7/8/9/10/11, with full SQLite synchronization before migration writes and free-space checks before every upgrade step and commit; unknown future or corrupt schemas stop with original files preserved. There is no updater or automatic destructive reset.

## Evidence and next slices

- [Current shared admission byte-total/LRU query-window evidence](docs/implementation/2026-10-18-admission-windows.md) — compact scoped verification; large fixtures deliberately not replayed
- [Previous indexed retention query-window and publication-application evidence](docs/implementation/2026-10-18-query-windows.md)
- [Previous global retention/metadata maintenance and control-plane sweep evidence](docs/implementation/2026-10-18-global-maintenance.md)
- [Previous bounded Activity publication/recovery completion and schema-10 evidence](docs/implementation/2026-10-18-bounded-activity-publication.md)
- [Previous bounded source/watch/check publication and schema-9 evidence](docs/implementation/2026-10-17-bounded-publication.md)
- [Previous bounded shared LRU admission evidence](docs/implementation/2026-10-17-bounded-lru-admission.md)
- [Previous bounded first-page source initialization evidence](docs/implementation/2026-10-17-bounded-source-initialization.md)
- [Previous bounded full-PR resource purge and schema-8 evidence](docs/implementation/2026-10-17-bounded-pr-purge.md)
- [Previous bounded check-access denial and schema-7 evidence](docs/implementation/2026-10-17-bounded-check-denial.md)
- [Previous bounded Clear downloaded content and schema-6 evidence](docs/implementation/2026-10-17-bounded-clear.md)
- [Previous bounded invalidation and schema-5 evidence](docs/implementation/2026-10-17-bounded-invalidation.md)
- [Previous full-size content-budget qualification](docs/implementation/2026-10-17-full-content-capacity.md)
- [Previous bounded Activity enrollment and evidence](docs/implementation/2026-10-17-activity-enrollment.md)
- [Previous migration storage admission and crash evidence](docs/implementation/2026-10-17-migration-durability.md)
- [Previous Activity storage integrity and crash evidence](docs/implementation/2026-10-17-activity-crash-durability.md)
- [Previous bounded recovery initialization and evidence](docs/implementation/2026-10-17-recovery-initialization.md)
- [Previous bounded startup reclamation and evidence](docs/implementation/2026-10-17-startup-reclamation.md)
- [Previous disposable metadata reclamation and evidence](docs/implementation/2026-10-17-metadata-reclamation.md)
- [Previous published check reclamation and evidence](docs/implementation/2026-10-17-check-reclamation.md)
- [Previous shared content-budget admission and evidence](docs/implementation/2026-10-17-content-budget.md)
- [Previous bounded retention maintenance and evidence](docs/implementation/2026-10-17-retention-maintenance.md)
- [Previous startup/cache storage availability and evidence](docs/implementation/2026-10-17-storage-availability.md)
- [Previous recovery/storage hardening and evidence](docs/implementation/2026-10-17-recovery-hardening.md)
- [Previous Activity implementation and evidence](docs/implementation/2026-10-17-activity-slice.md)
- [Previous watch/check implementation and evidence](docs/implementation/2026-10-17-watch-slice.md)
- [Historical first-slice checkpoint](docs/implementation/2026-10-17-first-slice.md)
- [Confirmed specification](.scratch/gitjuke/spec.md)
- [29-scenario acceptance matrix](.scratch/gitjuke/acceptance.md)
- [Completed decision map](.scratch/gitjuke/map.md)
- [Glossary](GLOSSARY.md)

The accepted fixture UI is preserved under `.scratch/gitjuke/prototype`; `npm run prototype` and `npm run prototype:build` still run it separately. Its browser evidence is historical and fixture-only. Production reuses its bundled JetBrains Mono and approved Carbon/Parchment four-color tokens, with Stacks as the reader structure. Production release gates remain open.
