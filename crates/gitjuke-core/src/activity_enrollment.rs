//! Enroll one bounded batch per scheduling pass, without capping watch membership.
use crate::{Result, store::storage_healthy};
use rusqlite::Connection;
use std::path::Path;

const BATCH: i64 = 128;
// EXISTS avoids multiplying locators by their membership reasons. DISTINCT also
// handles different historical stable IDs sharing a repository/number locator.
const MISSING: &str = "SELECT DISTINCT l.repo,l.number FROM watch_locator l
    WHERE EXISTS(SELECT 1 FROM watch_source s WHERE s.id=l.id AND s.source!='selected')
    AND NOT EXISTS(SELECT 1 FROM activity_target t WHERE t.repo=l.repo AND t.number=l.number)";

pub(crate) fn missing(conn: &Connection) -> Result<bool> {
    Ok(conn.query_row(&format!("SELECT EXISTS({MISSING})"), [], |r| r.get(0))?)
}

pub(crate) fn enroll(conn: &mut Connection) -> Result<usize> {
    storage_healthy(
        conn.path()
            .and_then(|p| Path::new(p).parent())
            .unwrap_or(Path::new("/")),
    )?;
    let tx = conn.transaction()?;
    // Existing targets retain their deadlines and completion, including targets
    // first discovered from notifications. No cursor is needed after a restart.
    let changed = tx.execute(
        &format!(
            "INSERT INTO activity_target(repo,number) {MISSING} ORDER BY l.repo,l.number LIMIT ?"
        ),
        [BATCH],
    )?;
    #[cfg(test)]
    point("before_commit");
    tx.commit()?;
    #[cfg(test)]
    point("after_commit");
    Ok(changed)
}

#[cfg(test)]
fn point(phase: &str) {
    if std::env::var("GITJUKE_TEST_ENROLL_PHASE").as_deref() == Ok(phase) {
        std::fs::write(
            std::env::var_os("GITJUKE_TEST_ENROLL_MARKER").unwrap(),
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

    fn count(c: &Connection, sql: &str) -> i64 {
        c.query_row(sql, [], |r| r.get(0)).unwrap()
    }
    fn seed(s: &mut Store) {
        s.bind(&Identity {
            id: 42,
            login: "reader".into(),
        })
        .unwrap();
        s.activity_recovery(123).unwrap();
        let tx = s.conn.transaction().unwrap();
        for n in 1..=257 {
            tx.execute(
                "INSERT INTO watch_locator(id,repo,number) VALUES(?,'a/b',?)",
                rusqlite::params![format!("P{n}"), n],
            )
            .unwrap();
            for source in ["direct", "team:a/readers", "selected"] {
                tx.execute(
                    "INSERT INTO watch_source VALUES(?,?)",
                    rusqlite::params![format!("P{n}"), source],
                )
                .unwrap();
            }
        }
        tx.execute_batch("INSERT INTO watch_locator(id,repo,number) VALUES('duplicate','a/b',1),('selected-only','c/d',1);
            INSERT INTO watch_source VALUES('duplicate','authored'),('selected-only','selected');
            INSERT INTO activity_target VALUES('z/z',1,4070908800,1);
            INSERT INTO activity VALUES('A','a/b',1,'comment',1,1,'eligible',1);
            INSERT INTO seen VALUES('A',0); INSERT INTO dedup VALUES('A');
            INSERT INTO progress VALUES('activity:baseline','1'),('catchup:A','A');
            INSERT INTO activity_intent VALUES(1,'pr','attempted',1),(2,'summary','unavailable',1);
            INSERT INTO intent_member VALUES(1,'A'),(2,'A'); INSERT INTO alert_attempt VALUES('attempt',1);
            INSERT INTO content VALUES('published','{}',1,1);
            INSERT INTO api_validator VALUES('notifications','etag',NULL,4070908800);
            INSERT INTO due VALUES('activity:notifications',4070908800);
            INSERT INTO resource_state VALUES('authored',1,1,NULL),('direct',1,1,NULL),('teams',1,1,NULL);
            UPDATE activity_cycle SET notifications_done=1;").unwrap();
        // The duplicate authored locator needs a successful check snapshot for
        // recovery readiness; no network or classification is faked as complete.
        tx.execute_batch(
            "UPDATE watch_locator SET head='sha',last_success=1 WHERE id='duplicate';
            INSERT INTO check_state VALUES('duplicate','sha',1,1,NULL,4070908800);",
        )
        .unwrap();
        tx.commit().unwrap();
    }
    fn preserved(s: &Store, revision: u64) {
        assert_eq!(s.identity().unwrap().unwrap().id, 42);
        assert_eq!(s.revision().unwrap(), revision);
        assert_eq!(
            count(&s.conn, "SELECT due FROM activity_target WHERE repo='z/z'"),
            4070908800
        );
        assert_eq!(
            count(&s.conn, "SELECT done FROM activity_target WHERE repo='z/z'"),
            1
        );
        assert_eq!(
            count(
                &s.conn,
                "SELECT COUNT(*) FROM activity_target WHERE repo='c/d'"
            ),
            0
        );
        assert_eq!(
            count(&s.conn, "SELECT seen FROM seen WHERE source_id='A'"),
            0
        );
        for (table, expected) in [
            ("activity", 1),
            ("dedup", 1),
            ("intent_member", 2),
            ("alert_attempt", 1),
            ("content", 1),
        ] {
            assert_eq!(
                count(&s.conn, &format!("SELECT COUNT(*) FROM {table}")),
                expected
            );
        }
        assert_eq!(
            count(
                &s.conn,
                "SELECT COUNT(*) FROM activity_intent WHERE state IN ('attempted','unavailable')"
            ),
            2
        );
        assert_eq!(count(&s.conn, "SELECT boundary FROM activity_cycle"), 123);
        assert_eq!(s.activity_progress("activity:baseline").unwrap(), 1);
        assert_eq!(s.due("activity:notifications").unwrap(), 4070908800);
        assert_eq!(
            count(&s.conn, "SELECT next_at FROM api_validator"),
            4070908800
        );
    }
    #[test]
    fn bounded_enrollment_deduplicates_and_gates_recovery_until_all_targets_finish() {
        let d = tempfile::tempdir().unwrap();
        let mut s = Store::open(&d.path().join("db")).unwrap();
        seed(&mut s);
        let revision = s.revision().unwrap();
        for expected in [128, 128, 1, 0] {
            let changes = s.conn.total_changes();
            assert_eq!(enroll(&mut s.conn).unwrap(), expected);
            assert_eq!(s.conn.total_changes() - changes, expected as u64);
            preserved(&s, revision);
            // Simulate completed hydration for every enrolled target, leaving
            // missing enrollment as the sole recovery blocker on early passes.
            s.conn
                .execute("UPDATE activity_target SET done=1,due=4070908800", [])
                .unwrap();
            if expected == 128 {
                assert!(missing(&s.conn).unwrap());
                s.activity_finish_recovery(false).unwrap();
                assert_eq!(
                    count(
                        &s.conn,
                        "SELECT COUNT(*) FROM activity_cycle WHERE mode='recovery'"
                    ),
                    1
                );
                assert_eq!(count(&s.conn, "SELECT COUNT(*) FROM activity_intent"), 2);
            }
        }
        assert!(!missing(&s.conn).unwrap());
        assert_eq!(count(&s.conn, "SELECT COUNT(*) FROM activity_target"), 258);
        // Newly enrolled but unfinished work independently blocks completion.
        s.conn
            .execute("UPDATE activity_target SET done=0 WHERE number=257", [])
            .unwrap();
        s.activity_finish_recovery(false).unwrap();
        assert_eq!(
            count(
                &s.conn,
                "SELECT COUNT(*) FROM activity_cycle WHERE mode='recovery'"
            ),
            1
        );
        s.conn
            .execute("UPDATE activity_target SET done=1 WHERE number=257", [])
            .unwrap();
        s.activity_finish_recovery(false).unwrap();
        s.activity_finish_recovery(false).unwrap();
        assert_eq!(
            count(
                &s.conn,
                "SELECT COUNT(*) FROM activity_cycle WHERE mode='normal'"
            ),
            1
        );
        // A is outside retention, so no new summary is manufactured.
        assert_eq!(count(&s.conn, "SELECT COUNT(*) FROM activity_intent"), 2);
    }
    #[test]
    fn later_enrollment_fault_rolls_back_batch_and_restarts_without_resetting_targets() {
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("db");
        let mut s = Store::open(&path).unwrap();
        seed(&mut s);
        let revision = s.revision().unwrap();
        assert_eq!(enroll(&mut s.conn).unwrap(), 128);
        s.conn.execute_batch("CREATE TRIGGER fail_enroll BEFORE INSERT ON activity_target WHEN NEW.number=200 BEGIN SELECT RAISE(ABORT,'fixture enrollment fault'); END;").unwrap();
        assert!(enroll(&mut s.conn).is_err());
        assert_eq!(count(&s.conn, "SELECT COUNT(*) FROM activity_target"), 129);
        preserved(&s, revision);
        drop(s);
        let mut s = Store::open(&path).unwrap();
        assert!(enroll(&mut s.conn).is_err());
        preserved(&s, revision);
        s.conn.execute_batch("DROP TRIGGER fail_enroll").unwrap();
        assert_eq!(enroll(&mut s.conn).unwrap(), 128);
        assert_eq!(enroll(&mut s.conn).unwrap(), 1);
        assert_eq!(enroll(&mut s.conn).unwrap(), 0);
        preserved(&s, revision);
    }
    #[test]
    fn crash_child_entry() {
        let Some(path) = std::env::var_os("GITJUKE_TEST_ENROLL_DB") else {
            return;
        };
        let mut s = Store::open(Path::new(&path)).unwrap();
        enroll(&mut s.conn).unwrap();
        panic!("child must stop at enrollment barrier");
    }
    #[test]
    fn sigkill_before_and_after_later_batch_commit_preserves_restart_progress() {
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
        for (phase, enrolled) in [("before_commit", 128), ("after_commit", 256)] {
            let d = tempfile::tempdir().unwrap();
            let path = d.path().join("db");
            let marker = d.path().join("marker");
            let mut s = Store::open(&path).unwrap();
            seed(&mut s);
            let revision = s.revision().unwrap();
            assert_eq!(enroll(&mut s.conn).unwrap(), 128);
            drop(s);
            let mut child = OwnedChild(
                Command::new(std::env::current_exe().unwrap())
                    .args([
                        "--exact",
                        "activity_enrollment::tests::crash_child_entry",
                        "--nocapture",
                    ])
                    .env("GITJUKE_TEST_ENROLL_DB", &path)
                    .env("GITJUKE_TEST_ENROLL_PHASE", phase)
                    .env("GITJUKE_TEST_ENROLL_MARKER", &marker)
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
            let raw = Connection::open(&path).unwrap();
            assert_eq!(
                raw.query_row("PRAGMA quick_check(1)", [], |r| r.get::<_, String>(0))
                    .unwrap(),
                "ok"
            );
            assert_eq!(
                count(
                    &raw,
                    "SELECT COUNT(*) FROM activity_target WHERE repo='a/b'"
                ),
                enrolled
            );
            assert_eq!(
                count(
                    &raw,
                    "SELECT COUNT(*) FROM activity_target WHERE repo='a/b' AND (done!=0 OR due!=0)"
                ),
                0
            );
            drop(raw);
            let mut s = Store::open(&path).unwrap();
            preserved(&s, revision);
            while missing(&s.conn).unwrap() {
                assert!(enroll(&mut s.conn).unwrap() <= BATCH as usize);
            }
            assert_eq!(count(&s.conn, "SELECT COUNT(*) FROM activity_target"), 258);
            preserved(&s, revision);
        }
    }
}
