# GitJuke documentation

Start with the guide that matches what you need:

| Guide | Contents |
| --- | --- |
| [Development](development.md) | Prerequisites, running locally, tests, and builds |
| [Architecture and data handling](architecture.md) | Native runtime, GitHub access, readers, storage, and recovery |
| [Project status](status.md) | Current verification, limitations, and release work |
| [Screenshots](screenshots/README.md) | Capture source and reproduction notes |
| [Design system](design/system.md) | Layout, typography, colors, and interaction conventions |
| [Design verification](design/verification.md) | Visual checks and observations |

## Engineering records

The dated files in [implementation/](implementation/) record individual changes, tests, and native observations. They are historical checkpoints: their schema versions, test counts, and open items describe the build examined at that time. For the integrated assessment, start with [daily-use qualification](implementation/2026-10-18-daily-qualification.md), then consult [project status](status.md).

Useful starting points:

- Readers: [Changes](implementation/2026-10-18-changes.md), [Conversation](implementation/2026-10-18-conversation.md), [Activity destinations](implementation/2026-10-18-destinations.md), and [job logs](implementation/2026-10-18-logs.md).
- Personal workflow: [saved queries](implementation/2026-10-18-saved-queries.md), [native alerts](implementation/2026-10-18-native-alerts.md), and [launch at login](implementation/2026-10-18-login-lifecycle.md).
- Reliability: [storage and recovery](implementation/2026-10-18-storage-recovery.md), [publication windows](implementation/2026-10-18-publication-windows.md), and [integration notes](implementation/2026-10-18-worktree-integration.md).
- Accessibility: [reader accessibility](implementation/2026-10-18-accessibility.md) and [keyboard navigation](implementation/2026-10-18-reader-navigation.md).

The [settled specification](../.scratch/gitjuke/spec.md), [acceptance matrix](../.scratch/gitjuke/acceptance.md), and [glossary](../GLOSSARY.md) describe the intended behavior and terminology. A specification requirement is not evidence that its release checks have passed.
