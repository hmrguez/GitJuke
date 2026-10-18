# GitJuke

A personal reader for pull requests and activity that involves the signed-in GitHub user.

## Language

**Local saved query**:
A named PR filter stored in GitJuke. It is independent of GitHub dashboard saved views.
_Avoid_: Synced preset, GitHub preset

**Stack**:
An ordered group of dependent pull requests registered as a native GitHub stack. GitJuke supports reading and navigating its published members.
_Avoid_: Branch chain, inferred stack

**Personal activity**:
Comments and reviews on the user's PRs, direct or team review requests, mentions on other PRs, and check or workflow completion related to the user's PRs.
_Avoid_: All repository activity

**GitHub notification**:
A GitHub discussion-thread notification used to discover personal activity. A thread notification can represent several actions.
_Avoid_: Event, individual comment

**Active watch set**:
The pull requests GitJuke follows for current PR and check state: open authored PRs, open direct or team review requests, and the selected open PR. Membership alone does not make every action eligible for an alert.
_Avoid_: Notification subscriptions, all visible PRs

**Native alert**:
An operating-system notification about eligible personal activity. It is distinct from the activity record shown in GitJuke and the GitHub notification used for discovery.
_Avoid_: GitHub notification, activity record
