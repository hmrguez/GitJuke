// Fictional records, held in memory. This prototype never calls GitHub.
export const prs = [
  { id: 183, repo: 'orbit/desktop', title: 'Keep notification state across restarts', author: 'you', time: '4m', mine: true, review: false, status: 'running', label: 'desktop', branch: 'feat/notification-store', base: 'feat/activity-feed', added: 126, removed: 34, comments: 3, unread: true, stack: true },
  { id: 184, repo: 'orbit/desktop', title: 'Add a quieter menu bar experience', author: 'you', time: '18m', mine: true, review: false, status: 'passed', label: 'desktop', branch: 'feat/menu-bar', base: 'feat/notification-store', added: 84, removed: 16, comments: 2, unread: true, stack: true },
  { id: 291, repo: 'orbit/api', title: 'Return cursor metadata with activity', author: 'marina', time: '26m', mine: false, review: true, status: 'passed', label: 'backend', branch: 'fix/activity-cursor', base: 'main', added: 47, removed: 12, comments: 2, unread: true },
  { id: 182, repo: 'orbit/desktop', title: 'Introduce the personal activity feed', author: 'you', time: '1h', mine: true, review: false, status: 'passed', label: 'desktop', branch: 'feat/activity-feed', base: 'main', added: 218, removed: 9, comments: 4, stack: true },
  { id: 76, repo: 'orbit/ui', title: 'Align keyboard focus across popovers', author: 'elliot', time: '2h', mine: false, review: true, team: true, status: 'failed', label: 'accessibility', branch: 'fix/popover-focus', base: 'main', added: 32, removed: 19, comments: 2 },
  { id: 289, repo: 'orbit/api', title: 'Handle empty team review requests', author: 'you', time: '3h', mine: true, review: false, status: 'passed', label: 'backend', branch: 'fix/team-requests', base: 'main', added: 23, removed: 7, comments: 1 },
  { id: 72, repo: 'orbit/ui', title: 'Refine the shared command palette', author: 'sam', time: '4h', mine: false, review: false, status: 'passed', label: 'design', branch: 'feat/commands', base: 'main', added: 61, removed: 24, comments: 1, unread: true },
];
export const activity = [
  { id: 'a1', pr: 183, person: 'Marina', initials: 'MC', type: 'comment', text: 'left a comment', excerpt: 'Could we keep the watermark until the whole page has been written?', time: '4m', tab: 'Conversation' },
  { id: 'a2', pr: 291, person: 'Marina', initials: 'MC', type: 'review', text: 'requested your review', excerpt: 'A small fix for the activity cursor. Ready for another pair of eyes.', time: '26m', tab: 'Overview' },
  { id: 'a3', pr: 184, person: 'Checks', initials: '✓', type: 'check', text: 'all checks passed', excerpt: 'Build, typecheck, and tests completed successfully.', time: '32m', tab: 'Checks' },
  { id: 'a4', pr: 76, person: 'UI team', initials: 'UI', type: 'review', text: 'was asked to review', excerpt: 'Your team has a new review request.', time: '2h', tab: 'Overview' },
  { id: 'a5', pr: 72, person: 'Sam', initials: 'SL', type: 'mention', text: 'mentioned you', excerpt: '@you — does this cover the keyboard behavior we discussed?', time: '4h', tab: 'Conversation' },
];
export const stack = [182, 183, 184];
export const fileNames = ['src-tauri/src/activity/store.rs', 'src-tauri/src/activity/sync.rs', 'src/hooks/useActivity.ts'];
export const diffLines = [
  ['context', 'use crate::activity::{Activity, Watermark};'],
  ['context', 'use rusqlite::Connection;'],
  ['context', ''],
  ['context', 'impl ActivityStore {'],
  ['remove', '    pub fn save(&self, activity: &[Activity]) -> Result<()> {'],
  ['add', '    pub fn commit_page('],
  ['add', '        &mut self,'],
  ['add', '        activity: &[Activity],'],
  ['add', '        watermark: &Watermark,'],
  ['add', '    ) -> Result<()> {'],
  ['add', '        let tx = self.connection.transaction()?;'],
  ['context', '        for item in activity {'],
  ['remove', '            self.insert(item)?;'],
  ['add', '            Self::insert(&tx, item)?;'],
  ['context', '        }'],
  ['add', '        Self::save_watermark(&tx, watermark)?;'],
  ['add', '        tx.commit()?;'],
  ['context', '        Ok(())'],
  ['context', '    }'],
  ['context', '}'],
];
