# Settle specification handoff and release acceptance

Labels: wayfinder:grilling
Type: grilling
Status: resolved
Assignee: hmrguez
Parent: ../map.md
Blocked by: 09, 10

## Question

What exact criteria make the specification ready for production implementation, and what separate evidence must the resulting app satisfy before a first release? Consolidate the confirmed decisions into a traceable acceptance matrix with scenario, expected outcome, validation method and evidence status. Identify any remaining product decision rather than treating a future test as completed evidence.

Cover reader completeness/partial states and native-stack provider compatibility; notification classification, grouping, pause, seen state, privacy and crash attempts; fifty-watch scheduling/quota targets; transport bounds, revision races, retention/disk protection and migration/recovery; keyboard/screen-reader/compact-window/reduced-motion behavior; and the selected packaging/signing/update matrix. Consolidate approved measurable performance criteria, supported fixture/account permissions, required versus deferred cases, and how failed compatibility/performance gates trigger revisiting the spec. Reconcile the remaining proposed minimum window size and any newly discovered gap, preserving confirmed reading/packaging decisions.

## Context

Historical review rounds below retain their original wording. The final confirmation and Answer supersede their open/claimed/proposed statements.

Both prerequisite decisions, [Choose large-diff reading and performance limits](09-large-diff-behavior.md) and [Choose macOS packaging, signing, and distribution](10-packaging-signing-distribution.md), are now resolved. This ticket is open, unassigned and unblocked; it is not claimed by resolving packaging. The UI direction and runtime/cache policies are also resolved; their future verification is not. Existing browser/build evidence is fixture-only. Authenticated API and native macOS behaviors remain unverified. The planning destination is an implementation-ready specification and human-reviewed prototype; completing production tests or shipping an app is outside it. A settled spec may carry clearly stated future implementation/release gates without pretending they have passed.

### Frontier reconciliation — 2026-10-17

The large-diff prerequisite is now resolved, including up to 50 discussion messages per page and its detailed rendering/accessibility/performance acceptance contract. This ticket remains blocked only by the unresolved packaging/signing/distribution prerequisite; retain both dependency references so completed provenance remains visible. Consolidate the approved reader targets and boundary/fault matrix rather than reopening them as unspecified. The reference device is physical M1/8-GiB class; supported reference OS follows packaging. Record measurements as pending until native evidence exists. Minimum-window-size approval and final handoff/release criteria remain this ticket's decisions.

### Packaging resolved; final frontier — 2026-10-17

The later packaging resolution supersedes the preceding historical blocked status. Preserve both completed dependency references for provenance. All packaging policy is confirmed: arm64/macOS 15, M1/8-GiB/macOS-15 performance reference, signed/notarized/stapled personal DMG, dev.hmrguez.gitjuke identity, manual schema-aware updates, deterministic stored-gh config/no environment-token-only auth, and opt-in/default-off LaunchAgent/single-instance lifecycle. Consolidate its detailed native matrix without reopening those choices.

Remaining work in this ticket: decide specification-handoff completeness versus first-release evidence, settle the proposed 960×640 minimum with compact/zoom/accessibility behavior, define required/deferred coverage and failure/revision handling, and create the traceable scenario/expected-outcome/method/evidence-status matrix. Actual Developer ID access/team, trusted release-source URL, artifact checksum and exact tested OS/gh versions are future release inputs governed by the confirmed policy; they must not be fabricated or represented as passed evidence. Production implementation, signing, provisioning and native/API execution remain outside this planning effort. No new fog or child ticket was required by the packaging resolution.

### Claimed for final human review — 2026-10-17

Claimed by hmrguez. Historical open/unassigned wording above is superseded by the current metadata. All prerequisites are resolved; this is the sole remaining decision. The review uses wayfinder, grilling and domain-modeling, preserving all confirmed contracts. No new external platform fact is needed for the policy proposals below, and no native/API/auth/build operation is being run.

### Handoff, window, coverage and failure rules — proposals only, 2026-10-17

Review asset: [handoff and release acceptance draft](../acceptance.md). It indexes confirmed scenario outcomes into methods and current evidence, while labeling the new acceptance policy and minimum-window choice as proposed. Source decision tickets remain authoritative for their confirmed details; the matrix does not supersede numerical limits or imply implementation/test completion.

1. **Handoff threshold.** The specification becomes settled for implementation handoff after human approval of this final decision, resolution of every map child, no remaining in-scope fog/contradictory current statement, and complete traceability to acceptance scenarios/methods plus the approved prototype. Explicitly pending API/native/performance/signing/version evidence does not prevent planning handoff, but does prevent unsupported release claims. Module names, exact libraries/harnesses and governed release inputs remain implementation/release work if they preserve the contract. Closing the map does not start production work; subsequent implementation needs the user's instruction.

2. **Window and accessible layout.** Confirm a minimum reader content viewport of 960×640 logical pixels, excluding native title-bar chrome, with resizing above it. At compact width or 200% text zoom, use list/detail/back navigation rather than requiring every Stacks pane simultaneously. Essential controls/dialogs, reading modes and actions remain reachable without app-wide horizontal navigation scrolling, clipping, focus traps or forced focus loss. Code/long-content scrolling remains allowed. Preserve existing keyboard/VoiceOver, semantic paged reading, reduced-motion, font/palette/contrast-customization policies. This is a proposed native acceptance requirement, not a claim the fixture has passed it.

3. **Required coverage and honest evidence.** Treat all in-scope matrix scenarios as required for the first distributable v1, inheriting every detailed subcase from source decisions. Require public/authorized private reads, authored/direct/team-request/mention coverage, published native stack and nonmember, paginated conversation/reviews/checks and Actions/external outcomes; deterministic fixtures cover boundary/race/crash/denial cases, and native/API compatibility requires real authorized evidence. Missing permission/fixture/reference hardware/signing access is Blocked, not a synthetic Pass or silent feature cut. Existing agreed omissions remain out of scope; optional syntax highlighting need not be implemented, but if enabled must obey its confirmed budgets. Use Pending/Pass/Fail/Blocked per subcase with exact applicable build/schema/OS/hardware/gh/API/fixture/method/evidence link. Historical prototype approval/browser evidence remains separate from production evidence. Required negative-path fallbacks can pass when they match the contract, but cannot substitute for supported happy-path evidence.

4. **Failure/revision and release rule.** Once implementation is separately authorized, validate risky gh/API/native-stack/auth/process/login feasibility early, before extensive UI integration. Release requires required deterministic and authorized native/API/signed-artifact evidence and recorded signing/team/source/version inputs. Fail/Blocked on a required scenario prevents release under the current contract; fix it or return for an explicit named human-reviewed specification revision and update decisions/matrix/claims. Do not silently lower limits/coverage, drop team requests/native stacks or replace the benchmark with a faster machine. Re-run affected gates on build/schema/dependency/OS/identity changes; reusing unaffected evidence needs an applicability record. Existing qualified polling/delivery targets stay qualified, without inventing universal network/OS guarantees.

All four proposals await human review. The acceptance matrix is a concrete draft for that review; no test or product policy is auto-approved by writing it. The spec remains draft and this ticket remains claimed, with no final Answer. No production implementation, API/auth operation, signing, fixture provisioning or publication.

### Confirmed final acceptance and planning completion — 2026-10-17

The user explicitly agreed to all four recommendations and instructed recording, resolution, marking the specification settled and the wayfinder map complete, with no production implementation. This confirms the detailed handoff/window/coverage/failure round above and the linked acceptance policy/matrix. All earlier draft/proposal/claimed wording describes historical review state and is superseded.

## Answer

Resolved 2026-10-17 through human review. All four recommendations are confirmed in full:

- **Handoff:** all child decisions resolved, no remaining in-scope fog or contradictory current contract, and confirmed requirements traceable to the [acceptance matrix](../acceptance.md) and approved [prototype](../prototype/README.md). The [specification](../spec.md) is settled for implementation handoff and the [wayfinder map](../map.md) is complete. Engineering choices and governed release inputs may remain future work without changing the contract. Production implementation requires a subsequent user instruction and has not started.
- **Window/accessibility:** minimum reader content viewport 960×640 logical pixels excluding native title-bar chrome; resizing above it, compact/200%-zoom list/detail/back navigation, reachable essential actions/dialogs, no app-wide horizontal navigation scroll/clipping/focus traps. Preserve code scrolling, keyboard/VoiceOver, semantic paged reading, reduced motion and confirmed palette/customization policies. This confirms acceptance requirements, not a passed native layout test.
- **Required coverage/evidence:** all 29 indexed in-scope scenarios and their linked detailed subcases are required before a distributable v1 release, including supported positive flows and honest negative-path fallback. Record Pending/Pass/Fail/Blocked with applicable build/schema/OS/hardware/gh/API/fixture/procedure/evidence. Missing fixture/permissions/reference hardware/signing access is Blocked, not Pass or silent scope reduction. Existing omissions remain excluded; optional syntax enhancement is optional but bounded when enabled. Human prototype acceptance/historical fixture checks do not establish production API/native/performance/signing readiness.
- **Failure/revision:** after separate implementation authorization, validate high-risk gh/API/native-stack/auth/process/login feasibility early. Fail/Blocked required gates prevent release until corrected or an explicit human-reviewed specification revision updates the decision/matrix/claims. Re-run affected evidence after relevant changes; assess applicability before reusing other results. Do not silently relax requirements, coverage, targets or benchmark conditions. Preserve qualified scheduling/delivery guarantees rather than inventing universal ones.

No product decision or in-scope fog remains in this planning map. API/native/performance/storage-fault/accessibility/signed-artifact evidence and release inputs remain explicitly Pending future gates. No production code, app/auth/API test, signing, login registration, provisioning or publication was performed to resolve this ticket. Completion is the specification/prototype planning destination, not release certification or implementation authorization.
