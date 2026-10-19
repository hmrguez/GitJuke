//! A small resource barrier hides denied Changes while disposable rows are reclaimed.
use crate::{
    Error, Result,
    store::{Store, bump},
};
use rusqlite::{OptionalExtension, params};
pub(crate) fn root(repo: &str, number: u64) -> String {
    format!("api:repos/{repo}/pulls/{number}/gitjuke-changes/")
}
fn marker(repo: &str, number: u64) -> String {
    format!("changes-denied:{repo}:{number}")
}
impl Store {
    /// Small cache commits amortize fsync while keeping accounting, admission and
    /// the 128-mutation guard in the existing publisher. Inputs are encoded IPC rows.
    pub(crate) fn put_changes_batch(
        &mut self,
        rows: &[(String, String)],
        generation: u64,
    ) -> Result<()> {
        if rows.is_empty() {
            return Ok(());
        }
        if rows.len() > 12 || rows.iter().any(|(_, json)| json.len() > 1024 * 1024) {
            return Err(Error::Safe(
                "Changes cache batch exceeds its bounds.".into(),
            ));
        }
        self.publication_ready()?;
        if self.generation()? != generation {
            return Ok(());
        }
        for (key, _) in rows {
            if crate::pr_purge::payload_hidden(&self.conn, key)? {
                return Ok(());
            }
        }
        let time = crate::now();
        crate::content_budget::publish_bounded(&mut self.conn, time, |tx| {
            for (key, json) in rows {
                tx.execute("INSERT INTO content VALUES(?,?,?,?) ON CONFLICT(key) DO UPDATE SET json=excluded.json,fetched=excluded.fetched,accessed=excluded.accessed", params![key,json,time as i64,time as i64])?;
                tx.execute("INSERT INTO resource_state VALUES(?,?,1,NULL) ON CONFLICT(key) DO UPDATE SET fetched=excluded.fetched,complete=1,error=NULL", params![key,time as i64])?;
            }
            bump(tx)?;
            Ok(())
        })
    }

    pub(crate) fn changes_denied(&self, repo: &str, number: u64) -> Result<bool> {
        Ok(self
            .conn
            .query_row(
                "SELECT 1 FROM resource_state WHERE key=?",
                [marker(repo, number)],
                |_| Ok(()),
            )
            .optional()?
            .is_some())
    }
    pub(crate) fn deny_changes(&mut self, repo: &str, number: u64) -> Result<()> {
        let tx = self.conn.transaction()?;
        tx.execute("INSERT INTO resource_state(key,complete,error) VALUES(?,0,'pending') ON CONFLICT(key) DO UPDATE SET error='pending'",[marker(repo,number)])?;
        tx.execute(
            "UPDATE meta SET value=CAST(value AS INTEGER)+1 WHERE key='generation'",
            [],
        )?;
        bump(&tx)?;
        tx.commit()?;
        self.clean_changes(repo, number)
    }
    pub(crate) fn clean_changes(&mut self, repo: &str, number: u64) -> Result<()> {
        let prefix = root(repo, number);
        let end = format!("{}0", prefix.trim_end_matches('/'));
        loop {
            let tx = self.conn.transaction()?;
            let before = tx.total_changes();
            // Content accounting triggers share the direct mutation allowance.
            let deleted=tx.execute("DELETE FROM content WHERE key IN (SELECT key FROM content WHERE key>=? AND key<? ORDER BY key LIMIT 32)",params![prefix,end])?;
            if deleted == 0 {
                tx.execute(
                    "UPDATE resource_state SET error='denied' WHERE key=? AND error!='denied'",
                    [marker(repo, number)],
                )?;
            }
            bump(&tx)?;
            if tx.total_changes() - before > 128 {
                return Err(Error::StorageUnavailable(
                    "Changes cleanup exceeded its mutation allowance.".into(),
                ));
            }
            tx.commit()?;
            if deleted == 0 {
                break;
            }
        }
        Ok(())
    }
    pub(crate) fn restore_changes(&mut self, repo: &str, number: u64) -> Result<()> {
        let tx = self.conn.transaction()?;
        if tx.execute(
            "DELETE FROM resource_state WHERE key=?",
            [marker(repo, number)],
        )? > 0
        {
            bump(&tx)?;
        }
        tx.commit()?;
        Ok(())
    }
    pub(crate) fn resume_changes_cleanup(&mut self) -> Result<()> {
        let mut after = "changes-denied:".to_owned();
        loop {
            let rows: Vec<(String, String)> = {
                let mut q=self.conn.prepare("SELECT key,error FROM resource_state WHERE key>? AND key<'changes-denied;' ORDER BY key LIMIT 32")?;
                q.query_map([&after], |r| Ok((r.get(0)?, r.get(1)?)))?
                    .collect::<rusqlite::Result<_>>()?
            };
            if rows.is_empty() {
                break;
            }
            for (key, state) in &rows {
                if state != "pending" {
                    continue;
                }
                let Some((repo, number)) =
                    key.trim_start_matches("changes-denied:").rsplit_once(':')
                else {
                    return Err(Error::Safe("Invalid Changes cleanup marker.".into()));
                };
                self.clean_changes(
                    repo,
                    number
                        .parse()
                        .map_err(|_| Error::Safe("Invalid Changes cleanup number.".into()))?,
                )?;
            }
            after = rows.last().unwrap().0.clone();
            if rows.len() < 32 {
                break;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn patch_cache_batches_are_bounded_atomic_and_generation_checked() {
        let d = tempfile::tempdir().unwrap();
        let mut s = Store::open(&d.path().join("db")).unwrap();
        let generation = s.generation().unwrap();
        let rows: Vec<_> = (0..12)
            .map(|n| (format!("batch-{n:02}"), "\"patch\"".into()))
            .collect();
        s.conn.execute_batch("CREATE TRIGGER fail_batch BEFORE INSERT ON content WHEN NEW.key='batch-11' BEGIN SELECT RAISE(ABORT,'fixture batch failure'); END;").unwrap();
        let revision = s.revision().unwrap();
        assert!(s.put_changes_batch(&rows, generation).is_err());
        assert_eq!(s.revision().unwrap(), revision);
        assert_eq!(
            s.conn
                .query_row(
                    "SELECT COUNT(*) FROM content WHERE key LIKE 'batch-%'",
                    [],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
            0
        );
        s.conn.execute_batch("DROP TRIGGER fail_batch").unwrap();
        s.put_changes_batch(&rows, generation + 1).unwrap();
        assert!(s.get::<String>("batch-00").unwrap().data.is_none());
        s.put_changes_batch(&rows, generation).unwrap();
        assert_eq!(
            s.get::<String>("batch-11").unwrap().data.as_deref(),
            Some("patch")
        );
        let mut too_many = rows.clone();
        too_many.push(("batch-12".into(), "null".into()));
        assert!(s.put_changes_batch(&too_many, generation).is_err());
    }

    #[test]
    fn denied_changes_remain_hidden_through_partial_cleanup_and_restart() {
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("db");
        let mut s = Store::open(&path).unwrap();
        let g = s.generation().unwrap();
        let prefix = root("a/b", 1);
        for i in 0..257 {
            s.put(&format!("{prefix}{i:04}"), &"patch", g).unwrap();
        }
        s.put("pr:a/b:1", &"permitted overview", g).unwrap();
        s.put(
            "api:repos/a/b/pulls/10/gitjuke-changes/keep",
            &"neighbor",
            g,
        )
        .unwrap();
        s.conn.execute_batch("CREATE TRIGGER fail_changes BEFORE DELETE ON content WHEN OLD.key LIKE '%gitjuke-changes/0140' BEGIN SELECT RAISE(ABORT,'fixture late cleanup'); END;").unwrap();
        assert!(s.deny_changes("a/b", 1).is_err());
        assert!(s.changes_denied("a/b", 1).unwrap());
        let left: i64 = s
            .conn
            .query_row(
                "SELECT COUNT(*) FROM content WHERE key>=? AND key<?",
                params![prefix, format!("{}0", prefix.trim_end_matches('/'))],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(left, 129); // Four 32-row transactions survive; the next rolls back in full.
        assert_eq!(
            s.get::<String>("pr:a/b:1").unwrap().data.as_deref(),
            Some("permitted overview")
        );
        drop(s);
        assert!(Store::open(&path).is_err());
        let raw = rusqlite::Connection::open(&path).unwrap();
        raw.execute_batch("DROP TRIGGER fail_changes").unwrap();
        drop(raw);
        let mut s = Store::open(&path).unwrap();
        assert!(s.changes_denied("a/b", 1).unwrap());
        assert_eq!(
            s.get::<String>("api:repos/a/b/pulls/10/gitjuke-changes/keep")
                .unwrap()
                .data
                .as_deref(),
            Some("neighbor")
        );
        let left: i64 = s
            .conn
            .query_row(
                "SELECT COUNT(*) FROM content WHERE key>=? AND key<?",
                params![prefix, format!("{}0", prefix.trim_end_matches('/'))],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(left, 0);
        s.restore_changes("a/b", 1).unwrap();
        assert!(!s.changes_denied("a/b", 1).unwrap());
    }
}
