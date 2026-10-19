# Validation scope

Match local validation to the changed behavior. Use the fast checks below for everyday work. GitHub Actions runs full Rust qualification and desktop builds for Rust/shared configuration changes and releases; frontend-only changes skip native jobs. Unknown paths or unavailable comparisons conservatively retain native validation.

| Change | Local validation |
| --- | --- |
| UI labels, copy, styling, layout, or frontend logic | Always run `bun test` and `bun run build`. |
| Rust code or the frontend/native IPC contract | Run the relevant frontend checks when applicable, plus `bun run test:rust` or focused affected Rust tests. |
| Documentation or agent instructions only | Review the diff; executable tests are unnecessary. |

For frontend-only changes, keep validation to frontend checks. A label change must not trigger Rust tests, storage/crash qualification, or a desktop release build.

`bun run test:rust` is the cached fast Rust suite. Use `bun run test:affected` for working-tree changes and their dependent crates, or `bun run test:rust -p <crate>` for a deliberately narrowed check. `bun test` runs frontend tests; `bun run test` is a Rust-only alias.

Reserve `bun run test:full`, `bun run test:all`, `bun run test:qualification`, and unfiltered `cargo test` commands for CI, explicit user requests, or a concrete Rust failure/change that focused checks cannot adequately validate. Before expanding local validation, state the specific reason. Never run these commands merely as a habitual final check for UI/frontend work.

Once the relevant checks pass, stop testing unless further edits or a new failure justify another run. Report what was checked and distinguish fast-suite results from full qualification.

For test setup, cache behavior, and CI details, read [docs/development.md](docs/development.md) when changing the test workflow or troubleshooting its commands.
