//! Recovery initialization finishes before scheduling can observe its state.
//! The cycle row durably owns the boundary before restartable cleanup begins.
use crate::{
    Result,
    store::{bump, storage_healthy},
};
use rusqlite::Connection;
use std::path::Path;

const RECOVERY_BATCH: usize = 128;

pub(crate) fn recover(conn: &mut Connection, time: u64) -> Result<()> {
    let directory = conn
        .path()
        .and_then(|p| Path::new(p).parent())
        .unwrap_or(Path::new("/"))
        .to_path_buf();
    begin(conn, &directory, time)?;
    loop {
        storage_healthy(&directory)?;
        if cleanup_batch(conn)? == 0 {
            return Ok(());
        }
    }
}

fn begin(conn: &mut Connection, directory: &Path, time: u64) -> Result<()> {
    storage_healthy(directory)?;
    let tx = conn.transaction()?;
    // Incomplete recovery keeps its boundary, including after partial cleanup.
    tx.execute("INSERT INTO activity_cycle VALUES(1,'recovery',?,0) ON CONFLICT(id) DO UPDATE SET mode='recovery',boundary=CASE WHEN mode IN ('normal','offline') THEN excluded.boundary ELSE boundary END,notifications_done=0", [time as i64])?;
    bump(&tx)?;
    tx.commit()?;
    #[cfg(test)]
    crash_point("boundary_committed");
    Ok(())
}

fn cleanup_batch(conn: &mut Connection) -> Result<usize> {
    let tx = conn.transaction()?;
    let batch_before = tx.total_changes();
    let mut changed = 0;

    for sql in [
        "UPDATE activity_target SET done=0,due=0 WHERE rowid IN (SELECT rowid FROM activity_target WHERE done!=0 OR due!=0 ORDER BY rowid LIMIT ?)",
        "DELETE FROM activity_stage WHERE rowid IN (SELECT rowid FROM activity_stage ORDER BY rowid LIMIT ?)",
        "DELETE FROM pending WHERE rowid IN (SELECT rowid FROM pending WHERE key LIKE 'activity:%' ORDER BY rowid LIMIT ?)",
        // Provisional state only: no dispatch or membership reconstruction.
        // Attempted/unavailable history must never become a replay backlog.
        "UPDATE activity_intent SET state='recovery' WHERE rowid IN (SELECT rowid FROM activity_intent WHERE state='pending' ORDER BY rowid LIMIT ?)",
    ] {
        let limit = (RECOVERY_BATCH - changed) / crate::shared_storage::cost(sql);
        if limit == 0 {
            break;
        }
        let before = tx.total_changes();
        let rows = tx.execute(sql, [limit as i64])?;
        changed += (tx.total_changes() - before) as usize;
        if rows == limit {
            break;
        }
    }
    #[cfg(test)]
    crash_point("before_commit");
    crate::shared_storage::bounded(&tx, batch_before)?;
    tx.commit()?;
    #[cfg(test)]
    crash_point("after_commit");
    Ok(changed)
}

#[cfg(test)]
fn crash_point(phase: &str) {
    if std::env::var("GITJUKE_TEST_RECOVERY_PHASE").as_deref() == Ok(phase) {
        std::fs::write(
            std::env::var_os("GITJUKE_TEST_RECOVERY_MARKER").unwrap(),
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
    use crate::{model::Identity, store::Store};

    fn count(conn: &Connection, query: &str) -> i64 {
        conn.query_row(query, [], |r| r.get(0)).unwrap()
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
        s.conn
            .execute_batch(
                "INSERT INTO activity_cycle VALUES(1,'normal',1,1);
            INSERT INTO progress VALUES('activity:baseline','1'),('catchup:A','A');
            INSERT INTO activity VALUES('A','a/b',1,'comment',1,1,'eligible',1);
            INSERT INTO seen VALUES('A',0); INSERT INTO dedup VALUES('A');
            INSERT INTO activity_intent VALUES(1,'pr','attempted',1),(2,'summary','unavailable',1);
            INSERT INTO intent_member VALUES(1,'A'),(2,'A');
            INSERT INTO alert_attempt VALUES('attempt',1);
            INSERT INTO content VALUES('published','{}',1,1);
            INSERT INTO watch_locator(id,repo,number) VALUES('P','a/b',1);
            INSERT INTO watch_source VALUES('P','authored');
            INSERT INTO check_context VALUES('P','sha','C','{}',0);
            INSERT INTO api_validator VALUES('notifications','etag',NULL,4070908800);
            INSERT INTO due VALUES('activity:notifications',4070908800);
            INSERT INTO pending(key,job,manual,foreground,at) VALUES('pr:a/b:1','{}',0,0,0);",
            )
            .unwrap();
        let tx = s.conn.transaction().unwrap();
        for n in 0..257 {
            tx.execute("INSERT INTO activity_target VALUES('a/b',?,9,1)", [n])
                .unwrap();
            tx.execute(
                "INSERT INTO activity_stage VALUES('a/b',1,?,'{}')",
                [n.to_string()],
            )
            .unwrap();
            tx.execute(
                "INSERT INTO pending(key,job,manual,foreground,at) VALUES(?,'{}',0,0,0)",
                [format!("activity:{n}")],
            )
            .unwrap();
            tx.execute(
                "INSERT INTO activity_intent(kind,state,created) VALUES('pr','pending',1)",
                [],
            )
            .unwrap();
        }
        tx.commit().unwrap();
    }
    fn preserved(s: &Store, boundary: i64) {
        assert_eq!(s.identity().unwrap().unwrap().id, 42);
        assert!(s.settings().unwrap().paused);
        assert_eq!(
            count(&s.conn, "SELECT boundary FROM activity_cycle"),
            boundary
        );
        assert_eq!(
            count(&s.conn, "SELECT notifications_done FROM activity_cycle"),
            0
        );
        assert_eq!(
            count(&s.conn, "SELECT seen FROM seen WHERE source_id='A'"),
            0
        );
        assert_eq!(s.activity_progress("activity:baseline").unwrap(), 1);
        assert_eq!(s.due("activity:notifications").unwrap(), 4070908800);
        assert_eq!(
            count(&s.conn, "SELECT next_at FROM api_validator"),
            4070908800
        );
        for table in [
            "activity",
            "dedup",
            "alert_attempt",
            "content",
            "watch_source",
            "check_context",
        ] {
            assert_eq!(
                count(&s.conn, &format!("SELECT COUNT(*) FROM {table}")),
                1,
                "{table}"
            );
        }
        assert_eq!(count(&s.conn, "SELECT COUNT(*) FROM intent_member"), 2);
        assert_eq!(
            count(
                &s.conn,
                "SELECT COUNT(*) FROM activity_intent WHERE state IN ('attempted','unavailable')"
            ),
            2
        );
        assert_eq!(
            s.conn
                .query_row("SELECT state FROM activity_intent WHERE id=1", [], |r| {
                    r.get::<_, String>(0)
                })
                .unwrap(),
            "attempted"
        );
        assert_eq!(
            s.conn
                .query_row("SELECT state FROM activity_intent WHERE id=2", [], |r| {
                    r.get::<_, String>(0)
                })
                .unwrap(),
            "unavailable"
        );
        assert_eq!(count(&s.conn, "SELECT COUNT(*) FROM progress"), 2);
    }
    fn complete(s: &Store) {
        for query in [
            "SELECT COUNT(*) FROM activity_target WHERE done!=0 OR due!=0",
            "SELECT COUNT(*) FROM activity_stage",
            "SELECT COUNT(*) FROM pending WHERE key LIKE 'activity:%'",
            "SELECT COUNT(*) FROM activity_intent WHERE state='pending'",
        ] {
            assert_eq!(count(&s.conn, query), 0);
        }
        assert_eq!(count(&s.conn, "SELECT COUNT(*) FROM activity_target"), 257);
        assert_eq!(
            count(
                &s.conn,
                "SELECT COUNT(*) FROM activity_intent WHERE state='recovery'"
            ),
            257
        );
        assert_eq!(count(&s.conn, "SELECT COUNT(*) FROM pending"), 1);
    }
    #[test]
    fn global_bound_and_idempotence_preserve_durable_history() {
        let d = tempfile::tempdir().unwrap();
        let mut s = Store::open(&d.path().join("db")).unwrap();
        seed(&mut s);
        let before = s.conn.total_changes();
        begin(&mut s.conn, d.path(), 123).unwrap();
        assert_eq!(s.conn.total_changes() - before, 2); // cycle + revision
        let mut passes = 0;
        loop {
            let before = s.conn.total_changes();
            let changed = cleanup_batch(&mut s.conn).unwrap();
            assert_eq!(s.conn.total_changes() - before, changed as u64);
            assert!(changed <= RECOVERY_BATCH);
            preserved(&s, 123);
            passes += 1;
            if changed == 0 {
                break;
            }
        }
        assert_eq!(passes, 12);
        complete(&s);
        let before = s.conn.total_changes();
        s.activity_recovery(999).unwrap();
        assert_eq!(s.conn.total_changes() - before, 2);
        preserved(&s, 123);
        complete(&s);
    }
    #[test]
    fn late_batch_fault_rolls_back_companions_and_restart_preserves_boundary() {
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("db");
        let mut s = Store::open(&path).unwrap();
        seed(&mut s);
        s.conn.execute_batch("CREATE TRIGGER fail_recovery BEFORE UPDATE ON activity_target WHEN OLD.number=256 BEGIN SELECT RAISE(ABORT,'fixture recovery fault'); END;").unwrap();
        assert!(s.activity_recovery(123).is_err());
        assert_eq!(
            count(&s.conn, "SELECT COUNT(*) FROM activity_target WHERE done=1"),
            1
        );
        assert_eq!(count(&s.conn, "SELECT COUNT(*) FROM activity_stage"), 257);
        preserved(&s, 123);
        s.conn.execute_batch("DROP TRIGGER fail_recovery; CREATE TRIGGER fail_intent BEFORE UPDATE ON activity_intent WHEN OLD.state='pending' BEGIN SELECT RAISE(ABORT,'fixture intent fault'); END;").unwrap();
        assert!(s.activity_recovery(999).is_err());
        // Stage deletions include counter writes. Five pending rows remain
        // in the failing final transaction beside provisional intent changes.
        assert_eq!(
            count(
                &s.conn,
                "SELECT COUNT(*) FROM pending WHERE key LIKE 'activity:%'"
            ),
            5
        );
        preserved(&s, 123);
        drop(s);
        let mut s = Store::open(&path).unwrap();
        assert!(s.activity_recovery(1000).is_err());
        preserved(&s, 123);
        s.conn.execute_batch("DROP TRIGGER fail_intent").unwrap();
        s.activity_recovery(1001).unwrap();
        complete(&s);
        preserved(&s, 123);
    }
    #[test]
    fn crash_child_entry() {
        let Some(path) = std::env::var_os("GITJUKE_TEST_RECOVERY_DB") else {
            return;
        };
        Store::open(Path::new(&path))
            .unwrap()
            .activity_recovery(123)
            .unwrap();
        panic!("child must stop at recovery barrier");
    }
    #[test]
    fn sigkill_at_boundary_and_batch_commit_restarts_recovery() {
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
        for (phase, remaining) in [
            ("boundary_committed", 257),
            ("before_commit", 257),
            ("after_commit", 129),
        ] {
            let d = tempfile::tempdir().unwrap();
            let path = d.path().join("db");
            let marker = d.path().join("barrier");
            let mut s = Store::open(&path).unwrap();
            seed(&mut s);
            drop(s);
            let mut child = OwnedChild(
                Command::new(std::env::current_exe().unwrap())
                    .args([
                        "--exact",
                        "recovery_cleanup::tests::crash_child_entry",
                        "--nocapture",
                    ])
                    .env("GITJUKE_TEST_RECOVERY_DB", &path)
                    .env("GITJUKE_TEST_RECOVERY_PHASE", phase)
                    .env("GITJUKE_TEST_RECOVERY_MARKER", &marker)
                    .stdout(Stdio::null())
                    .spawn()
                    .unwrap(),
            );
            let start = Instant::now();
            while !marker.exists() {
                assert!(start.elapsed() < Duration::from_secs(10));
                assert!(child.0.try_wait().unwrap().is_none());
                std::thread::sleep(Duration::from_millis(10));
            }
            child.0.kill().unwrap();
            use std::os::unix::process::ExitStatusExt;
            assert_eq!(child.0.wait().unwrap().signal(), Some(libc::SIGKILL));
            let conn = Connection::open(&path).unwrap();
            assert_eq!(
                count(&conn, "SELECT COUNT(*) FROM activity_target WHERE done=1"),
                remaining,
                "{phase}"
            );
            assert_eq!(count(&conn, "SELECT boundary FROM activity_cycle"), 123);
            assert_eq!(
                conn.query_row("PRAGMA quick_check(1)", [], |r| r.get::<_, String>(0))
                    .unwrap(),
                "ok"
            );
            drop(conn);
            let mut s = Store::open(&path).unwrap();
            s.activity_recovery(999).unwrap();
            complete(&s);
            preserved(&s, 123);
        }
    }
}
