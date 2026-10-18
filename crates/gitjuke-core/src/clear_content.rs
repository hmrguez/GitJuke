//! Clear publishes its logical barrier before bounded, restartable reclamation.
use crate::{
    Result,
    store::{bump, storage_healthy},
};
use rusqlite::Connection;
use std::path::Path;

const BATCH: usize = 128;
pub(crate) const MESSAGE: &str = "Downloaded content cleared";
pub(crate) fn pending(c: &Connection) -> Result<bool> {
    Ok(
        c.query_row("SELECT EXISTS(SELECT 1 FROM content_clear)", [], |r| {
            r.get(0)
        })?,
    )
}
fn admit(c: &Connection) -> Result<()> {
    storage_healthy(
        c.path()
            .and_then(|p| Path::new(p).parent())
            .unwrap_or(Path::new("/")),
    )
}
fn begin(c: &mut Connection) -> Result<()> {
    if pending(c)? {
        return Ok(());
    }
    admit(c)?;
    let tx = c.transaction()?;
    tx.execute("INSERT INTO content_clear VALUES(1)", [])?;
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
pub(crate) fn clear(c: &mut Connection) -> Result<()> {
    begin(c)?;
    resume(c)
}
pub(crate) fn resume(c: &mut Connection) -> Result<()> {
    while pending(c)? {
        admit(c)?;
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
        "DELETE FROM content WHERE rowid IN (SELECT rowid FROM content ORDER BY rowid LIMIT ?)",
        "DELETE FROM stage WHERE rowid IN (SELECT rowid FROM stage ORDER BY rowid LIMIT ?)",
        "DELETE FROM activity_stage WHERE rowid IN (SELECT rowid FROM activity_stage ORDER BY rowid LIMIT ?)",
        "DELETE FROM source_stage WHERE rowid IN (SELECT rowid FROM source_stage ORDER BY rowid LIMIT ?)",
        "DELETE FROM teams_stage WHERE rowid IN (SELECT rowid FROM teams_stage ORDER BY rowid LIMIT ?)",
        "DELETE FROM pending WHERE rowid IN (SELECT rowid FROM pending ORDER BY rowid LIMIT ?)",
        "DELETE FROM check_context WHERE rowid IN (SELECT rowid FROM check_context ORDER BY rowid LIMIT ?)",
        "UPDATE activity_target SET due=0 WHERE rowid IN (SELECT rowid FROM activity_target WHERE due!=0 ORDER BY rowid LIMIT ?)",
        "UPDATE check_state SET complete=0,error='Downloaded content cleared' WHERE rowid IN (SELECT rowid FROM check_state WHERE complete!=0 OR error IS NOT 'Downloaded content cleared' ORDER BY rowid LIMIT ?)",
        "UPDATE resource_state SET complete=0,error='Downloaded content cleared' WHERE rowid IN (SELECT rowid FROM resource_state WHERE complete!=0 OR error IS NOT 'Downloaded content cleared' ORDER BY rowid LIMIT ?)",
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
    let finished = !exhausted;
    if finished {
        changed += tx.execute("DELETE FROM content_clear", [])?;
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
    if std::env::var("GITJUKE_TEST_CLEAR_PHASE").as_deref() == Ok(phase) {
        std::fs::write(
            std::env::var_os("GITJUKE_TEST_CLEAR_MARKER").unwrap(),
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
            tx.execute(
                "INSERT INTO content VALUES(?,'{}',?,?)",
                rusqlite::params![key, now() as i64, now() as i64],
            )
            .unwrap();
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
            tx.execute("INSERT INTO check_context VALUES(?,'sha','C',?,?)", rusqlite::params![key,r#"{"id":"C","kind":"StatusContext","name":"CI","status":"SUCCESS","commit":"sha","sourceCreatedAt":"2026-10-17T00:00:00Z"}"#,i%2]).unwrap();
            tx.execute("INSERT INTO activity_target VALUES('a/b',?,123,1)", [i + 1])
                .unwrap();
            tx.execute(
                "INSERT INTO check_state VALUES(?,'sha',?,1,NULL,?)",
                rusqlite::params![key, now() as i64, now() as i64],
            )
            .unwrap();
            tx.execute("INSERT INTO resource_state VALUES(?,1,1,NULL)", [&key])
                .unwrap();
        }
        tx.execute_batch("INSERT INTO watch_locator(id,repo,number,head) VALUES('256','a/b',1,'sha');
            INSERT INTO watch_source VALUES('256','authored');
            INSERT INTO seen VALUES('A',0); INSERT INTO dedup VALUES('A');
            INSERT INTO progress VALUES('activity:baseline','1'),('catchup:A','A');
            INSERT INTO activity_intent VALUES(1,'pr','attempted',1),(2,'summary','unavailable',1);
            INSERT INTO intent_member VALUES(1,'A'),(2,'A'); INSERT INTO alert_attempt VALUES('attempt',1);
            INSERT INTO api_validator VALUES('notifications','etag',NULL,4070908800);
            INSERT INTO due VALUES('activity:notifications',4070908800);").unwrap();
        tx.execute(
            "INSERT INTO activity VALUES('A','a/b',1,'comment',?,1,'eligible',1)",
            [now() as i64],
        )
        .unwrap();
        tx.execute_batch(r#"UPDATE content SET key='activity-payload:A',json='{"id":"A","repo":"a/b","number":1,"kind":"comment","occurred":1,"eligible":true,"reason":"eligible","seen":false,"actor":"someone","url":"https://github.com/a/b/pull/1","head":null,"text":"downloaded excerpt"}' WHERE key='256';
            UPDATE resource_state SET key='authored' WHERE key='000';"#).unwrap();
        tx.commit().unwrap();
    }
    fn preserved(s: &Store, g: u64, r: u64) {
        assert_eq!(s.generation().unwrap(), g);
        assert_eq!(s.revision().unwrap(), r);
        assert_eq!(s.identity().unwrap().unwrap().id, 42);
        assert!(s.settings().unwrap().paused);
        for (table, count) in [
            ("activity", 1),
            ("seen", 1),
            ("dedup", 1),
            ("progress", 2),
            ("watch_locator", 1),
            ("watch_source", 1),
            ("activity_intent", 2),
            ("intent_member", 2),
            ("alert_attempt", 1),
            ("api_validator", 1),
            ("activity_target", 257),
        ] {
            assert_eq!(
                n(&s.conn, &format!("SELECT COUNT(*) FROM {table}")),
                count,
                "{table}"
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
            n(&s.conn, "SELECT COUNT(*) FROM activity_target WHERE done=1"),
            257
        );
        assert_eq!(n(&s.conn, "SELECT boundary FROM activity_cycle"), 123);
        assert_eq!(n(&s.conn, "SELECT next_at FROM api_validator"), 4070908800);
        assert_eq!(s.due("activity:notifications").unwrap(), 4070908800);
    }
    fn complete(s: &Store) {
        assert!(!pending(&s.conn).unwrap());
        for table in [
            "content",
            "stage",
            "activity_stage",
            "source_stage",
            "teams_stage",
            "pending",
            "check_context",
        ] {
            assert_eq!(
                n(&s.conn, &format!("SELECT COUNT(*) FROM {table}")),
                0,
                "{table}"
            );
        }
        assert_eq!(
            n(&s.conn, "SELECT COUNT(*) FROM activity_target WHERE due!=0"),
            0
        );
        for table in ["check_state", "resource_state"] {
            assert_eq!(
                n(
                    &s.conn,
                    &format!(
                        "SELECT COUNT(*) FROM {table} WHERE complete=0 AND error='Downloaded content cleared'"
                    )
                ),
                257
            );
        }
    }
    fn hidden(s: &mut Store) {
        let snap = s.get::<serde_json::Value>("activity-payload:A").unwrap();
        assert!(snap.data.is_none() && !snap.complete && snap.error.as_deref() == Some(MESSAGE));
        for view in ["authored", "requested"] {
            let page = s.list_page(view, 0).unwrap();
            assert!(
                page.items.is_empty() && !page.complete && page.error.as_deref() == Some(MESSAGE)
            );
        }
        let health = s.health().unwrap();
        assert_eq!(health.len(), 1);
        assert!(!health[0].complete && health[0].error.as_deref() == Some(MESSAGE));
        let checks = s.check_page("a/b", 1, 0).unwrap();
        assert!(
            checks.items.is_empty()
                && checks.pr.is_none()
                && checks.error.as_deref() == Some(MESSAGE)
        );
        assert!(
            s.activity_check_records("a/b", 1, true, "sha")
                .unwrap()
                .is_empty()
        );
        let a = s.activity_page(0).unwrap();
        assert_eq!(a.items.len(), 1);
        assert!(!a.items[0].seen);
        assert!(a.items[0].text.is_none() && a.items[0].url.is_empty());
    }
    #[test]
    fn clear_global_bound_and_logical_barrier_preserve_durable_history() {
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("db");
        let mut s = Store::open(&path).unwrap();
        seed(&mut s);
        assert!(
            !s.activity_check_records("a/b", 1, true, "sha")
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            s.activity_page(0).unwrap().items[0].text.as_deref(),
            Some("downloaded excerpt")
        );
        let g = s.generation().unwrap() + 1;
        let r = s.revision().unwrap() + 1;
        let changes = s.conn.total_changes();
        begin(&mut s.conn).unwrap();
        assert_eq!(s.conn.total_changes() - changes, 3);
        assert_eq!(
            n(
                &s.conn,
                "SELECT complete FROM check_state WHERE pr_id='256'"
            ),
            1
        );
        hidden(&mut s);
        let mut protected = Store::open_cached(&path).unwrap();
        hidden(&mut protected);
        let mut batches = 0;
        while pending(&s.conn).unwrap() {
            let before = s.conn.total_changes();
            let changed = batch(&mut s.conn).unwrap();
            assert!(changed > 0 && changed <= 128);
            assert_eq!(s.conn.total_changes() - before, changed as u64);
            preserved(&s, g, r);
            batches += 1;
        }
        assert_eq!(batches, 35);
        complete(&s);
        let before = s.conn.total_changes();
        resume(&mut s.conn).unwrap();
        assert_eq!(s.conn.total_changes(), before);
        assert!(!s.put("late", &"obsolete", g - 1).unwrap());
        drop(s);
        let s = Store::open(&path).unwrap();
        complete(&s);
        preserved(&s, g, r);
    }
    #[test]
    fn clear_barrier_late_batch_and_final_marker_faults_roll_back_and_resume() {
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("db");
        let mut s = Store::open(&path).unwrap();
        seed(&mut s);
        let g = s.generation().unwrap();
        let r = s.revision().unwrap();
        s.conn.execute_batch("CREATE TRIGGER fail_barrier BEFORE UPDATE ON meta WHEN NEW.key='revision' BEGIN SELECT RAISE(ABORT,'fixture barrier'); END;").unwrap();
        assert!(s.clear().is_err());
        assert!(!pending(&s.conn).unwrap());
        preserved(&s, g, r);
        assert_eq!(n(&s.conn, "SELECT COUNT(*) FROM content"), 257);
        s.conn.execute_batch("DROP TRIGGER fail_barrier; CREATE TRIGGER fail_late BEFORE DELETE ON content WHEN OLD.key='200' BEGIN SELECT RAISE(ABORT,'fixture late'); END;").unwrap();
        assert!(s.clear().is_err());
        assert_eq!(n(&s.conn, "SELECT COUNT(*) FROM content"), 65);
        assert!(s.clear().is_err());
        preserved(&s, g + 1, r + 1);
        hidden(&mut s);
        drop(s);
        assert!(Store::open(&path).is_err());
        let mut protected = Store::open_cached(&path).unwrap();
        hidden(&mut protected);
        let c = Connection::open(&path).unwrap();
        c.execute_batch("DROP TRIGGER fail_late; CREATE TRIGGER fail_finish BEFORE DELETE ON content_clear BEGIN SELECT RAISE(ABORT,'fixture finish'); END;").unwrap();
        assert!(Store::open(&path).is_err());
        assert!(pending(&c).unwrap());
        assert_eq!(
            n(&c, "SELECT COUNT(*) FROM resource_state WHERE complete=1"),
            13
        );
        c.execute_batch("DROP TRIGGER fail_finish").unwrap();
        drop(c);
        let s = Store::open(&path).unwrap();
        complete(&s);
        preserved(&s, g + 1, r + 1);
    }
    #[test]
    fn clear_barrier_blocks_otherwise_ready_recovery_completion() {
        let d = tempfile::tempdir().unwrap();
        let mut s = Store::open(&d.path().join("db")).unwrap();
        seed(&mut s);
        let mut settings = s.settings().unwrap();
        settings.paused = false;
        s.save_settings(&settings).unwrap();
        s.conn
            .execute_batch(
                "DELETE FROM pending;
            UPDATE watch_locator SET last_success=1;
            UPDATE activity_cycle SET notifications_done=1;
            INSERT INTO resource_state VALUES('direct',1,1,NULL),('teams',1,1,NULL);",
            )
            .unwrap();
        assert!(!crate::activity_enrollment::missing(&s.conn).unwrap());
        // At the barrier every physical check/source snapshot is still complete.
        begin(&mut s.conn).unwrap();
        let r = s.revision().unwrap();
        s.activity_finish_recovery(false).unwrap();
        assert_eq!(s.revision().unwrap(), r);
        assert_eq!(
            n(
                &s.conn,
                "SELECT COUNT(*) FROM activity_cycle WHERE mode='recovery'"
            ),
            1
        );
        // Removing only the logical barrier proves that the other readiness
        // conditions permit completion (no reclamation has run in this fixture).
        s.conn.execute("DELETE FROM content_clear", []).unwrap();
        s.activity_finish_recovery(false).unwrap();
        assert_eq!(
            n(
                &s.conn,
                "SELECT COUNT(*) FROM activity_cycle WHERE mode='normal'"
            ),
            1
        );
        assert_eq!(n(&s.conn, "SELECT COUNT(*) FROM activity_intent"), 3);
    }
    #[test]
    fn clear_unchanged_rows_do_not_consume_the_batch_allowance() {
        let d = tempfile::tempdir().unwrap();
        let mut s = Store::open(&d.path().join("db")).unwrap();
        for i in 0..1000 {
            s.conn
                .execute(
                    "INSERT INTO check_state VALUES(?,'sha',1,0,'Downloaded content cleared',1)",
                    [i.to_string()],
                )
                .unwrap();
            s.conn
                .execute("INSERT INTO activity_target VALUES('a/b',?,0,1)", [i + 1])
                .unwrap();
        }
        begin(&mut s.conn).unwrap();
        let before = s.conn.total_changes();
        assert_eq!(batch(&mut s.conn).unwrap(), 1);
        assert_eq!(s.conn.total_changes() - before, 1);
        assert!(!pending(&s.conn).unwrap());
    }
    #[test]
    fn crash_child_entry() {
        let Some(path) = std::env::var_os("GITJUKE_TEST_CLEAR_DB") else {
            return;
        };
        let conn = Connection::open(Path::new(&path)).unwrap();
        conn.pragma_update(None, "synchronous", "FULL").unwrap();
        Store {
            conn,
            cache_writes: true,
        }
        .clear()
        .unwrap();
        panic!("child missed clear barrier");
    }
    #[test]
    fn clear_sigkill_barrier_and_batch_commits_resume_before_startup() {
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
        for (phase, content, remaining, marked) in [
            ("barrier_committed", 257, 257, true),
            ("batch_before_commit", 257, 257, true),
            ("batch_after_commit", 193, 257, true),
            ("finish_before_commit", 0, 13, true),
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
                        "clear_content::tests::crash_child_entry",
                        "--nocapture",
                    ])
                    .env("GITJUKE_TEST_CLEAR_DB", &path)
                    .env("GITJUKE_TEST_CLEAR_PHASE", phase)
                    .env("GITJUKE_TEST_CLEAR_MARKER", &marker)
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
            assert_eq!(n(&raw, "SELECT COUNT(*) FROM content"), content);
            assert_eq!(
                n(&raw, "SELECT COUNT(*) FROM resource_state WHERE complete=1"),
                remaining
            );
            let mut cached = Store::open_cached(&path).unwrap();
            preserved(&cached, g, r);
            if marked {
                hidden(&mut cached);
            }
            drop(cached);
            drop(raw);
            let s = Store::open(&path).unwrap();
            complete(&s);
            preserved(&s, g, r);
        }
    }
}
