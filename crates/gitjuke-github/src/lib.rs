//! GitHub protocol parsing and bounded gh subprocess transport.
//! Independent of SQLite, runtime scheduling, and the native desktop shell.
pub use gitjuke_model as model;
pub mod activity;
pub mod changes;
pub mod conversation;
pub mod github;
pub mod sync;
pub mod transport;

pub type Result<T> = std::result::Result<T, Error>;
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(
        "This resource is unavailable to the configured identity; existing membership is retained."
    )]
    Denied,
    #[error("{0}")]
    Safe(String),
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
