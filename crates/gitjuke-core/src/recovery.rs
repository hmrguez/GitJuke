//! Read-only startup diagnostics; never creates a database or attempts repair.
use rusqlite::{Connection, OpenFlags};
use serde::Serialize;
use std::path::Path;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum DatabaseState {
    Missing,
    Legacy,
    Current,
    Newer,
    Corrupt,
    Unreadable,
}
pub fn inspect(path: &Path) -> DatabaseState {
    match std::fs::metadata(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return DatabaseState::Missing,
        Ok(m) if m.is_file() => (),
        _ => return DatabaseState::Unreadable,
    }
    let result = (|| -> rusqlite::Result<(i64, bool)> {
        let c = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        c.busy_timeout(std::time::Duration::from_millis(250))?;
        let version = c.pragma_query_value(None, "user_version", |r| r.get(0))?;
        let check: String = c.query_row("PRAGMA quick_check(1)", [], |r| r.get(0))?;
        Ok((version, check == "ok"))
    })();
    match result {
        Ok((_, false)) => DatabaseState::Corrupt,
        Ok((v, true)) if v == crate::store::SCHEMA_VERSION => DatabaseState::Current,
        Ok((v, true)) if v > crate::store::SCHEMA_VERSION => DatabaseState::Newer,
        Ok((v, true)) if v >= 0 => DatabaseState::Legacy,
        Ok(_) => DatabaseState::Corrupt,
        Err(rusqlite::Error::SqliteFailure(e, _))
            if matches!(
                e.code,
                rusqlite::ErrorCode::DatabaseCorrupt | rusqlite::ErrorCode::NotADatabase
            ) =>
        {
            DatabaseState::Corrupt
        }
        Err(_) => DatabaseState::Unreadable,
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unreadable_posix_file_preserves_original_bytes_and_reports_recovery() {
        use std::os::unix::fs::PermissionsExt;
        assert_ne!(
            unsafe { libc::geteuid() },
            0,
            "run this permission fixture without root privileges"
        );
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("db");
        let c = Connection::open(&path).unwrap();
        c.pragma_update(None, "user_version", crate::store::SCHEMA_VERSION)
            .unwrap();
        drop(c);
        let before = std::fs::read(&path).unwrap();
        let permissions = std::fs::metadata(&path).unwrap().permissions();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o0)).unwrap();
        let state = inspect(&path);
        let rejected = crate::store::Store::open(&path).is_err();
        std::fs::set_permissions(&path, permissions).unwrap();
        assert_eq!(state, DatabaseState::Unreadable);
        assert!(rejected);
        assert_eq!(std::fs::read(&path).unwrap(), before);
    }
    #[test]
    fn invalid_negative_schema_is_rejected_without_creating_a_baseline() {
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("db");
        let c = Connection::open(&path).unwrap();
        c.pragma_update(None, "user_version", -1).unwrap();
        drop(c);
        let before = std::fs::read(&path).unwrap();
        assert!(crate::store::Store::open(&path).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), before);
    }
    #[test]
    fn diagnostics_preserve_missing_legacy_current_newer_corrupt_and_unreadable_inputs() {
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("db");
        assert_eq!(inspect(&path), DatabaseState::Missing);
        assert!(!path.exists());
        for (version, expected) in [
            (-1, DatabaseState::Corrupt),
            (1, DatabaseState::Legacy),
            (crate::store::SCHEMA_VERSION, DatabaseState::Current),
            (999, DatabaseState::Newer),
        ] {
            let c = Connection::open(&path).unwrap();
            c.pragma_update(None, "user_version", version).unwrap();
            drop(c);
            let before = std::fs::read(&path).unwrap();
            assert_eq!(inspect(&path), expected);
            assert_eq!(std::fs::read(&path).unwrap(), before);
        }
        std::fs::write(&path, b"original corrupt data").unwrap();
        assert_eq!(inspect(&path), DatabaseState::Corrupt);
        assert_eq!(std::fs::read(&path).unwrap(), b"original corrupt data");
        std::fs::remove_file(&path).unwrap();
        std::fs::create_dir(&path).unwrap();
        assert_eq!(inspect(&path), DatabaseState::Unreadable);
    }
}
