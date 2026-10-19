//! Logical database/sidecar file sizes, separate from the disposable JSON budget.
//! Transient SQLite sort files are not measured; this is not a full peak-disk bound.
use crate::{
    Result,
    model::StorageUsage,
    store::{CONTENT_LIMIT, Store},
};
use std::path::Path;
pub const STORAGE_WARNING_BYTES: u64 = 1024 * 1024 * 1024;

fn file_bytes(path: &Path, suffix: &str) -> Result<u64> {
    let mut name = path.as_os_str().to_os_string();
    name.push(suffix);
    match std::fs::metadata(Path::new(&name)) {
        Ok(meta) => Ok(meta.len()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(0),
        Err(e) => Err(e.into()),
    }
}
fn files(path: &Path) -> Result<StorageUsage> {
    let database_bytes = file_bytes(path, "")?;
    let wal_bytes = file_bytes(path, "-wal")?;
    let shared_memory_bytes = file_bytes(path, "-shm")?;
    let journal_bytes = file_bytes(path, "-journal")?;
    let managed_bytes = database_bytes
        .saturating_add(wal_bytes)
        .saturating_add(shared_memory_bytes)
        .saturating_add(journal_bytes);
    Ok(StorageUsage {
        database_bytes,
        wal_bytes,
        shared_memory_bytes,
        journal_bytes,
        managed_bytes,
        transient_bytes: None,
        disposable_bytes: 0,
        content_limit_bytes: CONTENT_LIMIT as u64,
        warning_at_bytes: STORAGE_WARNING_BYTES,
        warning: managed_bytes >= STORAGE_WARNING_BYTES,
    })
}
impl Store {
    pub fn storage_usage(&self, path: &Path) -> Result<StorageUsage> {
        let mut usage = files(path)?;
        usage.disposable_bytes = crate::content_budget::disposable_bytes(&self.conn)? as u64;
        Ok(usage)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn counts_only_owned_database_sidecars_and_warns_at_one_gib() {
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("db");
        std::fs::write(&path, [0; 10]).unwrap();
        std::fs::write(d.path().join("db-wal"), [0; 20]).unwrap();
        std::fs::write(d.path().join("db-shm"), [0; 30]).unwrap();
        std::fs::write(d.path().join("unrelated"), [0; 100]).unwrap();
        let journal = std::fs::File::create(d.path().join("db-journal")).unwrap();
        journal.set_len(STORAGE_WARNING_BYTES - 61).unwrap(); // sparse, no GiB allocation
        let below = files(&path).unwrap();
        assert_eq!(below.managed_bytes, STORAGE_WARNING_BYTES - 1);
        assert!(!below.warning);
        assert!(below.transient_bytes.is_none());
        assert!(serde_json::to_value(&below).unwrap()["transientBytes"].is_null());
        journal.set_len(STORAGE_WARNING_BYTES - 60).unwrap();
        let at = files(&path).unwrap();
        assert_eq!(at.managed_bytes, STORAGE_WARNING_BYTES);
        assert!(at.warning);
        assert_eq!(
            (at.database_bytes, at.wal_bytes, at.shared_memory_bytes),
            (10, 20, 30)
        );
        std::fs::remove_file(d.path().join("db-wal")).unwrap();
        assert_eq!(files(&path).unwrap().wal_bytes, 0);
    }
}
