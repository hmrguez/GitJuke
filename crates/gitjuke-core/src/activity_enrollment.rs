//! Enroll one bounded batch per scheduling pass, without capping watch membership.
use crate::{Result, store::storage_healthy};
use rusqlite::{Connection, params};
use std::path::Path;

const BATCH: i64 = 128;
// Retained only as an independent test oracle for the previous complete relation.
#[cfg(test)]
const MISSING: &str = "SELECT DISTINCT l.repo,l.number FROM watch_locator l
    WHERE EXISTS(SELECT 1 FROM watch_source s WHERE s.id=l.id AND s.source!='selected')
    AND NOT EXISTS(SELECT 1 FROM activity_target t WHERE t.repo=l.repo AND t.number=l.number)";

#[cfg(test)]
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
    // Native index scans intentionally read main tables. Never enroll from a
    // partially applied publication hidden behind the protected reader views.
    if crate::publication::pending(&tx)? {
        return Err(crate::Error::StorageUnavailable(
            "Activity enrollment waits for pending publication recovery.".into(),
        ));
    }
    let before = tx.total_changes();
    let changed = enroll_in(&tx)?;
    crate::shared_storage::bounded(&tx, before)?;
    #[cfg(test)]
    crate::publication::record(&tx, before);
    #[cfg(test)]
    point("before_commit");
    tx.commit()?;
    #[cfg(test)]
    point("after_commit");
    Ok(changed)
}

// The schema-14 covering index supplies the complete native order. Keep first
// and successor statements separate so the tuple is an index lower bound.
const FIRST: &str = "SELECT repo,number,id FROM main.watch_locator INDEXED BY watch_locator_location ORDER BY repo,number,id LIMIT 128";
const NEXT: &str = "SELECT repo,number,id FROM main.watch_locator INDEXED BY watch_locator_location WHERE (repo,number,id)>(?1,?2,?3) ORDER BY repo,number,id LIMIT 128";
const ELIGIBLE: &str = "SELECT EXISTS(SELECT 1 FROM main.watch_source WHERE id=?1 AND source!='selected') AND NOT EXISTS(SELECT 1 FROM main.activity_target WHERE repo=?2 AND number=?3)";

fn enroll_in(c: &Connection) -> Result<usize> {
    let mut after: Option<(String, i64, String)> = None;
    let mut changed = 0;
    loop {
        let mut q = c.prepare(if after.is_some() { NEXT } else { FIRST })?;
        let row = |r: &rusqlite::Row<'_>| Ok((r.get(0)?, r.get(1)?, r.get(2)?));
        let rows: Vec<(String, i64, String)> = if let Some((repo, number, id)) = &after {
            q.query_map(params![repo, number, id], row)?
                .collect::<rusqlite::Result<_>>()?
        } else {
            q.query_map([], row)?.collect::<rusqlite::Result<_>>()?
        };
        crate::publication_completion::observe(&q, rows.len());
        #[cfg(test)]
        ON_WINDOW.with(|hook| {
            if let Some(f) = hook.borrow_mut().take() {
                f();
            }
        });
        for (repo, number, id) in &rows {
            // The source PK admits at most two inspected reasons: only the
            // single exact value 'selected' can be rejected. Target is a point
            // lookup, including targets inserted earlier in this transaction.
            let mut q = c.prepare(ELIGIBLE)?;
            let eligible: bool = q.query_row(params![id, repo, number], |r| r.get(0))?;
            crate::publication_completion::observe(&q, 1);
            if eligible {
                c.execute(
                    "INSERT INTO activity_target(repo,number) VALUES(?,?)",
                    params![repo, number],
                )?;
                changed += 1;
                if changed == BATCH as usize {
                    return Ok(changed);
                }
            }
        }
        if rows.len() < BATCH as usize {
            return Ok(changed);
        }
        after = rows.into_iter().last();
    }
}

#[cfg(test)]
thread_local! {static ON_WINDOW:std::cell::RefCell<Option<Box<dyn FnOnce()>>>=const{std::cell::RefCell::new(None)};}

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
        let prefix = targets(&s.conn);
        let bytes = crate::content_budget::disposable_bytes(&s.conn).unwrap();
        s.conn.execute_batch("CREATE TRIGGER fail_enroll BEFORE INSERT ON activity_target WHEN NEW.number=200 BEGIN SELECT RAISE(ABORT,'fixture enrollment fault'); END;").unwrap();
        assert!(enroll(&mut s.conn).is_err());
        assert_eq!(targets(&s.conn), prefix);
        assert_eq!(
            crate::content_budget::disposable_bytes(&s.conn).unwrap(),
            bytes
        );
        assert_eq!(count(&s.conn, "SELECT COUNT(*) FROM activity_target"), 129);
        preserved(&s, revision);
        drop(s);
        let mut s = Store::open(&path).unwrap();
        assert!(enroll(&mut s.conn).is_err());
        assert_eq!(targets(&s.conn), prefix);
        preserved(&s, revision);
        s.conn.execute_batch("DROP TRIGGER fail_enroll").unwrap();
        assert_eq!(enroll(&mut s.conn).unwrap(), 128);
        assert_eq!(enroll(&mut s.conn).unwrap(), 1);
        assert_eq!(enroll(&mut s.conn).unwrap(), 0);
        preserved(&s, revision);
    }
    fn targets(c: &Connection) -> Vec<(String, i64, i64, i64)> {
        c.prepare("SELECT repo,number,due,done FROM activity_target ORDER BY repo,number")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap()
    }
    fn oracle(c: &Connection) -> Vec<(String, i64)> {
        c.prepare(&format!("{MISSING} ORDER BY l.repo,l.number LIMIT 128"))
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap()
    }
    #[test]
    fn enrollment_windows_match_distinct_oracle_through_large_rejected_prefixes() {
        let mut reference_vm = None;
        for prefix in [1025, 8193] {
            let d = tempfile::tempdir().unwrap();
            let mut s = Store::open(&d.path().join("db")).unwrap();
            seed(&mut s);
            let tx = s.conn.transaction().unwrap();
            // All these native index candidates sort before the eligible a/b
            // suffix: absent membership, selected-only, already-enrolled, and
            // aliases of one locator spanning many candidate windows.
            for n in (0..prefix).rev() {
                for (kind, repo) in [
                    (0, "0/none"),
                    (1, "1/selected"),
                    (2, "2/enrolled"),
                    (3, "3/alias"),
                ] {
                    let id = format!("{kind}-{n:05}");
                    let number = if kind == 3 { 1 } else { n as i64 };
                    tx.execute(
                        "INSERT INTO watch_locator(id,repo,number) VALUES(?,?,?)",
                        params![id, repo, number],
                    )
                    .unwrap();
                    if kind != 0 {
                        tx.execute(
                            "INSERT INTO watch_source VALUES(?,?)",
                            params![id, if kind == 1 { "selected" } else { "direct" }],
                        )
                        .unwrap();
                    }
                    if kind == 2 {
                        tx.execute(
                            "INSERT INTO activity_target VALUES(?,?,4070908800,1)",
                            params![repo, number],
                        )
                        .unwrap();
                    }
                }
            }
            // Native signed-number endpoints, case-sensitive locators and an
            // exact-case source distinction; all retain the old SQL order.
            for (id, repo, number, source) in [
                ("min", "A/b", i64::MIN, "Selected"),
                ("max", "A/b", i64::MAX, "direct"),
                ("empty", "", 0, "authored"),
                ("utf8", "é/repo", 1, "team:x"),
            ] {
                tx.execute(
                    "INSERT INTO watch_locator(id,repo,number) VALUES(?,?,?)",
                    params![id, repo, number],
                )
                .unwrap();
                tx.execute("INSERT INTO watch_source VALUES(?,?)", params![id, source])
                    .unwrap();
            }
            // An arbitrary number of reasons cannot make the point eligibility
            // query traverse them: only one unique exact value is excluded.
            for n in 0..prefix {
                tx.execute(
                    "INSERT INTO watch_source VALUES('selected-only',?)",
                    [format!("z{n:05}")],
                )
                .unwrap();
            }
            tx.commit().unwrap();
            let revision = s.revision().unwrap();
            let bytes = crate::content_budget::disposable_bytes(&s.conn).unwrap();
            let mut windows = 0;
            let mut max_vm = 0;
            loop {
                let expected = oracle(&s.conn);
                let before = targets(&s.conn);
                crate::publication_completion::capture();
                let mutations = s.conn.total_changes();
                assert_eq!(enroll(&mut s.conn).unwrap(), expected.len());
                assert_eq!(s.conn.total_changes() - mutations, expected.len() as u64);
                let mut full = before.clone();
                full.extend(expected.iter().map(|(r, n)| (r.clone(), *n, 0, 0)));
                full.sort();
                assert_eq!(targets(&s.conn), full);
                for m in crate::publication_completion::metrics() {
                    assert!(m.rows <= 128, "{m:?}");
                    assert!(m.vm < 10000, "{m:?}");
                    assert!(m.scans <= 128, "{m:?}");
                    assert_eq!(m.sorts, 0, "{m:?}");
                    assert_eq!(m.autoindexes, 0, "{m:?}");
                    if m.rows == 128 {
                        windows += 1;
                    }
                    max_vm = max_vm.max(m.vm);
                }
                assert_eq!(s.revision().unwrap(), revision);
                assert_eq!(
                    crate::content_budget::disposable_bytes(&s.conn).unwrap(),
                    bytes
                );
                if expected.is_empty() {
                    break;
                }
            }
            println!("protected-prefix={prefix} full-windows={windows} max-statement-vm={max_vm}");
            if let Some(reference) = reference_vm {
                assert_eq!(
                    max_vm, reference,
                    "statement work must not grow with the rejected prefix"
                );
            }
            reference_vm = Some(max_vm);
            assert!(windows > prefix / 128);
        }
    }
    #[test]
    fn concurrent_target_commit_aborts_stale_enrollment_then_retry_preserves_deadline() {
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("db");
        let mut s = Store::open(&path).unwrap();
        seed(&mut s);
        s.conn.pragma_update(None, "journal_mode", "WAL").unwrap();
        let writer = Connection::open(&path).unwrap();
        ON_WINDOW.with(|hook| {
            *hook.borrow_mut() = Some(Box::new(move || {
                writer
                    .execute(
                        "INSERT INTO activity_target VALUES('a/b',1,4070908800,1)",
                        [],
                    )
                    .unwrap();
            }))
        });
        assert!(matches!(
            enroll(&mut s.conn),
            Err(crate::Error::Storage(rusqlite::Error::SqliteFailure(e, _)))
                if e.code == rusqlite::ErrorCode::DatabaseBusy
        ));
        assert!(s.conn.is_autocommit());
        assert_eq!(count(&s.conn, "SELECT COUNT(*) FROM activity_target"), 2);
        let expected = oracle(&s.conn);
        assert_eq!(enroll(&mut s.conn).unwrap(), 128);
        assert_eq!(expected.first().unwrap(), &("a/b".to_string(), 2));
        assert_eq!(
            count(
                &s.conn,
                "SELECT due FROM activity_target WHERE repo='a/b' AND number=1"
            ),
            4070908800
        );
        assert_eq!(
            count(
                &s.conn,
                "SELECT done FROM activity_target WHERE repo='a/b' AND number=1"
            ),
            1
        );
        assert_eq!(
            count(
                &s.conn,
                "SELECT MAX(number) FROM activity_target WHERE repo='a/b'"
            ),
            129
        );
    }
    #[test]
    fn enrollment_guard_counts_dependent_trigger_mutations_and_rolls_back() {
        let d = tempfile::tempdir().unwrap();
        let mut s = Store::open(&d.path().join("db")).unwrap();
        seed(&mut s);
        let before = targets(&s.conn);
        let revision = s.revision().unwrap();
        s.conn.execute_batch("CREATE TRIGGER extra_enrollment AFTER INSERT ON activity_target BEGIN INSERT INTO due VALUES('extra:'||NEW.number,1); END;").unwrap();
        assert!(enroll(&mut s.conn).is_err());
        assert_eq!(targets(&s.conn), before);
        assert_eq!(
            count(&s.conn, "SELECT COUNT(*) FROM due WHERE key LIKE 'extra:%'"),
            0
        );
        preserved(&s, revision);
        s.conn
            .execute_batch("DROP TRIGGER extra_enrollment")
            .unwrap();
        assert_eq!(enroll(&mut s.conn).unwrap(), 128);
        preserved(&s, revision);
    }
    #[test]
    fn pending_publication_blocks_native_enrollment_without_bypassing_protected_reads() {
        let d = tempfile::tempdir().unwrap();
        let mut s = Store::open(&d.path().join("db")).unwrap();
        seed(&mut s);
        let original = targets(&s.conn);
        let revision = s.revision().unwrap();
        for phase in ["planning", "applying", "committed"] {
            s.conn.execute("INSERT INTO publication(id,kind,args,phase) VALUES(1,'source','{\"source\":\"authored\",\"time\":1}',?)",[phase]).unwrap();
            crate::publication::protect(&s.conn).unwrap();
            let before = s.conn.total_changes();
            assert!(matches!(
                enroll(&mut s.conn),
                Err(crate::Error::StorageUnavailable(_))
            ));
            assert_eq!(s.conn.total_changes(), before);
            assert_eq!(targets(&s.conn), original);
            preserved(&s, revision);
            crate::publication::unprotect(&s.conn).unwrap();
            s.conn.execute("DELETE FROM publication", []).unwrap();
        }
        assert_eq!(enroll(&mut s.conn).unwrap(), 128);
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
        for (journal, phase, enrolled) in [
            ("DELETE", "before_commit", 128),
            ("DELETE", "after_commit", 256),
            ("WAL", "before_commit", 128),
            ("WAL", "after_commit", 256),
        ] {
            let d = tempfile::tempdir().unwrap();
            let path = d.path().join("db");
            let marker = d.path().join("marker");
            let mut s = Store::open(&path).unwrap();
            seed(&mut s);
            s.conn
                .execute(
                    "UPDATE content SET fetched=?1,accessed=?1",
                    [crate::now() as i64],
                )
                .unwrap();
            let revision = s.revision().unwrap();
            assert_eq!(enroll(&mut s.conn).unwrap(), 128);
            s.conn.pragma_update(None, "journal_mode", journal).unwrap();
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
            let raw_store = Store {
                conn: raw,
                cache_writes: false,
            };
            preserved(&raw_store, revision);
            let expected: Vec<_> = (1..=enrolled)
                .map(|n| ("a/b".to_string(), n, 0, 0))
                .chain(std::iter::once(("z/z".to_string(), 1, 4070908800, 1)))
                .collect();
            assert_eq!(targets(&raw_store.conn), expected);
            assert_eq!(
                crate::content_budget::disposable_bytes(&raw_store.conn).unwrap(),
                2
            );
            drop(raw_store);
            let mut cached = Store::open_cached(&path).unwrap();
            assert_eq!(
                cached.get::<serde_json::Value>("published").unwrap().data,
                Some(serde_json::json!({}))
            );
            preserved(&cached, revision);
            drop(cached);
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
