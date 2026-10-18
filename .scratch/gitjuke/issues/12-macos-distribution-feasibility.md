# Establish macOS packaging and gh launch constraints

Labels: wayfinder:research
Type: research
Status: resolved
Assignee: packaging_research
Parent: ../map.md
Blocked by: none

## Question

Which current primary-source facts constrain GitJuke's macOS distribution decision: Tauri v2 arm64/x86_64 and deployment targets; direct DMG versus store distribution, Developer ID signing/notarization/stapling and prerequisites; manual versus built-in updater trust and schema rollback limitations; installed-gh discovery/auth/environment from GUI/login launch; and launch-at-login integration for a same-process menu-bar app?

Assess a proposed personal Apple-Silicon-only release with a conservative macOS minimum, signed/notarized direct DMG and manual updates. Separate requirements from product recommendations, and theoretical documentation support from tested compatibility. Do not infer official gh stack provider compatibility from generic gh support. Record relevant uncertainty; no production execution or account provisioning.

## Context

Research for [Choose macOS packaging, signing, and distribution](10-packaging-signing-distribution.md). Follow the research skill; primary sources only, findings in [macOS distribution research](../research/macos-distribution.md). No Git repository exists; keep a local Markdown asset rather than creating a branch. A physical M1/8-GiB-class reader performance reference is confirmed; reference OS remains to select. Scope is one GitHub.com identity and installed gh, not credentials installation/account changes.

## Answer

Resolved 2026-10-17 by delegated research following the research skill. [macOS distribution research](../research/macos-distribution.md) records primary-source facts, product inferences and unverified gates separately.

Tauri supports separate Apple Silicon/Intel targets and a configurable macOS minimum; Apple lists M1 hardware as compatible with macOS 15. Choosing arm64/macOS 15 is a product/test scope, not a framework requirement. Direct Developer ID signing/notarization requires appropriate Apple Developer Program access, hardened runtime/timestamps and valid signatures; stapling supports offline Gatekeeper evaluation. Direct DMG is documented; store sandbox feasibility for external gh is not established. Manual updates avoid updater signing-feed maintenance but do not solve schema downgrade safety.

GUI/login launches do not inherit shell-dotfile PATH. gh token, host, repo and config-directory environment overrides affect authentication/target context; safe executable discovery and a deliberate child environment are needed. gh auth-status JSON fields must be interpreted rather than relying only on exit status, and tokens must not be printed. Current Tauri autostart docs/source provide a LaunchAgent route, but packaged native behavior remains unverified.

Version-only local observations do not prove a public macOS release/support matrix or authenticated API/provider compatibility. No private data/auth check, installation, signing, purchase, build or production implementation occurred. These findings unblock the product discussion; the parent ticket's platform/distribution recommendations still require human approval.
