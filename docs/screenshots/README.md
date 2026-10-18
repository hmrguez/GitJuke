# App screenshots

Captured on 2026-10-18 from the installed native development app at `/Applications/GitJuke.app`, version 0.1.0, at 1280 × 800 in Graphite. These are actual app captures with public GitHub data, not mockups or synthetic qualification fixtures. The installed copy was updated from `target/debug/bundle/macos/GitJuke.app` before these captures; it includes the simplified readers without hunk navigation controls.

| Image | Content |
| --- | --- |
| [Overview](overview.png) | A saved search for `repo:cli/cli is:open`, with [cli/cli#14727](https://github.com/cli/cli/pull/14727) selected and its real description visible |
| [Changes](changes.png) | The same search, with [cli/cli#14719](https://github.com/cli/cli/pull/14719) selected and the all-files diff reader visible |

To reproduce, run the native app with an existing GitHub CLI login, save that public search, select the PR, and open Overview or Changes. For the diff capture, open Changes, wait for the real patches to load, and scroll the reader to the changed lines. Before each capture, move the pointer into the native title bar, outside the retained app content. PR state, search counts, and content will change over time.

The search was created locally for these captures and remains available in the app. No GitHub content was created or modified. The visible account handle is public; the captured PRs and code are public too. The images are cropped to the app interior (1272 × 750), removing the macOS title bar, screen-sharing indicator, and rounded outer corners. The retained app content is unchanged; crops are saved as PNG to avoid further JPEG compression.
