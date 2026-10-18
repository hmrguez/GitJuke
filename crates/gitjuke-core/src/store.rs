//! Content is disposable. Identity/configuration/seen/progress/attempts are not.
use crate::{Error, Result, model::*, now};
use rusqlite::{Connection, OptionalExtension, params};
use std::path::Path;
pub const SCHEMA_VERSION: i64 = 12;
pub const CONTENT_LIMIT: i64 = 500 * 1024 * 1024;
pub const MIN_FREE_BYTES: u64 = 1024 * 1024 * 1024;
pub struct Store {
    pub(crate) conn: Connection,
    pub(crate) cache_writes: bool,
}
impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        storage_healthy(path.parent().unwrap_or(Path::new("/")))?;
        let mut conn = Connection::open(path)?;
        conn.busy_timeout(std::time::Duration::from_secs(2))?;
        let version: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
        if version > SCHEMA_VERSION {
            return Err(Error::Safe(
                "This database requires a newer GitJuke. Its schema was not modified.".into(),
            ));
        }
        let check: String = conn.query_row("PRAGMA quick_check(1)", [], |r| r.get(0))?;
        if check != "ok" {
            return Err(Error::Safe("The database failed integrity checking. Original files are preserved; monitoring is stopped.".into()));
        }
        crate::migrations::upgrade(&mut conn, version, path.parent().unwrap_or(Path::new("/")))?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        crate::publication::resume(&mut conn)?;
        crate::pr_purge::resume(&mut conn)?;
        crate::check_purge::resume(&mut conn)?;
        crate::clear_content::resume(&mut conn)?;
        crate::invalidation::resume(&mut conn)?;
        crate::control_sweep::resume(&mut conn)?;
        crate::startup_cleanup::cleanup(&mut conn, path.parent().unwrap_or(Path::new("/")))?;
        Ok(Self {
            conn,
            cache_writes: true,
        })
    }
    /// Open an existing compatible cache without migrations or startup writes.
    /// Authentication must still be validated before private content is exposed.
    pub(crate) fn open_cached(path: &Path) -> Result<Self> {
        let conn = Connection::open_with_flags(
            path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        conn.busy_timeout(std::time::Duration::from_secs(2))?;
        let version: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
        if version != SCHEMA_VERSION {
            return Err(Error::StorageUnavailable("Storage protection cannot open this schema without migration. Free disk space and reopen GitJuke; original files are preserved.".into()));
        }
        let check: String = conn.query_row("PRAGMA quick_check(1)", [], |r| r.get(0))?;
        if check != "ok" {
            return Err(Error::Safe("The database failed integrity checking. Original files are preserved; monitoring is stopped.".into()));
        }
        crate::publication::protect(&conn)?;
        Ok(Self {
            conn,
            cache_writes: false,
        })
    }
    pub fn revision(&self) -> Result<u64> {
        self.meta("revision")
    }
    pub fn generation(&self) -> Result<u64> {
        self.meta("generation")
    }
    fn meta(&self, key: &str) -> Result<u64> {
        Ok(self.conn.query_row(
            "SELECT CAST(value AS INTEGER) FROM meta WHERE key=?",
            [key],
            |r| r.get::<_, i64>(0).map(|v| v as u64),
        )?)
    }
    pub fn settings(&self) -> Result<Settings> {
        let v: Option<String> = self
            .conn
            .query_row("SELECT json FROM settings WHERE id=1", [], |r| r.get(0))
            .optional()?;
        v.map(|s| serde_json::from_str(&s).map_err(Error::from))
            .unwrap_or_else(|| Ok(Settings::default()))
    }
    pub fn save_settings(&mut self, s: &Settings) -> Result<()> {
        let tx = self.conn.transaction()?;
        tx.execute(
            "INSERT INTO settings VALUES(1,?) ON CONFLICT(id) DO UPDATE SET json=excluded.json",
            [serde_json::to_string(s)?],
        )?;
        bump(&tx)?;
        tx.commit()?;
        Ok(())
    }
    pub fn identity(&self) -> Result<Option<Identity>> {
        Ok(self
            .conn
            .query_row("SELECT user_id,login FROM identity WHERE id=1", [], |r| {
                Ok(Identity {
                    id: r.get::<_, i64>(0)? as u64,
                    login: r.get(1)?,
                })
            })
            .optional()?)
    }
    pub fn bind(&mut self, id: &Identity) -> Result<()> {
        if let Some(bound) = self.identity()?
            && bound.id != id.id
        {
            return Err(Error::Safe("The active gh account differs from the configured stable identity. Private content is locked. Restore the original gh account externally.".into()));
        }
        if self.identity()?.as_ref() == Some(id) {
            return Ok(());
        }
        let tx = self.conn.transaction()?;
        tx.execute("INSERT INTO identity VALUES(1,'github.com',?,?) ON CONFLICT(id) DO UPDATE SET login=excluded.login",params![i64::try_from(id.id).map_err(|_|Error::Safe("Unsupported identity ID".into()))?,id.login])?;
        bump(&tx)?;
        tx.commit()?;
        Ok(())
    }
    pub fn invalidate(&mut self) -> Result<()> {
        self.publication_ready()?;
        crate::pr_purge::resume(&mut self.conn)?;
        crate::check_purge::resume(&mut self.conn)?;
        crate::clear_content::resume(&mut self.conn)?;
        crate::invalidation::invalidate(&mut self.conn)
    }
    pub fn clear(&mut self) -> Result<()> {
        self.publication_ready()?;
        crate::pr_purge::resume(&mut self.conn)?;
        crate::check_purge::resume(&mut self.conn)?;
        crate::invalidation::resume(&mut self.conn)?;
        crate::clear_content::clear(&mut self.conn)
    }
    pub fn purge(&mut self, key: &str) -> Result<()> {
        self.publication_ready()?;
        if let Some(locator) = key.strip_prefix("pr:") {
            let (repo, number) = locator
                .rsplit_once(':')
                .ok_or_else(|| Error::Safe("Invalid PR purge locator.".into()))?;
            let number = number
                .parse()
                .map_err(|_| Error::Safe("Invalid PR purge number.".into()))?;
            crate::pr_purge::purge(&mut self.conn, repo, number, None)?;
            return Ok(());
        }
        self.invalidate()?;
        let tx = self.conn.transaction()?;
        tx.execute("DELETE FROM content WHERE key=?", [key])?;
        tx.execute("INSERT INTO resource_state(key,complete,error) VALUES(?,0,'Resource unavailable') ON CONFLICT(key) DO UPDATE SET complete=0,error=excluded.error",[key])?;
        bump(&tx)?;
        tx.commit()?;
        Ok(())
    }
    pub fn error(&mut self, key: &str, message: &str) -> Result<()> {
        let tx = self.conn.transaction()?;
        tx.execute("INSERT INTO resource_state(key,complete,error) VALUES(?,0,?) ON CONFLICT(key) DO UPDATE SET error=excluded.error",params![key,message])?;
        bump(&tx)?;
        tx.commit()?;
        Ok(())
    }
    pub fn put<T: serde::Serialize>(
        &mut self,
        key: &str,
        value: &T,
        generation: u64,
    ) -> Result<bool> {
        self.publication_ready()?;
        if self.generation()? != generation || crate::pr_purge::payload_hidden(&self.conn, key)? {
            return Ok(false);
        }
        let json = serde_json::to_string(value)?;
        if json.len() > 1024 * 1024 {
            return Err(Error::Safe(
                "Detail exceeds the current IPC limit; use its GitHub source.".into(),
            ));
        }
        let time = now();
        crate::content_budget::publish_bounded_observed(
            &mut self.conn,
            time,
            |tx| {
                tx.execute("INSERT INTO content VALUES(?,?,?,?) ON CONFLICT(key) DO UPDATE SET json=excluded.json,fetched=excluded.fetched,accessed=excluded.accessed",params![key,json,time as i64,time as i64])?;
                tx.execute("INSERT INTO resource_state VALUES(?,?,1,NULL) ON CONFLICT(key) DO UPDATE SET fetched=excluded.fetched,complete=1,error=NULL",params![key,time as i64])?;
                bump(tx)?;
                Ok(())
            },
            || {
                #[cfg(test)]
                crate::content_capacity::point("before_commit");
            },
        )?;
        #[cfg(test)]
        crate::content_capacity::point("after_commit");
        Ok(true)
    }
    pub fn get<T: serde::de::DeserializeOwned>(
        &mut self,
        key: &str,
    ) -> Result<Snapshot<Option<T>>> {
        let cleared = crate::clear_content::pending(&self.conn)?;
        if cleared
            || crate::pr_purge::payload_hidden(&self.conn, key)?
            || crate::check_purge::payload_hidden(&self.conn, key)?
        {
            return Ok(Snapshot {
                data: None,
                fetched_at: None,
                complete: false,
                error: Some(
                    if cleared {
                        crate::clear_content::MESSAGE
                    } else if crate::pr_purge::payload_hidden(&self.conn, key)? {
                        crate::pr_purge::MESSAGE
                    } else {
                        crate::check_purge::MESSAGE
                    }
                    .into(),
                ),
                revision: self.revision()?,
            });
        }
        let row: Option<(String, u64)> = self
            .conn
            .query_row(
                "SELECT json,fetched FROM content WHERE key=? AND accessed>=?",
                params![key, now().saturating_sub(30 * 86400) as i64],
                |r| Ok((r.get(0)?, r.get::<_, i64>(1)? as u64)),
            )
            .optional()?;
        let state: Option<(bool, Option<String>)> = self
            .conn
            .query_row(
                "SELECT complete,error FROM resource_state WHERE key=?",
                [key],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        if self.cache_writes && row.is_some() {
            self.conn.execute(
                "UPDATE content SET accessed=? WHERE key=?",
                params![now() as i64, key],
            )?;
        }
        let (data, fetched_at) = match row {
            Some((s, t)) => (Some(serde_json::from_str(&s)?), Some(t)),
            None => (None, None),
        };
        Ok(Snapshot {
            complete: data.is_some() && state.as_ref().is_some_and(|s| s.0),
            data,
            fetched_at,
            error: state.and_then(|s| s.1),
            revision: self.revision()?,
        })
    }
    pub fn start_authored(&mut self) -> Result<()> {
        self.begin_source("authored")
    }
    pub fn stage_authored(&mut self, prs: &[PullRequest], generation: u64) -> Result<bool> {
        if self.generation()? != generation {
            return Ok(false);
        }
        self.stage_source("authored", prs)?;
        Ok(true)
    }
    pub fn finish_authored(&mut self, generation: u64) -> Result<bool> {
        if self.generation()? != generation {
            return Ok(false);
        }
        self.publish_source("authored")?;
        Ok(true)
    }
    pub fn page(&mut self, offset: usize) -> Result<Page> {
        self.list_page("authored", offset)
    }
    pub fn due(&self, key: &str) -> Result<u64> {
        Ok(self
            .conn
            .query_row("SELECT at FROM due WHERE key=?", [key], |r| {
                r.get::<_, i64>(0)
            })
            .optional()?
            .unwrap_or(0) as u64)
    }
    pub fn set_due(&self, key: &str, at: u64) -> Result<()> {
        self.conn.execute(
            "INSERT INTO due VALUES(?,?) ON CONFLICT(key) DO UPDATE SET at=excluded.at",
            params![key, at as i64],
        )?;
        Ok(())
    }
    pub fn checkpoint(&self) -> Result<()> {
        let (busy, frames, written): (i64, i64, i64) =
            self.conn
                .query_row("PRAGMA wal_checkpoint(PASSIVE)", [], |r| {
                    Ok((r.get(0)?, r.get(1)?, r.get(2)?))
                })?;
        if busy != 0 || frames != written {
            return Err(Error::StorageUnavailable("Local storage checkpoint could not complete; monitoring stopped until storage is healthy.".into()));
        }
        Ok(())
    }
    pub(crate) fn storage_probe(&mut self) -> Result<()> {
        // A successful read is not evidence that writes/commits work again.
        let tx = self.conn.transaction()?;
        bump(&tx)?;
        tx.commit()?;
        self.checkpoint()
    }
}
pub(crate) fn bump(tx: &rusqlite::Transaction) -> Result<()> {
    tx.execute(
        "UPDATE meta SET value=CAST(value AS INTEGER)+1 WHERE key='revision'",
        [],
    )?;
    Ok(())
}
#[cfg(test)]
pub(crate) use crate::content_budget::evict;
pub fn storage_healthy(path: &Path) -> Result<()> {
    let path = std::ffi::CString::new(path.as_os_str().as_encoded_bytes())
        .map_err(|_| Error::StorageUnavailable("Invalid storage path.".into()))?;
    let mut stat = std::mem::MaybeUninit::<libc::statvfs>::uninit();
    if unsafe { libc::statvfs(path.as_ptr(), stat.as_mut_ptr()) } != 0 {
        return Err(Error::StorageUnavailable(
            "Cannot verify available storage; monitoring stopped.".into(),
        ));
    }
    let stat = unsafe { stat.assume_init() };
    if (stat.f_bavail as u64).saturating_mul(stat.f_frsize) < MIN_FREE_BYTES {
        return Err(Error::StorageUnavailable("Less than 1 GiB is free. Remote ingestion is stopped; cached reading remains available.".into()));
    }
    Ok(())
}

#[cfg(test)]
mod storage_fault_tests {
    use super::*;
    #[test]
    fn read_only_list_expiry_is_incomplete_and_preserves_membership() {
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("db");
        let mut s = Store::open(&path).unwrap();
        let g = s.generation().unwrap();
        let pr = PullRequest {
            id: "PR_1".into(),
            repo_id: Some(1),
            repo: "a/b".into(),
            number: 1,
            title: "Cached title".into(),
            url: "https://github.com/a/b/pull/1".into(),
            state: "open".into(),
            draft: false,
            author: Some("reader".into()),
            updated_at: "2026-10-17T00:00:00Z".into(),
        };
        s.start_authored().unwrap();
        s.stage_authored(&[pr], g).unwrap();
        s.finish_authored(g).unwrap();
        assert!(s.page(0).unwrap().complete);
        s.conn.execute("UPDATE content SET accessed=0", []).unwrap();
        drop(s);
        let mut s = Store::open_cached(&path).unwrap();
        let page = s.page(0).unwrap();
        assert!(page.items.is_empty());
        assert!(
            !page.complete,
            "eviction/expiry cannot assert a successful empty source"
        );
        assert_eq!(s.watch_page(0).unwrap().items.len(), 1);
    }
    #[test]
    fn sqlite_full_rolls_back_and_read_only_cache_filters_expired_payloads() {
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("db");
        let mut s = Store::open(&path).unwrap();
        let identity = Identity {
            id: 42,
            login: "reader".into(),
        };
        s.bind(&identity).unwrap();
        let g = s.generation().unwrap();
        s.put("old", &"cached", g).unwrap();
        s.put("expired", &"expired", g).unwrap();
        s.conn
            .execute("UPDATE content SET accessed=0 WHERE key='expired'", [])
            .unwrap();
        let revision = s.revision().unwrap();
        let pages: i64 = s
            .conn
            .pragma_query_value(None, "page_count", |r| r.get(0))
            .unwrap();
        s.conn.pragma_update(None, "max_page_count", pages).unwrap();
        let err = s.put("large", &"x".repeat(900_000), g).unwrap_err();
        assert!(
            matches!(err, Error::Storage(rusqlite::Error::SqliteFailure(e,_)) if e.code == rusqlite::ErrorCode::DiskFull)
        );
        assert_eq!(s.revision().unwrap(), revision);
        assert_eq!(s.identity().unwrap(), Some(identity));
        assert_eq!(
            s.get::<String>("old").unwrap().data.as_deref(),
            Some("cached")
        );
        assert!(s.get::<String>("large").unwrap().data.is_none());
        drop(s);
        let mut readonly = Store::open_cached(&path).unwrap();
        assert!(readonly.get::<String>("expired").unwrap().data.is_none());
        assert_eq!(
            readonly.get::<String>("old").unwrap().data.as_deref(),
            Some("cached")
        );
        assert_eq!(readonly.revision().unwrap(), revision);
        // Read-only expiry filters without deleting durable or disposable rows.
        assert_eq!(
            readonly
                .conn
                .query_row(
                    "SELECT COUNT(*) FROM content WHERE key='expired'",
                    [],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
            1
        );
    }
}
