//! Shared serialized reader and settings types; no database or process dependencies.
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Identity {
    pub id: u64,
    pub login: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Theme {
    pub mode: String,
    pub canvas: String,
    pub surface: String,
    pub accent: String,
    pub ink: String,
}
impl Default for Theme {
    fn default() -> Self {
        Self {
            mode: "dark".into(),
            canvas: "#121512".into(),
            surface: "#1c211b".into(),
            accent: "#b8d68a".into(),
            ink: "#e7ecdf".into(),
        }
    }
}
impl Theme {
    pub fn valid(&self) -> bool {
        ["light", "dark"].contains(&self.mode.as_str())
            && [&self.canvas, &self.surface, &self.accent, &self.ink]
                .iter()
                .all(|v| {
                    v.len() == 7
                        && v.starts_with('#')
                        && v[1..].bytes().all(|b| b.is_ascii_hexdigit())
                })
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    #[serde(default)]
    pub alerts: AlertSettings,
    #[serde(default)]
    pub paged_changes: bool,
    pub paused: bool,
    pub theme: Theme,
    pub gh_path: Option<String>,
    pub config_dir: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PullRequest {
    pub id: String,
    pub repo_id: Option<u64>,
    pub repo: String,
    pub number: u64,
    pub title: String,
    pub url: String,
    pub state: String,
    pub draft: bool,
    pub author: Option<String>,
    pub updated_at: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Overview {
    #[serde(default)]
    pub repo_id: Option<u64>,
    pub id: String,
    pub repo: String,
    pub number: u64,
    pub title: String,
    pub url: String,
    pub body: Option<String>,
    pub body_limited: bool,
    pub head: String,
    pub base: String,
    pub state: String,
    pub draft: bool,
    pub author: Option<String>,
    pub stack: StackState,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum StackState {
    Unknown {
        reason: String,
    },
    None,
    Published {
        number: u64,
        trunk: String,
        members: Vec<StackMember>,
    },
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StackMember {
    pub number: u64,
    pub title: String,
    pub url: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot<T> {
    pub data: T,
    pub fetched_at: Option<u64>,
    pub complete: bool,
    pub error: Option<String>,
    pub revision: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub settings: Settings,
    pub identity: Option<Identity>,
    pub locked: bool,
    pub health: String,
    pub storage_stopped: bool,
    pub storage: StorageUsage,
    pub gh_version: Option<String>,
    pub gh_path: Option<String>,
    pub config_dir: String,
    pub ignored_overrides: Vec<String>,
    pub revision: u64,
    pub queued: usize,
    pub busy: bool,
    pub next_remote_at: u64,
    pub discovery_more: bool,
    pub watch_count: usize,
    pub delayed_watches: usize,
    pub discovery: Vec<ResourceHealth>,
    pub budgets: Vec<Budget>,
    pub selected: Option<(String, u64)>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Page {
    pub items: Vec<PullRequest>,
    pub next_offset: Option<usize>,
    pub fetched_at: Option<u64>,
    pub complete: bool,
    pub error: Option<String>,
    pub revision: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourceHealth {
    pub resource: String,
    pub fetched_at: Option<u64>,
    pub complete: bool,
    pub error: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Budget {
    pub resource: String,
    pub remaining: u64,
    pub reset_at: u64,
    pub blocked_until: u64,
    pub requests: u64,
    pub payload_bytes: u64,
    pub cost: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WatchRow {
    pub id: String,
    pub repo: String,
    pub number: u64,
    pub reasons: Vec<String>,
    pub head: Option<String>,
    pub merge: Option<String>,
    pub refreshed_at: Option<u64>,
    pub due_at: u64,
    pub error: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WatchPage {
    pub items: Vec<WatchRow>,
    pub next_offset: Option<usize>,
    pub revision: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckPage {
    pub pr: Option<PullRequest>,
    pub error: Option<String>,
    pub review: Option<String>,
    pub items: Vec<CheckContext>,
    pub next_offset: Option<usize>,
    pub head: Option<String>,
    pub commits: Vec<ResourceHealth>,
    pub revision: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageUsage {
    pub database_bytes: u64,
    pub wal_bytes: u64,
    pub shared_memory_bytes: u64,
    pub journal_bytes: u64,
    pub managed_bytes: u64,
    /// SQLite can unlink temporary files while they remain open. Unknown is not zero.
    pub transient_bytes: Option<u64>,
    pub disposable_bytes: u64,
    pub content_limit_bytes: u64,
    pub warning_at_bytes: u64,
    pub warning: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub enum Permission {
    #[default]
    Unknown,
    NotDetermined,
    Denied,
    Granted,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct AlertSettings {
    pub onboarded: bool,
    pub enabled: bool,
    pub previews: bool,
    pub categories: [bool; 5],
    pub since: [u64; 5],
    pub permission: Permission,
    pub floor: i64,
}
impl Default for AlertSettings {
    fn default() -> Self {
        Self {
            onboarded: false,
            enabled: false,
            previews: false,
            categories: [true; 5],
            since: [0; 5],
            permission: Permission::Unknown,
            floor: 0,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckContext {
    pub id: String,
    pub kind: String,
    pub name: String,
    pub status: String,
    pub conclusion: Option<String>,
    pub url: Option<String>,
    pub suite_id: Option<String>,
    pub suite_status: Option<String>,
    pub suite_conclusion: Option<String>,
    pub suite_updated_at: Option<String>,
    #[serde(default)]
    pub source_created_at: Option<String>,
    pub run_id: Option<u64>,
    pub attempt: Option<u64>,
    pub commit: String,
}
