use crate::{Identity, Settings};
use serde::{Deserialize, Serialize};

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
