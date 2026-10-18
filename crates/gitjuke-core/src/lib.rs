pub mod activity;
#[cfg(test)]
mod activity_crash;
mod activity_enrollment;
mod activity_store;
mod admission_query;
mod check_purge;
mod clear_content;
mod content_budget;
#[cfg(test)]
mod content_capacity;
mod control_sweep;
pub mod github;
mod invalidation;
mod migrations;
pub mod model;
mod pr_purge;
mod publication;
mod recovery_cleanup;
mod retention;
mod retention_window;
pub mod runtime;
mod runtime_activity;
mod runtime_sync;
mod source_cleanup;
mod startup_cleanup;
mod storage_usage;
pub mod store;
pub mod sync;
pub mod transport;
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
pub fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod activity_bounded;

#[cfg(test)]
mod maintenance_tests;

#[cfg(test)]
mod query_work_tests;
