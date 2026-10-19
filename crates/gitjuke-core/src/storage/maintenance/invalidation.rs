//! A durable generation barrier precedes restartable invalidation cleanup.
use crate::{
    Result,
    store::{bump, storage_healthy},
};
use rusqlite::Connection;
use std::path::{Path, PathBuf};

const BATCH: usize = 128;
fn directory(c: &Connection) -> PathBuf {
    c.path()
        .and_then(|p| Path::new(p).parent())
        .unwrap_or(Path::new("/"))
        .to_path_buf()
}
pub(crate) fn pending(c: &Connection) -> Result<bool> {
    Ok(
        c.query_row("SELECT EXISTS(SELECT 1 FROM invalidation)", [], |r| {
            r.get(0)
        })?,
    )
}
fn begin(c: &mut Connection) -> Result<()> {
    if pending(c)? {
        return Ok(());
    }
    storage_healthy(&directory(c))?;
    let tx = c.transaction()?;
    tx.execute("INSERT INTO invalidation VALUES(1)", [])?;
    tx.execute(
        "UPDATE meta SET value=CAST(value AS INTEGER)+1 WHERE key='generation'",
        [],
    )?;
    bump(&tx)?;
    tx.commit()?;
    #[cfg(test)]
    point("barrier_committed");
    Ok(())
}
pub(crate) fn invalidate(c: &mut Connection) -> Result<()> {
    begin(c)?;
    resume(c)
}
pub(crate) fn resume(c: &mut Connection) -> Result<()> {
    while pending(c)? {
        storage_healthy(&directory(c))?;
        batch(c)?;
    }
    Ok(())
}
fn batch(c: &mut Connection) -> Result<usize> {
    let tx = c.transaction()?;
    let batch_before = tx.total_changes();
    let mut changed = 0;
    let mut exhausted = false;
    for sql in [
        "DELETE FROM stage WHERE rowid IN (SELECT rowid FROM stage ORDER BY rowid LIMIT ?)",
        "DELETE FROM activity_stage WHERE rowid IN (SELECT rowid FROM activity_stage ORDER BY rowid LIMIT ?)",
        "DELETE FROM source_stage WHERE rowid IN (SELECT rowid FROM source_stage ORDER BY rowid LIMIT ?)",
        "DELETE FROM teams_stage WHERE rowid IN (SELECT rowid FROM teams_stage ORDER BY rowid LIMIT ?)",
        "DELETE FROM pending WHERE rowid IN (SELECT rowid FROM pending ORDER BY rowid LIMIT ?)",
        "DELETE FROM check_context WHERE rowid IN (SELECT rowid FROM check_context WHERE staged=1 ORDER BY rowid LIMIT ?)",
        "UPDATE check_state SET complete=0,error='Content invalidated' WHERE rowid IN (SELECT rowid FROM check_state WHERE complete!=0 OR error IS NOT 'Content invalidated' ORDER BY rowid LIMIT ?)",
    ] {
        let limit = (BATCH - changed) / crate::shared_storage::cost(sql);
        if limit == 0 {
            exhausted = true;
            break;
        }
        let before = tx.total_changes();
        let rows = tx.execute(sql, [limit as i64])?;
        changed += (tx.total_changes() - before) as usize;
        if rows == limit {
            exhausted = true;
            break;
        }
    }
    // Marker removal shares the transaction allowance. Until it commits,
    // readers treat all checks as incomplete, including untouched later rows.
    let finished = !exhausted;
    if finished {
        changed += tx.execute("DELETE FROM invalidation", [])?;
    }
    #[cfg(test)]
    point(if finished {
        "finish_before_commit"
    } else {
        "batch_before_commit"
    });
    crate::shared_storage::bounded(&tx, batch_before)?;
    tx.commit()?;
    #[cfg(test)]
    point(if finished {
        "finish_after_commit"
    } else {
        "batch_after_commit"
    });
    Ok(changed)
}
#[cfg(test)]
fn point(phase: &str) {
    if std::env::var("GITJUKE_TEST_INVALIDATION_PHASE").as_deref() == Ok(phase) {
        std::fs::write(
            std::env::var_os("GITJUKE_TEST_INVALIDATION_MARKER").unwrap(),
            phase,
        )
        .unwrap();
        loop {
            std::thread::park();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{model::Identity, now, store::Store};
    fn n(c: &Connection, sql: &str) -> i64 {
        c.query_row(sql, [], |r| r.get(0)).unwrap()
    }
    fn seed(s: &mut Store) {
        s.bind(&Identity {
            id: 42,
            login: "reader".into(),
        })
        .unwrap();
        let mut settings = s.settings().unwrap();
        settings.paused = true;
        s.save_settings(&settings).unwrap();
        s.activity_recovery(123).unwrap();
        let tx = s.conn.transaction().unwrap();
        for i in 0..257 {
            let key = format!("{i:03}");
            tx.execute("INSERT INTO stage VALUES(?,'{}')", [&key])
                .unwrap();
            tx.execute("INSERT INTO activity_stage VALUES('a/b',1,?,'{}')", [&key])
                .unwrap();
            tx.execute("INSERT INTO source_stage VALUES('authored',?,'{}')", [&key])
                .unwrap();
            tx.execute("INSERT INTO teams_stage VALUES(?,?)", [&key, &key])
                .unwrap();
            tx.execute(
                "INSERT INTO pending(key,job,manual,foreground,at) VALUES(?,'{}',1,1,0)",
                [&key],
            )
            .unwrap();
            tx.execute(
                "INSERT INTO check_context VALUES('P','sha',?,'{}',1)",
                [&key],
            )
            .unwrap();
            tx.execute(
                "INSERT INTO check_state VALUES(?,'sha',?,1,NULL,?)",
                rusqlite::params![format!("P{key}"), now() as i64, now() as i64],
            )
            .unwrap();
        }
        tx.execute_batch("INSERT INTO watch_locator(id,repo,number,head) VALUES('P256','a/b',1,'sha');
            INSERT INTO watch_source VALUES('P256','authored');
            INSERT INTO activity VALUES('A','a/b',1,'comment',1,1,'eligible',1);
            INSERT INTO seen VALUES('A',0); INSERT INTO dedup VALUES('A');
            INSERT INTO progress VALUES('activity:baseline','1'),('catchup:A','A');
            INSERT INTO activity_intent VALUES(1,'pr','attempted',1),(2,'summary','unavailable',1);
            INSERT INTO intent_member VALUES(1,'A'),(2,'A'); INSERT INTO alert_attempt VALUES('attempt',1);
            INSERT INTO api_validator VALUES('notifications','etag',NULL,4070908800);
            INSERT INTO due VALUES('activity:notifications',4070908800);
            INSERT INTO resource_state VALUES('authored',1,1,NULL);").unwrap();
        tx.execute(
            "INSERT INTO content VALUES('published','{}',?,?)",
            [now() as i64, now() as i64],
        )
        .unwrap();
        tx.execute("INSERT INTO check_context VALUES('P256','sha','published',?,0)", [r#"{"id":"C","kind":"StatusContext","name":"CI","status":"SUCCESS","commit":"sha","sourceCreatedAt":"2026-10-17T00:00:00Z"}"#]).unwrap();
        tx.commit().unwrap();
    }
    fn preserved(s: &Store, generation: u64, revision: u64) {
        assert_eq!(s.identity().unwrap().unwrap().id, 42);
        assert!(s.settings().unwrap().paused);
        assert_eq!(s.generation().unwrap(), generation);
        assert_eq!(s.revision().unwrap(), revision);
        for (t, count) in [
            ("content", 1),
            ("watch_locator", 1),
            ("watch_source", 1),
            ("activity", 1),
            ("seen", 1),
            ("dedup", 1),
            ("progress", 2),
            ("activity_intent", 2),
            ("intent_member", 2),
            ("alert_attempt", 1),
            ("api_validator", 1),
        ] {
            assert_eq!(
                n(&s.conn, &format!("SELECT COUNT(*) FROM {t}")),
                count,
                "{t}"
            );
        }
        assert_eq!(n(&s.conn, "SELECT seen FROM seen"), 0);
        assert_eq!(
            n(
                &s.conn,
                "SELECT COUNT(*) FROM activity_intent WHERE state IN ('attempted','unavailable')"
            ),
            2
        );
        assert_eq!(
            n(&s.conn, "SELECT COUNT(*) FROM check_context WHERE staged=0"),
            1
        );
        assert_eq!(n(&s.conn, "SELECT complete FROM resource_state"), 1);
        assert_eq!(n(&s.conn, "SELECT boundary FROM activity_cycle"), 123);
        assert_eq!(s.due("activity:notifications").unwrap(), 4070908800);
        assert_eq!(n(&s.conn, "SELECT next_at FROM api_validator"), 4070908800);
    }
    fn complete(s: &Store) {
        assert!(!pending(&s.conn).unwrap());
        for table in [
            "stage",
            "activity_stage",
            "source_stage",
            "teams_stage",
            "pending",
        ] {
            assert_eq!(n(&s.conn, &format!("SELECT COUNT(*) FROM {table}")), 0);
        }
        assert_eq!(
            n(&s.conn, "SELECT COUNT(*) FROM check_context WHERE staged=1"),
            0
        );
        assert_eq!(
            n(
                &s.conn,
                "SELECT COUNT(*) FROM check_state WHERE complete=0 AND error='Content invalidated'"
            ),
            257
        );
    }
    #[test]
    fn invalidation_global_bound_and_logical_read_barrier_preserve_history() {
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("db");
        let mut s = Store::open(&path).unwrap();
        seed(&mut s);
        assert_eq!(
            s.activity_check_records("a/b", 1, true, "sha")
                .unwrap()
                .len(),
            1
        );
        let generation = s.generation().unwrap() + 1;
        let revision = s.revision().unwrap() + 1;
        s.conn.execute_batch("CREATE TRIGGER fail_barrier BEFORE UPDATE ON meta WHEN OLD.key='revision' BEGIN SELECT RAISE(ABORT,'fixture barrier fault'); END;").unwrap();
        assert!(s.invalidate().is_err());
        assert!(!pending(&s.conn).unwrap());
        preserved(&s, generation - 1, revision - 1);
        assert_eq!(n(&s.conn, "SELECT COUNT(*) FROM stage"), 257);
        s.conn.execute_batch("DROP TRIGGER fail_barrier").unwrap();
        let before = s.conn.total_changes();
        begin(&mut s.conn).unwrap();
        assert_eq!(s.conn.total_changes() - before, 3);
        assert_eq!(
            n(
                &s.conn,
                "SELECT complete FROM check_state WHERE pr_id='P256'"
            ),
            1
        );
        let cached = Store::open_cached(&path).unwrap();
        assert!(
            cached
                .activity_check_records("a/b", 1, true, "sha")
                .unwrap()
                .is_empty()
        );
        let page = cached.check_page("a/b", 1, 0).unwrap();
        assert_eq!(page.items.len(), 1);
        assert!(!page.commits[0].complete);
        assert_eq!(
            page.commits[0].error.as_deref(),
            Some("Content invalidated")
        );
        drop(cached);
        let mut passes = 0;
        while pending(&s.conn).unwrap() {
            let before = s.conn.total_changes();
            let changed = batch(&mut s.conn).unwrap();
            assert_eq!(s.conn.total_changes() - before, changed as u64);
            assert!(changed <= BATCH);
            preserved(&s, generation, revision);
            passes += 1;
        }
        assert_eq!(passes, 27); // Includes counter writes and conservative per-row reservation.
        complete(&s);
        let before = s.conn.total_changes();
        resume(&mut s.conn).unwrap();
        assert_eq!(s.conn.total_changes(), before);
        assert!(!s.put("late", &"obsolete", generation - 1).unwrap());
        drop(s);
        let s = Store::open(&path).unwrap();
        complete(&s);
        preserved(&s, generation, revision);
    }
    #[test]
    fn invalidation_late_fault_is_restartable_and_protected_reads_stay_incomplete() {
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("db");
        let mut s = Store::open(&path).unwrap();
        seed(&mut s);
        let g = s.generation().unwrap() + 1;
        let r = s.revision().unwrap() + 1;
        s.conn.execute_batch("CREATE TRIGGER fail_invalidation BEFORE DELETE ON activity_stage BEGIN SELECT RAISE(ABORT,'fixture invalidation fault'); END;").unwrap();
        assert!(s.invalidate().is_err());
        // Four 64-row batches committed; the next batch's last legacy row
        // deletion rolled back with its first Activity-stage deletion.
        assert_eq!(n(&s.conn, "SELECT COUNT(*) FROM stage"), 1);
        assert_eq!(n(&s.conn, "SELECT COUNT(*) FROM activity_stage"), 257);
        preserved(&s, g, r);
        assert!(s.invalidate().is_err());
        preserved(&s, g, r); // no second generation bump
        drop(s);
        assert!(Store::open(&path).is_err());
        let cached = Store::open_cached(&path).unwrap();
        assert!(!cached.check_page("a/b", 1, 0).unwrap().commits[0].complete);
        preserved(&cached, g, r);
        let c = Connection::open(&path).unwrap();
        c.execute_batch("DROP TRIGGER fail_invalidation; CREATE TRIGGER fail_finish BEFORE DELETE ON invalidation BEGIN SELECT RAISE(ABORT,'fixture finish fault'); END;").unwrap();
        assert!(Store::open(&path).is_err());
        assert!(pending(&c).unwrap());
        assert_eq!(
            n(&c, "SELECT COUNT(*) FROM check_state WHERE complete=1"),
            8
        );
        c.execute_batch("DROP TRIGGER fail_finish").unwrap();
        drop(c);
        drop(cached);
        let s = Store::open(&path).unwrap();
        complete(&s);
        preserved(&s, g, r);
    }
    #[test]
    fn crash_child_entry() {
        let Some(path) = std::env::var_os("GITJUKE_TEST_INVALIDATION_DB") else {
            return;
        };
        // Represent an already-open session with active traversal state; normal
        // startup cleanup would otherwise discard that state before this call.
        let conn = Connection::open(Path::new(&path)).unwrap();
        conn.pragma_update(None, "synchronous", "FULL").unwrap();
        Store {
            conn,
            cache_writes: true,
        }
        .invalidate()
        .unwrap();
        panic!("child missed invalidation barrier");
    }
    #[test]
    fn invalidation_sigkill_barrier_and_batch_commits_resume_before_startup() {
        use std::{
            process::{Child, Command, Stdio},
            time::{Duration, Instant},
        };
        struct OwnedChild(Child);
        impl Drop for OwnedChild {
            fn drop(&mut self) {
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
        }
        for (phase, remaining, incomplete, marked) in [
            ("barrier_committed", 257, 257, true),
            ("batch_before_commit", 257, 257, true),
            ("batch_after_commit", 193, 257, true),
            ("finish_before_commit", 0, 8, true),
            ("finish_after_commit", 0, 0, false),
        ] {
            let d = tempfile::tempdir().unwrap();
            let path = d.path().join("db");
            let marker = d.path().join("marker");
            let mut s = Store::open(&path).unwrap();
            seed(&mut s);
            let g = s.generation().unwrap() + 1;
            let r = s.revision().unwrap() + 1;
            drop(s);
            let mut child = OwnedChild(
                Command::new(std::env::current_exe().unwrap())
                    .args([
                        "--exact",
                        "invalidation::tests::crash_child_entry",
                        "--nocapture",
                    ])
                    .env("GITJUKE_TEST_INVALIDATION_DB", &path)
                    .env("GITJUKE_TEST_INVALIDATION_PHASE", phase)
                    .env("GITJUKE_TEST_INVALIDATION_MARKER", &marker)
                    .stdout(Stdio::null())
                    .spawn()
                    .unwrap(),
            );
            let start = Instant::now();
            while !marker.exists() {
                assert!(start.elapsed() < Duration::from_secs(10));
                assert!(child.0.try_wait().unwrap().is_none());
                std::thread::sleep(Duration::from_millis(5));
            }
            child.0.kill().unwrap();
            use std::os::unix::process::ExitStatusExt;
            assert_eq!(child.0.wait().unwrap().signal(), Some(libc::SIGKILL));
            let raw = Connection::open(&path).unwrap();
            assert_eq!(
                raw.query_row("PRAGMA quick_check(1)", [], |r| r.get::<_, String>(0))
                    .unwrap(),
                "ok"
            );
            assert_eq!(pending(&raw).unwrap(), marked);
            assert_eq!(n(&raw, "SELECT COUNT(*) FROM stage"), remaining);
            assert_eq!(
                n(&raw, "SELECT COUNT(*) FROM check_state WHERE complete=1"),
                incomplete
            );
            drop(raw);
            let s = Store::open(&path).unwrap();
            complete(&s);
            preserved(&s, g, r);
        }
    }
}
