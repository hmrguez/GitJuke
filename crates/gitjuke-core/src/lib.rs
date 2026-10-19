//! Runtime and persistence interfaces for the desktop reader.
//! Explicit module paths preserve public imports and test-suite ownership.
pub use gitjuke_github::{activity, changes, conversation, github, sync, transport};
pub use gitjuke_model as model;
#[cfg(test)]
#[path = "activity/activity_crash.rs"]
mod activity_crash;
#[path = "activity/activity_due.rs"]
mod activity_due;
#[path = "activity/activity_enrollment.rs"]
mod activity_enrollment;
#[path = "activity/activity_store.rs"]
mod activity_store;
#[path = "storage/content/admission_query.rs"]
mod admission_query;
#[path = "features/alerts.rs"]
pub mod alerts;
#[path = "features/changes_store.rs"]
mod changes_store;
#[path = "storage/maintenance/check_purge.rs"]
mod check_purge;
#[path = "storage/maintenance/clear_content.rs"]
mod clear_content;
#[path = "storage/content/content_budget.rs"]
mod content_budget;
#[cfg(test)]
#[path = "storage/content/content_capacity.rs"]
mod content_capacity;
#[path = "storage/maintenance/control_sweep.rs"]
mod control_sweep;
#[path = "storage/maintenance/invalidation.rs"]
mod invalidation;
#[path = "storage/migrations.rs"]
mod migrations;
#[path = "watch/pending_selection.rs"]
mod pending_selection;
#[path = "storage/maintenance/pr_purge.rs"]
mod pr_purge;
#[path = "storage/publication/publication.rs"]
mod publication;
#[path = "storage/maintenance/recovery_cleanup.rs"]
mod recovery_cleanup;
#[path = "storage/maintenance/retention.rs"]
mod retention;
#[path = "storage/maintenance/retention_window.rs"]
mod retention_window;
#[path = "runtime/runtime.rs"]
pub mod runtime;
#[path = "runtime/runtime_activity.rs"]
mod runtime_activity;
#[path = "runtime/runtime_alerts.rs"]
mod runtime_alerts;
#[path = "runtime/runtime_changes.rs"]
mod runtime_changes;
#[path = "runtime/runtime_sync.rs"]
mod runtime_sync;
#[path = "storage/maintenance/source_cleanup.rs"]
mod source_cleanup;
#[path = "storage/maintenance/startup_cleanup.rs"]
mod startup_cleanup;
#[path = "storage/storage_usage.rs"]
mod storage_usage;
#[path = "storage/store.rs"]
pub mod store;
#[path = "watch/watch_reader.rs"]
mod watch_reader;
#[path = "watch/watch_reservations.rs"]
mod watch_reservations;
#[path = "watch/watch_scheduling.rs"]
mod watch_scheduling;
#[path = "watch/watch_store.rs"]
pub mod watch_store;

pub type Result<T> = std::result::Result<T, Error>;
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(
        "This resource is unavailable to the configured identity; existing membership is retained."
    )]
    Denied,
    #[error("{0}")]
    Safe(String),
    #[error("Local storage failed; monitoring stopped. The original database is preserved.")]
    Storage(#[from] rusqlite::Error),
    #[error("{0}")]
    StorageUnavailable(String),
    #[error("The GitHub response has an unsupported or incomplete schema.")]
    Schema(#[from] serde_json::Error),
    #[error("The local operation failed ({0}).")]
    Io(std::io::ErrorKind),
}
impl From<std::io::Error> for Error {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value.kind())
    }
}
impl From<gitjuke_github::Error> for Error {
    fn from(value: gitjuke_github::Error) -> Self {
        match value {
            gitjuke_github::Error::Denied => Self::Denied,
            gitjuke_github::Error::Safe(message) => Self::Safe(message),
            gitjuke_github::Error::Schema(error) => Self::Schema(error),
            gitjuke_github::Error::Io(kind) => Self::Io(kind),
        }
    }
}
pub fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
#[path = "activity/activity_bounded.rs"]
mod activity_bounded;

#[cfg(test)]
#[path = "storage/maintenance/maintenance_tests.rs"]
mod maintenance_tests;

#[cfg(test)]
#[path = "features/query_work_tests.rs"]
mod query_work_tests;

#[path = "storage/shared_storage.rs"]
mod shared_storage;

#[path = "storage/publication/publication_completion.rs"]
mod publication_completion;

#[path = "features/conversation_store.rs"]
mod conversation_store;
#[path = "runtime/runtime_comments.rs"]
mod runtime_comments;
#[path = "runtime/runtime_conversation.rs"]
mod runtime_conversation;

#[path = "features/logs.rs"]
pub mod logs;
#[path = "features/saved_queries.rs"]
pub mod saved_queries;

#[cfg(test)]
#[path = "storage/physical_storage_tests.rs"]
mod physical_storage_tests;
#[path = "recovery/recovery.rs"]
pub mod recovery;
