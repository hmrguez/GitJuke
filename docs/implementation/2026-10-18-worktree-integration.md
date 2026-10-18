# Worktree integration — 2026-10-18

Integrated the accessibility, saved-query, storage-recovery, native-alert and login-lifecycle work into the primary `master` checkout, preserving its newer log and notification-route fixes. Sources were uncommitted copies of the initial commit; integration used the captured shared reader baseline and feature hunks. Original worktrees are retained. Pre-integration snapshots are stored locally under `/private/tmp/gitjuke-integration/snapshot-{0..5}.tar.gz`.

## Shared contracts

- Schema 15 retains the alert-dispatch index; schema 16 adds persistent saved queries. The final step accepts either independent schema-15 feature layout, preserves query rows and ensures both structures exist. Earlier feature evidence remains scoped to its recorded source/schema.
- Runtime dispatch retains Logs, Query, Activity, Changes and Conversation jobs, quota deferral and synchronous lifecycle cancellation. Queue access required by existing reader modules remains crate-visible.
- Recovery owns runtime initialization and Retry. Installation validation, single-instance ownership failure, opt-out background launch, Quit and invalidation still prevent startup. Background initialization uses `start_background`; successful startup installs the native alert sink and honors current reader visibility. Retry never resets a database.
- The shell retains notification routes, query navigation/editing/results, bounded Checks logs, text enlargement, scroll focus, Activity origin restoration and launch-at-login controls. The native window retains hidden startup with the corrected 672px minimum outer height.
- Bun remains the package manager. Old npm locks, temporary baseline copies, node_modules links and local qualification build overrides are excluded.

## Verification

- Production frontend build: pass. Bun: 51 passed, zero failed.
- Desktop Rust: 12 passed, zero failed. The sandbox initially denied Unix sockets; the same tests passed with local socket access. Tests use temporary namespaces.
- Workspace/all-target Clippy with warnings denied, Rust formatting and whitespace checks: pass.
- Core unit coverage: the first broad run passed 249 cases, failed five stale/integration fixtures and one migration child-process run and skipped two opt-in mounted-volume cases. All six failed cases subsequently pass in focused reruns: legacy schema-nine setup (one), authoritative workflow fixture (one), shared-accounting purge/cross-commit expectations (three), and the complete migration SIGKILL matrix (one). A new upgrade regression also passes for both independent version-15 database layouts. The final crash matrix passes; its initial run reported a child exiting before the after-commit marker. This is coverage across runs, not a claim that the initial broad invocation passed.
- Current migration group: all 11 passed, including per-step storage admission and crash boundaries. Purge group: all five passed with direct/dependent writes still capped at 128 and terminal history preserved.
- Core integration tests: 74 passed, zero failed, one authorized-live-network test ignored. All eight saved-query tests also pass in a focused final run. Doc-test target completed with no cases.

[evidence](evidence/2026-10-18-worktree-integration/) retains the initial broad failure and final focused runs. Historical fixtures now include authoritative suite IDs; expected purge checkpoints account for the existing shared-byte triggers rather than relaxing production budgets.

 No normal application database, credentials, login preferences, notification permissions or remote fixtures are changed. Historic native observations do not qualify the combined app; signed/native delivery, physical fault environments and the 29 grouped release scenarios retain their existing limits.

## Subsequent combined-source applicability

The [integrated daily-use assessment](2026-10-18-daily-qualification.md) records the current schema-16 source/artifact hashes, fresh full Rust regression, affected final frontend/static/native reruns and remaining release gates. This checkpoint retains its original feature schema, fixture and capture scope; instructions to integrate or rerun describe its historical handoff. No grouped acceptance row is promoted.
