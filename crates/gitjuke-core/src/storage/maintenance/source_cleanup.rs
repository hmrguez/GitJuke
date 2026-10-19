//! A source restart guard prevents partial staging cleanup from publishing
//! membership. Only a successful fresh initialization removes that guard.
use crate::{
    Result,
    store::{bump, storage_healthy},
};
use rusqlite::{Connection, params};
use std::path::Path;

const BATCH: usize = 128;

fn admit(c: &Connection) -> Result<()> {
    storage_healthy(
        c.path()
            .and_then(|p| Path::new(p).parent())
            .unwrap_or(Path::new("/")),
    )
}

fn begin(c: &mut Connection, source: &str) -> Result<()> {
    admit(c)?;
    let tx = c.transaction()?;
    if tx.execute("INSERT OR IGNORE INTO source_restart VALUES(?)", [source])? != 0 {
        bump(&tx)?;
    }
    #[cfg(test)]
    point("barrier_before_commit");
    tx.commit()?;
    #[cfg(test)]
    point("barrier_committed");
    Ok(())
}

pub(crate) fn initialize(c: &mut Connection, source: &str) -> Result<()> {
    begin(c, source)?;
    loop {
        admit(c)?;
        if cleanup_batch(c, source)?.1 {
            return Ok(());
        }
    }
}

fn cleanup_batch(c: &mut Connection, source: &str) -> Result<(usize, bool)> {
    let tx = c.transaction()?;
    let batch_before = tx.total_changes();
    let before = tx.total_changes();
    let rows = tx.execute(
        "DELETE FROM source_stage WHERE rowid IN (SELECT rowid FROM source_stage WHERE source=? ORDER BY rowid LIMIT ?)",
        params![source, (BATCH / 2) as i64],
    )?;
    let mut changed = (tx.total_changes() - before) as usize;
    let finished = rows < BATCH / 2;
    if finished {
        changed += tx.execute("DELETE FROM source_restart WHERE source=?", [source])?;
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
    Ok((changed, finished))
}

#[cfg(test)]
fn point(phase: &str) {
    if std::env::var("GITJUKE_TEST_SOURCE_INIT_PHASE").as_deref() == Ok(phase) {
        std::fs::write(
            std::env::var_os("GITJUKE_TEST_SOURCE_INIT_MARKER").unwrap(),
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
    use crate::{model::*, store::Store};

    fn n(c: &Connection, sql: &str) -> i64 {
        c.query_row(sql, [], |r| r.get(0)).unwrap()
    }
    fn pr() -> PullRequest {
        serde_json::from_value(serde_json::json!({
            "id":"P", "repoId":1,"repo":"a/b","number":1,"title":"Published",
            "url":"https://github.com/a/b/pull/1","state":"open","draft":false,
            "author":null,"updatedAt":"2026-10-17T00:00:00Z"
        }))
        .unwrap()
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
        s.stage_source("authored", &[pr()]).unwrap();
        s.publish_source("authored").unwrap();
        s.conn
            .execute_batch(
                "INSERT INTO source_stage VALUES('independent','neighbor','{}');
            INSERT INTO activity VALUES('A','a/b',1,'comment',1,1,'eligible',1);
            INSERT INTO seen VALUES('A',0); INSERT INTO dedup VALUES('A');
            INSERT INTO progress VALUES('activity:baseline','1'),('catchup:A','A');
            INSERT INTO activity_cycle VALUES(1,'recovery',123,0);
            INSERT INTO activity_intent VALUES(1,'pr','attempted',1),(2,'summary','unavailable',1);
            INSERT INTO intent_member VALUES(1,'A'),(2,'A');
            INSERT INTO alert_attempt VALUES('attempt',1);
            INSERT INTO api_validator VALUES('notifications?window','etag',NULL,4070908800);
            INSERT INTO due VALUES('activity:notifications',4070908800);",
            )
            .unwrap();
        let tx = s.conn.transaction().unwrap();
        for i in 0..257 {
            tx.execute(
                "INSERT INTO source_stage VALUES('authored',?,'{}')",
                [format!("old-{i:03}")],
            )
            .unwrap();
        }
        tx.commit().unwrap();
    }
    // Compare values, not just row counts, through failure and raw pre-startup restart.
    fn preserved(c: &Connection) -> Vec<Vec<rusqlite::types::Value>> {
        let mut all = vec![];
        for table in [
            "identity",
            "settings",
            "watch_locator",
            "watch_source",
            "watch",
            "content",
            "resource_state",
            "activity",
            "seen",
            "dedup",
            "progress",
            "activity_cycle",
            "activity_intent",
            "intent_member",
            "alert_attempt",
            "api_validator",
            "due",
        ] {
            let mut q = c
                .prepare(&format!("SELECT * FROM {table} ORDER BY rowid"))
                .unwrap();
            let width = q.column_count();
            all.extend(
                q.query_map([], |r| {
                    (0..width)
                        .map(|i| r.get(i))
                        .collect::<rusqlite::Result<Vec<_>>>()
                })
                .unwrap()
                .map(|r| r.unwrap()),
            );
        }
        all
    }
    fn remaining(c: &Connection) -> i64 {
        n(
            c,
            "SELECT COUNT(*) FROM source_stage WHERE source='authored'",
        )
    }
    #[test]
    fn global_bound_guard_isolation_and_fresh_publication() {
        let d = tempfile::tempdir().unwrap();
        let mut s = Store::open(&d.path().join("db")).unwrap();
        seed(&mut s);
        let original = preserved(&s.conn);
        let before = s.conn.total_changes();
        let revision = s.revision().unwrap();
        let generation = s.generation().unwrap();
        begin(&mut s.conn, "authored").unwrap();
        assert_eq!(s.conn.total_changes() - before, 2);
        assert_eq!(s.revision().unwrap(), revision + 1);
        assert_eq!(s.generation().unwrap(), generation);
        assert!(s.stage_source("authored", &[pr()]).is_err());
        assert!(s.publish_source("authored").is_err());
        assert!(!s.list_page("authored", 0).unwrap().complete);
        let before = s.conn.total_changes();
        begin(&mut s.conn, "authored").unwrap();
        assert_eq!(s.conn.total_changes(), before);
        for expected in [
            (128, false),
            (128, false),
            (128, false),
            (128, false),
            (3, true),
        ] {
            let before = s.conn.total_changes();
            let result = cleanup_batch(&mut s.conn, "authored").unwrap();
            assert_eq!(result, expected);
            assert_eq!(s.conn.total_changes() - before, result.0 as u64);
            assert!(result.0 <= BATCH);
            assert_eq!(preserved(&s.conn), original);
            assert_eq!(
                n(
                    &s.conn,
                    "SELECT COUNT(*) FROM source_stage WHERE source='independent'"
                ),
                1
            );
        }
        assert_eq!(cleanup_batch(&mut s.conn, "authored").unwrap(), (0, true));
        s.stage_source("authored", &[pr()]).unwrap();
        s.publish_source("authored").unwrap();
        assert!(s.list_page("authored", 0).unwrap().complete);
        assert_eq!(s.list_page("authored", 0).unwrap().items.len(), 1);
        assert_eq!(
            n(
                &s.conn,
                "SELECT COUNT(*) FROM watch_source WHERE source='authored'"
            ),
            1
        );
    }
    #[test]
    fn barrier_later_batch_and_final_marker_faults_preserve_committed_prefix() {
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("db");
        let mut s = Store::open(&path).unwrap();
        seed(&mut s);
        let original = preserved(&s.conn);
        let revision = s.revision().unwrap();
        s.conn.execute_batch("CREATE TRIGGER fail_barrier BEFORE UPDATE ON meta WHEN OLD.key='revision' BEGIN SELECT RAISE(ABORT,'fixture barrier'); END;").unwrap();
        assert!(s.begin_source("authored").is_err());
        assert!(!crate::pr_purge::source_restart(&s.conn, "authored").unwrap());
        assert_eq!(remaining(&s.conn), 257);
        s.conn.execute_batch("DROP TRIGGER fail_barrier;
            CREATE TRIGGER fail_batch BEFORE DELETE ON source_stage WHEN OLD.id='old-200' BEGIN SELECT RAISE(ABORT,'fixture cleanup'); END;").unwrap();
        assert!(s.begin_source("authored").is_err());
        assert_eq!(remaining(&s.conn), 65);
        assert_eq!(s.revision().unwrap(), revision + 1);
        assert!(s.begin_source("authored").is_err());
        assert_eq!(s.revision().unwrap(), revision + 1);
        assert_eq!(preserved(&s.conn), original);
        s.conn.execute_batch("DROP TRIGGER fail_batch;
            CREATE TRIGGER fail_finish BEFORE DELETE ON source_restart BEGIN SELECT RAISE(ABORT,'fixture finalization'); END;").unwrap();
        assert!(s.begin_source("authored").is_err());
        assert_eq!(remaining(&s.conn), 1); // companion deletion in final batch rolled back
        assert_eq!(preserved(&s.conn), original);
        assert!(s.publish_source("authored").is_err());
        drop(s);
        let raw = Connection::open(&path).unwrap();
        assert_eq!(remaining(&raw), 1);
        assert_eq!(preserved(&raw), original);
        raw.execute_batch("DROP TRIGGER fail_finish").unwrap();
        drop(raw);
        let mut s = Store::open(&path).unwrap();
        assert_eq!(remaining(&s.conn), 0); // ordinary bounded startup discards abandoned staging
        assert!(crate::pr_purge::source_restart(&s.conn, "authored").unwrap());
        assert!(s.publish_source("authored").is_err());
        assert_eq!(preserved(&s.conn), original);
        s.begin_source("authored").unwrap();
        assert!(!crate::pr_purge::source_restart(&s.conn, "authored").unwrap());
    }
    #[test]
    fn restart_guard_blocks_otherwise_ready_recovery() {
        let d = tempfile::tempdir().unwrap();
        let mut s = Store::open(&d.path().join("db")).unwrap();
        s.activity_recovery(123).unwrap();
        s.conn.execute_batch("UPDATE activity_cycle SET notifications_done=1;
            INSERT INTO resource_state VALUES('authored',1,1,NULL),('direct',1,1,NULL),('teams',1,1,NULL);").unwrap();
        begin(&mut s.conn, "authored").unwrap();
        let revision = s.revision().unwrap();
        s.activity_finish_recovery(false).unwrap();
        assert_eq!(s.revision().unwrap(), revision);
        assert_eq!(
            n(
                &s.conn,
                "SELECT COUNT(*) FROM activity_cycle WHERE mode='recovery'"
            ),
            1
        );
        s.begin_source("authored").unwrap();
        s.activity_finish_recovery(false).unwrap();
        assert_eq!(
            n(
                &s.conn,
                "SELECT COUNT(*) FROM activity_cycle WHERE mode='normal'"
            ),
            1
        );
    }
    #[test]
    fn crash_child_entry() {
        let Some(path) = std::env::var_os("GITJUKE_TEST_SOURCE_INIT_DB") else {
            return;
        };
        // Open raw so normal startup does not remove the deliberately abandoned stage.
        let conn = Connection::open(path).unwrap();
        let mut s = Store {
            conn,
            cache_writes: true,
        };
        s.begin_source("authored").unwrap();
        panic!("child did not reach configured barrier");
    }
    #[test]
    fn sigkill_initialization_keeps_guard_until_last_cleanup_commit() {
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
        for (phase, remaining_rows, marked, increment) in [
            ("barrier_before_commit", 257, false, 0),
            ("barrier_committed", 257, true, 1),
            ("batch_before_commit", 257, true, 1),
            ("batch_after_commit", 193, true, 1),
            ("finish_before_commit", 1, true, 1),
            ("finish_after_commit", 0, false, 1),
        ] {
            let d = tempfile::tempdir().unwrap();
            let path = d.path().join("db");
            let marker = d.path().join("marker");
            let mut s = Store::open(&path).unwrap();
            seed(&mut s);
            let original = preserved(&s.conn);
            let revision = s.revision().unwrap();
            drop(s);
            let mut child = OwnedChild(
                Command::new(std::env::current_exe().unwrap())
                    .args([
                        "--exact",
                        "source_cleanup::tests::crash_child_entry",
                        "--nocapture",
                    ])
                    .env("GITJUKE_TEST_SOURCE_INIT_DB", &path)
                    .env("GITJUKE_TEST_SOURCE_INIT_PHASE", phase)
                    .env("GITJUKE_TEST_SOURCE_INIT_MARKER", &marker)
                    .stdout(Stdio::null())
                    .spawn()
                    .unwrap(),
            );
            let start = Instant::now();
            while !marker.exists() {
                assert!(start.elapsed() < Duration::from_secs(10), "{phase}");
                assert!(child.0.try_wait().unwrap().is_none(), "{phase}");
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
            assert_eq!(remaining(&raw), remaining_rows, "{phase}");
            assert_eq!(
                crate::pr_purge::source_restart(&raw, "authored").unwrap(),
                marked,
                "{phase}"
            );
            assert_eq!(preserved(&raw), original);
            assert_eq!(
                n(
                    &raw,
                    "SELECT COUNT(*) FROM source_stage WHERE source='independent'"
                ),
                1
            );
            let mut cached = Store::open_cached(&path).unwrap();
            assert_eq!(cached.revision().unwrap(), revision + increment);
            if marked {
                assert!(!cached.list_page("authored", 0).unwrap().complete);
            }
            drop(cached);
            drop(raw);
            let mut s = Store::open(&path).unwrap();
            assert_eq!(preserved(&s.conn), original);
            assert_eq!(remaining(&s.conn), 0);
            if marked {
                assert!(s.publish_source("authored").is_err());
            }
            s.begin_source("authored").unwrap();
            s.stage_source("authored", &[pr()]).unwrap();
            s.publish_source("authored").unwrap();
            assert!(s.list_page("authored", 0).unwrap().complete);
            assert_eq!(s.list_page("authored", 0).unwrap().items.len(), 1);
        }
    }
}
