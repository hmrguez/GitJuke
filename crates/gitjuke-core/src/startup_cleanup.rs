//! Restartable reclamation before any worker or remote read can start.
//! Only abandoned staging/session work is removed. Published and durable state
//! survives every committed batch; no persistent cleanup cursor is required.
use crate::{Result, store::storage_healthy};
use rusqlite::Connection;
use std::path::Path;

const STARTUP_BATCH: usize = 128;

pub(crate) fn cleanup(conn: &mut Connection, directory: &Path) -> Result<()> {
    loop {
        storage_healthy(directory)?;
        if cleanup_batch(conn)? == 0 {
            return Ok(());
        }
    }
}

fn cleanup_batch(conn: &mut Connection) -> Result<usize> {
    let tx = conn.transaction()?;
    let mut changed = 0;
    // All tables use ordinary SQLite rowids. Limit the whole transaction, not
    // each statement; unchanged rows never consume the budget or prevent exit.
    for statement in [
        "UPDATE watch SET selected=0 WHERE rowid IN (SELECT rowid FROM watch WHERE selected!=0 LIMIT ?)",
        "DELETE FROM stage WHERE rowid IN (SELECT rowid FROM stage LIMIT ?)",
        "DELETE FROM source_stage WHERE rowid IN (SELECT rowid FROM source_stage LIMIT ?)",
        "DELETE FROM teams_stage WHERE rowid IN (SELECT rowid FROM teams_stage LIMIT ?)",
        "DELETE FROM pending WHERE rowid IN (SELECT rowid FROM pending WHERE key LIKE 'discover:%' OR key LIKE 'checks:%' OR key LIKE 'legacy:%' OR key LIKE 'activity:%' LIMIT ?)",
        "DELETE FROM check_context WHERE rowid IN (SELECT rowid FROM check_context WHERE staged=1 LIMIT ?)",
        "DELETE FROM watch_source WHERE rowid IN (SELECT rowid FROM watch_source WHERE source='selected' LIMIT ?)",
        "UPDATE pending SET manual=0,foreground=0 WHERE rowid IN (SELECT rowid FROM pending WHERE manual!=0 OR foreground!=0 LIMIT ?)",
        "UPDATE due SET at=0 WHERE rowid IN (SELECT rowid FROM due WHERE key='discovery' AND at!=0 LIMIT ?)",
        "DELETE FROM activity_stage WHERE rowid IN (SELECT rowid FROM activity_stage LIMIT ?)",
    ] {
        changed += tx.execute(statement, [(STARTUP_BATCH - changed) as i64])?;
        if changed == STARTUP_BATCH {
            break;
        }
    }
    #[cfg(test)]
    crash_point("before_commit");
    tx.commit()?;
    #[cfg(test)]
    crash_point("after_commit");
    Ok(changed)
}

#[cfg(test)]
fn crash_point(phase: &str) {
    // Only the subprocess fixture supplies these variables. No fault switch is
    // compiled into production. The parent SIGKILLs/reaps at the named boundary.
    if std::env::var("GITJUKE_TEST_CLEANUP_PHASE").as_deref() == Ok(phase) {
        let marker = std::env::var_os("GITJUKE_TEST_CLEANUP_MARKER").unwrap();
        std::fs::write(marker, phase).unwrap();
        loop {
            std::thread::park();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{model::Identity, store::Store};
    use rusqlite::params;

    fn count(conn: &Connection, table: &str) -> usize {
        conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| {
            r.get::<_, i64>(0)
        })
        .unwrap() as usize
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
        s.conn.execute_batch("INSERT INTO content VALUES('published','{}',1,1);
            INSERT INTO watch_locator(id,repo,number,head) VALUES('P','a/b',1,'sha');
            INSERT INTO watch_source VALUES('P','authored');
            INSERT INTO check_context VALUES('P','sha','published','{}',0);
            INSERT INTO check_state VALUES('P','sha',1,1,NULL,1);
            INSERT INTO resource_state VALUES('authored',1,1,'offline');
            INSERT INTO activity VALUES('A','a/b',1,'comment',1,1,'eligible',1);
            INSERT INTO seen VALUES('A',0); INSERT INTO dedup VALUES('A');
            INSERT INTO progress VALUES('activity:baseline','1'),('catchup:A','A'),('selection','[\"a/b\",1]');
            INSERT INTO activity_cycle VALUES(1,'recovery',123,0);
            INSERT INTO activity_intent VALUES(1,'pr','attempted',1),(2,'summary','unavailable',1);
            INSERT INTO intent_member VALUES(1,'A'),(2,'A');
            INSERT INTO alert_attempt VALUES('attempt',1);
            INSERT INTO api_validator VALUES('notifications?window','etag',NULL,4070908800);
            INSERT INTO due VALUES('activity:notifications',4070908800),('discovery',99);
            INSERT INTO pending(key,job,manual,foreground,at) VALUES('pr:a/b:1','{}',1,1,0);").unwrap();
    }
    fn assert_preserved(s: &Store, revision: u64) {
        assert!(s.settings().unwrap().paused);
        assert_eq!(s.identity().unwrap().unwrap().id, 42);
        assert_eq!(s.revision().unwrap(), revision);
        for (table, expected) in [
            ("content", 1),
            ("watch_locator", 1),
            ("activity", 1),
            ("seen", 1),
            ("dedup", 1),
            ("progress", 3),
            ("activity_intent", 2),
            ("intent_member", 2),
            ("alert_attempt", 1),
            ("api_validator", 1),
        ] {
            assert_eq!(count(&s.conn, table), expected, "{table}");
        }
        assert_eq!(s.due("activity:notifications").unwrap(), 4070908800);
        assert_eq!(
            s.conn
                .query_row("SELECT boundary FROM activity_cycle", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            123
        );
        assert!(
            !s.conn
                .query_row("SELECT seen FROM seen", [], |r| r.get::<_, bool>(0))
                .unwrap()
        );
        assert_eq!(s.conn.query_row("SELECT COUNT(*) FROM activity_intent WHERE state IN ('attempted','unavailable')", [], |r| r.get::<_, i64>(0)).unwrap(), 2);
        assert_eq!(
            s.conn
                .query_row("SELECT complete FROM check_state", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            1
        );
        assert_eq!(
            s.conn
                .query_row("SELECT error FROM resource_state", [], |r| r
                    .get::<_, String>(0))
                .unwrap(),
            "offline"
        );
    }

    #[test]
    fn global_mutation_bound_covers_every_startup_class_and_preserves_published_state() {
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("db");
        let mut s = Store::open(&path).unwrap();
        seed(&mut s);
        let revision = s.revision().unwrap();
        let tx = s.conn.transaction().unwrap();
        for n in 0..257 {
            let id = format!("staged-{n:04}");
            tx.execute("INSERT INTO watch VALUES(?,1,0,1)", [&id])
                .unwrap();
            tx.execute("INSERT INTO stage VALUES(?,'{}')", [&id])
                .unwrap();
            tx.execute("INSERT INTO source_stage VALUES('authored',?,'{}')", [&id])
                .unwrap();
            tx.execute("INSERT INTO teams_stage VALUES(?,'org/team')", [&id])
                .unwrap();
            tx.execute(
                "INSERT INTO check_context VALUES('P','sha',?,'{}',1)",
                [&id],
            )
            .unwrap();
            tx.execute("INSERT INTO watch_source VALUES(?,'selected')", [&id])
                .unwrap();
            tx.execute("INSERT INTO activity_stage VALUES('a/b',1,?,'{}')", [&id])
                .unwrap();
            for prefix in ["discover", "checks", "legacy", "activity"] {
                tx.execute(
                    "INSERT INTO pending(key,job,manual,foreground,at) VALUES(?,'{}',1,1,0)",
                    [format!("{prefix}:{id}")],
                )
                .unwrap();
            }
        }
        tx.commit().unwrap();
        let mut passes = 0;
        loop {
            let before = s.conn.total_changes();
            let changed = cleanup_batch(&mut s.conn).unwrap();
            assert_eq!(s.conn.total_changes() - before, changed as u64);
            assert!(changed <= STARTUP_BATCH);
            assert_preserved(&s, revision);
            passes += 1;
            if changed == 0 {
                break;
            }
        }
        assert!(passes > 20, "fixture spans many committed batches");
        for table in ["stage", "source_stage", "teams_stage", "activity_stage"] {
            assert_eq!(count(&s.conn, table), 0);
        }
        assert_eq!(count(&s.conn, "check_context"), 1);
        assert_eq!(count(&s.conn, "watch_source"), 1);
        assert_eq!(count(&s.conn, "pending"), 1);
        assert_eq!(count(&s.conn, "watch"), 257);
        assert_eq!(
            s.conn
                .query_row(
                    "SELECT COUNT(*) FROM watch WHERE authored=1 AND selected=0",
                    [],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
            257
        );
        assert_eq!(
            s.conn
                .query_row("SELECT manual+foreground FROM pending", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
        assert_eq!(s.due("discovery").unwrap(), 0);
        drop(s);
        let s = Store::open(&path).unwrap();
        assert_preserved(&s, revision);
    }

    #[test]
    fn failed_batch_rolls_back_companions_and_restart_continues_committed_batches() {
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("db");
        let mut s = Store::open(&path).unwrap();
        seed(&mut s);
        let revision = s.revision().unwrap();
        for n in 0..129 {
            s.conn
                .execute("INSERT INTO stage VALUES(?,'{}')", [n.to_string()])
                .unwrap();
        }
        s.conn.execute_batch("INSERT INTO source_stage VALUES('authored','left','{}');
            CREATE TRIGGER fail_cleanup BEFORE DELETE ON source_stage BEGIN SELECT RAISE(ABORT,'fixture cleanup fault'); END;").unwrap();
        assert_eq!(cleanup_batch(&mut s.conn).unwrap(), 128);
        assert!(cleanup_batch(&mut s.conn).is_err());
        assert_eq!(count(&s.conn, "stage"), 1);
        assert_eq!(count(&s.conn, "source_stage"), 1);
        assert_preserved(&s, revision);
        drop(s);
        assert!(Store::open(&path).is_err());
        let conn = Connection::open(&path).unwrap();
        assert_eq!(count(&conn, "stage"), 1);
        conn.execute_batch("DROP TRIGGER fail_cleanup").unwrap();
        drop(conn);
        let s = Store::open(&path).unwrap();
        assert_preserved(&s, revision);
        assert_eq!(count(&s.conn, "stage"), 0);
        assert_eq!(count(&s.conn, "source_stage"), 0);
    }

    #[test]
    fn crash_child_entry() {
        let Some(path) = std::env::var_os("GITJUKE_TEST_CLEANUP_DB") else {
            return;
        };
        Store::open(Path::new(&path)).unwrap();
        panic!("child must reach the configured cleanup barrier");
    }

    #[test]
    fn sigkill_during_and_between_batches_recovers_only_committed_cleanup() {
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
        for (phase, remaining) in [("before_commit", 257), ("after_commit", 129)] {
            let d = tempfile::tempdir().unwrap();
            let path = d.path().join("db");
            let marker = d.path().join("barrier");
            let mut s = Store::open(&path).unwrap();
            seed(&mut s);
            let revision = s.revision().unwrap();
            for n in 0..257 {
                s.conn
                    .execute("INSERT INTO stage VALUES(?,'{}')", params![n.to_string()])
                    .unwrap();
            }
            drop(s);
            let mut child = OwnedChild(
                Command::new(std::env::current_exe().unwrap())
                    .args([
                        "--exact",
                        "startup_cleanup::tests::crash_child_entry",
                        "--nocapture",
                    ])
                    .env("GITJUKE_TEST_CLEANUP_DB", &path)
                    .env("GITJUKE_TEST_CLEANUP_PHASE", phase)
                    .env("GITJUKE_TEST_CLEANUP_MARKER", &marker)
                    .stdout(Stdio::null())
                    .spawn()
                    .unwrap(),
            );
            let start = Instant::now();
            while !marker.exists() {
                assert!(
                    start.elapsed() < Duration::from_secs(10),
                    "child did not reach {phase}"
                );
                assert!(
                    child.0.try_wait().unwrap().is_none(),
                    "child exited before barrier"
                );
                std::thread::sleep(Duration::from_millis(10));
            }
            child.0.kill().unwrap();
            let status = child.0.wait().unwrap();
            use std::os::unix::process::ExitStatusExt;
            assert_eq!(status.signal(), Some(libc::SIGKILL));
            let conn = Connection::open(&path).unwrap();
            assert_eq!(count(&conn, "stage"), remaining, "{phase}");
            assert_eq!(
                conn.query_row("PRAGMA quick_check(1)", [], |r| r.get::<_, String>(0))
                    .unwrap(),
                "ok"
            );
            drop(conn);
            let s = Store::open(&path).unwrap();
            assert_eq!(count(&s.conn, "stage"), 0);
            assert_preserved(&s, revision);
        }
    }
}
