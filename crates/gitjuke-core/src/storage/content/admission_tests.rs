//! Real SQLite mutation accounting, full-ceiling publication and SIGKILL fixtures.
use super::*;
use crate::{model::Identity, now, store::Store};
use std::path::Path;

pub(crate) fn n(c: &Connection, sql: &str) -> i64 {
    c.query_row(sql, [], |r| r.get(0)).unwrap()
}
fn history(s: &mut Store) {
    s.bind(&Identity {
        id: 42,
        login: "reader".into(),
    })
    .unwrap();
    let mut settings = s.settings().unwrap();
    settings.paused = true;
    s.save_settings(&settings).unwrap();
    s.conn.execute_batch("INSERT INTO watch_locator(id,repo,number,head) VALUES('root','a/b',1,'sha');
        INSERT INTO watch_source VALUES('root','authored');
        INSERT INTO activity VALUES('old','a/b',1,'comment',1,1,'eligible',1);
        INSERT INTO seen VALUES('old',0);INSERT INTO dedup VALUES('old');
        INSERT INTO progress VALUES('activity:baseline','1'),('catchup:old','old');
        INSERT INTO activity_cycle VALUES(1,'recovery',123,0);
        INSERT INTO activity_intent VALUES(1,'pr','attempted',1),(2,'summary','unavailable',1);
        INSERT INTO intent_member VALUES(1,'old'),(2,'old'); INSERT INTO alert_attempt VALUES('attempt',1);
        INSERT INTO api_validator VALUES('notifications','etag',NULL,4070908800);
        INSERT INTO due VALUES('activity:notifications',4070908800);").unwrap();
}
fn durable(c: &Connection) -> Vec<Vec<rusqlite::types::Value>> {
    let mut all = vec![];
    for table in [
        "identity",
        "settings",
        "watch_locator",
        "watch_source",
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
fn mixed_rows(s: &mut Store, count: usize, bytes: usize) {
    let tx = s.conn.transaction().unwrap();
    let json = format!("\"{}\"", "x".repeat(bytes - 2));
    for i in 0..count {
        let id = format!("lru-{i:04}");
        let at = now() as i64 - 2000 + i as i64;
        if i % 2 == 0 {
            tx.execute(
                "INSERT INTO content VALUES(?,?,?,?)",
                params![id, json, at, at],
            )
            .unwrap();
        } else {
            tx.execute(
                "INSERT INTO check_state VALUES(?,'sha',?,1,NULL,?)",
                params![id, at, at],
            )
            .unwrap();
            tx.execute(
                "INSERT INTO check_context VALUES(?,'sha','ctx',?,0)",
                params![id, json],
            )
            .unwrap();
        }
    }
    tx.commit().unwrap();
}
pub(crate) fn seed_full(s: &mut Store) {
    seed_full_with_rows(s, 1024);
}
pub(crate) fn seed_full_with_rows(s: &mut Store, row_bytes: usize) {
    mixed_rows(s, 513, row_bytes);
    let mut remaining = CONTENT_LIMIT - disposable_bytes(&s.conn).unwrap();
    let tx = s.conn.transaction().unwrap();
    let mut i = 0;
    while remaining > 0 {
        let bytes = remaining.min(1024 * 1024);
        let json = if bytes == 1 {
            "0".into()
        } else {
            format!("\"{}\"", "y".repeat(bytes as usize - 2))
        };
        tx.execute(
            "INSERT INTO content VALUES(?,?,4070908800,4070908800)",
            params![format!("bulk-{i:03}"), json],
        )
        .unwrap();
        remaining -= bytes;
        i += 1;
    }
    tx.commit().unwrap();
    assert_eq!(disposable_bytes(&s.conn).unwrap(), CONTENT_LIMIT);
}
fn old_count(c: &Connection) -> i64 {
    n(
        c,
        "SELECT (SELECT COUNT(*) FROM content WHERE key GLOB 'lru-*')+(SELECT COUNT(*) FROM check_context WHERE pr_id GLOB 'lru-*' AND staged=0)",
    )
}
fn consistent(c: &Connection) {
    assert_eq!(
        n(
            c,
            "SELECT COUNT(*) FROM check_state s WHERE pr_id GLOB 'lru-*' AND complete=1 AND NOT EXISTS(SELECT 1 FROM check_context x WHERE x.pr_id=s.pr_id AND x.commit_sha=s.commit_sha AND staged=0)"
        ),
        0
    );
}
#[test]
fn mixed_reclamation_has_one_global_budget_including_check_states_and_revision() {
    let d = tempfile::tempdir().unwrap();
    let mut s = Store::open(&d.path().join("db")).unwrap();
    history(&mut s);
    mixed_rows(&mut s, 513, 4);
    // Add many contexts sharing one state: its invalidation consumes a row only once.
    s.conn
        .execute_batch(
            "INSERT INTO check_state VALUES('shared','sha',1,0,'Cached checks evicted',1);",
        )
        .unwrap();
    for i in 0..257 {
        s.conn
            .execute(
                "INSERT INTO check_context VALUES('shared','sha',?,'xx',0)",
                [i.to_string()],
            )
            .unwrap();
    }
    let original = durable(&s.conn);
    let mut batches = 0;
    loop {
        let before = s.conn.total_changes();
        let (changed, _) = reclaim_batch(&mut s.conn, i64::MAX).unwrap();
        assert!(changed <= ADMISSION_BATCH);
        assert_eq!(s.conn.total_changes() - before, changed as u64);
        assert_eq!(durable(&s.conn), original);
        consistent(&s.conn);
        batches += 1;
        if changed == 0 {
            break;
        }
    }
    assert!(batches > 7);
    assert_eq!(disposable_bytes(&s.conn).unwrap(), 0);
}
#[test]
fn speculative_admission_shares_expiry_pressure_and_check_invalidation_allowance() {
    let d = tempfile::tempdir().unwrap();
    let mut s = Store::open(&d.path().join("db")).unwrap();
    mixed_rows(&mut s, 513, 4);
    // Every independent check needs two mutations; opportunistic age cleanup
    // and pressure reclamation must share one allowance, not one per class.
    s.conn.execute("UPDATE content SET accessed=1", []).unwrap();
    s.conn
        .execute("UPDATE check_state SET accessed=1", [])
        .unwrap();
    let before = s.conn.total_changes();
    let tx = s.conn.transaction().unwrap();
    assert!(evict_to_limit(&tx, now(), 0).unwrap().is_some());
    assert!(tx.total_changes() - before <= 128);
    tx.rollback().unwrap();
    assert_eq!(old_count(&s.conn), 513);
    consistent(&s.conn);
}
#[test]
fn impossible_staging_does_not_commit_pointless_reclamation_or_candidate_progress() {
    let d = tempfile::tempdir().unwrap();
    let mut s = Store::open(&d.path().join("db")).unwrap();
    mixed_rows(&mut s, 513, 4);
    let r = s.revision().unwrap();
    let result = publish_to_limit(&mut s.conn, now(), 2052, |tx| {
        tx.execute(
            "INSERT INTO stage VALUES('too-large',?)",
            ["x".repeat(2053)],
        )?;
        tx.execute("INSERT INTO progress VALUES('must-rollback','1')", [])?;
        crate::store::bump(tx)?;
        Ok(())
    });
    assert!(matches!(result, Err(Error::Safe(_))));
    assert_eq!(old_count(&s.conn), 513);
    assert_eq!(s.revision().unwrap(), r);
    assert_eq!(n(&s.conn, "SELECT COUNT(*) FROM stage"), 0);
    assert_eq!(
        n(
            &s.conn,
            "SELECT COUNT(*) FROM progress WHERE key='must-rollback'"
        ),
        0
    );
}
#[test]
fn later_reclaim_and_publication_faults_preserve_prefix_but_never_candidate_state() {
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("db");
    let mut s = Store::open(&path).unwrap();
    history(&mut s);
    mixed_rows(&mut s, 513, 4);
    let original = durable(&s.conn);
    let write = |tx: &Transaction<'_>| -> Result<()> {
        tx.execute(
            "INSERT INTO content VALUES('new',?,4070908800,4070908800)",
            ["z".repeat(1600)],
        )?;
        tx.execute("INSERT INTO progress VALUES('publication','1')", [])?;
        crate::store::bump(tx)?;
        Ok(())
    };
    s.conn.execute_batch("CREATE TRIGGER fail_admission BEFORE DELETE ON content WHEN OLD.key='lru-0200' BEGIN SELECT RAISE(ABORT,'fixture later reclamation'); END;").unwrap();
    assert!(publish_to_limit(&mut s.conn, now(), 2052, write).is_err());
    let left = old_count(&s.conn);
    assert!(left < 513 && left > 300);
    assert_eq!(durable(&s.conn), original);
    consistent(&s.conn);
    assert_eq!(
        n(&s.conn, "SELECT COUNT(*) FROM content WHERE key='new'"),
        0
    );
    assert!(s.revision().unwrap() > 0);
    s.conn.execute_batch("DROP TRIGGER fail_admission;
        CREATE TRIGGER fail_publication BEFORE INSERT ON progress WHEN NEW.key='publication' AND EXISTS(SELECT 1 FROM check_state WHERE error='Cached checks evicted') BEGIN SELECT RAISE(ABORT,'fixture final publication'); END;").unwrap();
    assert!(publish_to_limit(&mut s.conn, now(), 2052, write).is_err());
    assert_eq!(old_count(&s.conn), left);
    assert_eq!(durable(&s.conn), original);
    drop(s);
    let raw = Connection::open(&path).unwrap();
    crate::shared_storage::register(&raw).unwrap();
    consistent(&raw);
    assert_eq!(durable(&raw), original);
    raw.execute_batch("DROP TRIGGER fail_publication").unwrap();
    drop(raw);
    let mut s = Store::open(&path).unwrap();
    publish_to_limit(&mut s.conn, now(), 2052, write).unwrap();
    assert_eq!(
        n(
            &s.conn,
            "SELECT COUNT(*) FROM progress WHERE key='publication'"
        ),
        1
    );
    assert_eq!(
        n(&s.conn, "SELECT COUNT(*) FROM content WHERE key='new'"),
        1
    );
    assert!(disposable_bytes(&s.conn).unwrap() <= 2052);
    consistent(&s.conn);
}
#[test]
fn full_ceiling_many_victims_publish_atomically_and_survive_restart() {
    let _serial = crate::content_capacity::CAPACITY.blocking_lock();
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("db");
    let mut s = Store::open(&path).unwrap();
    history(&mut s);
    seed_full(&mut s);
    let original = durable(&s.conn);
    let r = s.revision().unwrap();
    let generation = s.generation().unwrap();
    s.put("new", &"z".repeat(384 * 1024 - 2), generation)
        .unwrap();
    assert!(old_count(&s.conn) < 513 - 128);
    assert!(s.revision().unwrap() > r + 1);
    assert_eq!(durable(&s.conn), original);
    assert!(disposable_bytes(&s.conn).unwrap() <= CONTENT_LIMIT);
    consistent(&s.conn);
    drop(s);
    let mut s = Store::open(&path).unwrap();
    assert_eq!(durable(&s.conn), original);
    assert_eq!(
        s.get::<String>("new").unwrap().data.unwrap().len(),
        384 * 1024 - 2
    );
    consistent(&s.conn);
}
#[test]
fn crash_child_entry() {
    let Some(path) = std::env::var_os("GITJUKE_TEST_ADMISSION_DB") else {
        return;
    };
    let mut s = Store::open(Path::new(&path)).unwrap();
    s.conn.pragma_update(None, "cache_size", 8).unwrap();
    let g = s.generation().unwrap();
    s.put("new", &"z".repeat(384 * 1024 - 2), g).unwrap();
    panic!("child did not reach configured admission barrier");
}
#[test]
fn sigkill_full_ceiling_reclamation_and_publication_keep_only_committed_state() {
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
    let _serial = crate::content_capacity::CAPACITY.blocking_lock();
    for (phase, reclaimed, published) in [
        ("candidate_rolled_back", false, false),
        ("reclaim_before_commit", false, false),
        ("reclaim_after_commit", true, false),
        ("publication_before_commit", true, false),
        ("publication_after_commit", true, true),
    ] {
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("db");
        let marker = d.path().join("marker");
        let mut s = Store::open(&path).unwrap();
        history(&mut s);
        seed_full(&mut s);
        let original = durable(&s.conn);
        let r = s.revision().unwrap();
        s.checkpoint().unwrap();
        drop(s);
        let mut child = OwnedChild(
            Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "content_budget::admission_tests::crash_child_entry",
                    "--nocapture",
                ])
                .env("GITJUKE_TEST_ADMISSION_DB", &path)
                .env("GITJUKE_TEST_ADMISSION_PHASE", phase)
                .env("GITJUKE_TEST_ADMISSION_MARKER", &marker)
                .stdout(Stdio::null())
                .spawn()
                .unwrap(),
        );
        let start = Instant::now();
        while !marker.exists() {
            assert!(start.elapsed() < Duration::from_secs(30), "{phase}");
            assert!(child.0.try_wait().unwrap().is_none(), "{phase}");
            std::thread::sleep(Duration::from_millis(5));
        }
        child.0.kill().unwrap();
        use std::os::unix::process::ExitStatusExt;
        assert_eq!(child.0.wait().unwrap().signal(), Some(libc::SIGKILL));
        let raw = Connection::open(&path).unwrap();
        crate::shared_storage::register(&raw).unwrap();
        assert_eq!(
            raw.query_row("PRAGMA quick_check(1)", [], |r| r.get::<_, String>(0))
                .unwrap(),
            "ok"
        );
        assert_eq!(durable(&raw), original);
        consistent(&raw);
        assert_eq!(old_count(&raw) < 513, reclaimed, "{phase}");
        let expected = match phase {
            "candidate_rolled_back" | "reclaim_before_commit" => 513,
            "reclaim_after_commit" => 471,
            // Four candidate writes leave room for 41 mixed victims.
            // The preceding 343 victims commit in independent batches.
            "publication_before_commit" => 170,
            "publication_after_commit" => 129,
            _ => unreachable!(),
        };
        assert_eq!(old_count(&raw), expected, "{phase}");
        assert_eq!(
            n(&raw, "SELECT COUNT(*) FROM content WHERE key='new'") == 1,
            published,
            "{phase}"
        );
        let cached = Store::open_cached(&path).unwrap();
        assert_eq!(cached.revision().unwrap() > r, reclaimed, "{phase}");
        assert!(disposable_bytes(&raw).unwrap() <= CONTENT_LIMIT);
        drop(cached);
        drop(raw);
        let mut s = Store::open(&path).unwrap();
        let g = s.generation().unwrap();
        if !published {
            s.put("new", &"z".repeat(384 * 1024 - 2), g).unwrap();
        }
        assert_eq!(durable(&s.conn), original);
        consistent(&s.conn);
        assert_eq!(
            s.get::<String>("new").unwrap().data.unwrap().len(),
            384 * 1024 - 2
        );
    }
}

#[test]
fn repeated_candidates_do_not_publish_activity_dedup_progress_or_terminal_intents() {
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("db");
    let mut s = Store::open(&path).unwrap();
    history(&mut s);
    mixed_rows(&mut s, 513, 4);
    let mut attempts = 0;
    publish_to_limit(&mut s.conn, now(), 2052, |tx| {
        attempts += 1;
        tx.execute("INSERT INTO dedup VALUES('new')", [])?;
        tx.execute("INSERT INTO seen VALUES('new',0)", [])?;
        tx.execute(
            "INSERT INTO activity VALUES('new','a/b',1,'comment',1,1,'eligible',1)",
            [],
        )?;
        tx.execute(
            "INSERT INTO activity_intent(kind,state,created) VALUES('pr','unavailable',1)",
            [],
        )?;
        let intent = tx.last_insert_rowid();
        tx.execute("INSERT INTO intent_member VALUES(?,'new')", [intent])?;
        tx.execute("INSERT INTO progress VALUES('published','1')", [])?;
        tx.execute(
            "INSERT INTO content VALUES('activity-payload:new',?,4070908800,4070908800)",
            ["z".repeat(1600)],
        )?;
        crate::store::bump(tx)?;
        Ok(())
    })
    .unwrap();
    assert!(attempts >= 3);
    for table in ["dedup", "seen", "activity"] {
        assert_eq!(n(&s.conn, &format!("SELECT COUNT(*) FROM {table}")), 2);
    }
    assert_eq!(n(&s.conn, "SELECT COUNT(*) FROM activity_intent"), 3);
    assert_eq!(
        n(
            &s.conn,
            "SELECT COUNT(*) FROM activity_intent WHERE id=3 AND state='unavailable'"
        ),
        1
    );
    assert_eq!(n(&s.conn, "SELECT COUNT(*) FROM intent_member"), 3);
    assert_eq!(
        n(
            &s.conn,
            "SELECT COUNT(*) FROM progress WHERE key='published'"
        ),
        1
    );
    drop(s);
    let s = Store::open(&path).unwrap();
    assert_eq!(n(&s.conn, "SELECT COUNT(*) FROM activity_intent"), 3);
    assert_eq!(n(&s.conn, "SELECT COUNT(*) FROM seen WHERE seen=0"), 2);
    assert_eq!(
        n(
            &s.conn,
            "SELECT COUNT(*) FROM activity_intent WHERE state='attempted'"
        ),
        1
    );
    assert!(disposable_bytes(&s.conn).unwrap() <= 2052);
}

#[test]
fn stale_optional_publication_has_no_admission_side_effects() {
    let d = tempfile::tempdir().unwrap();
    let mut s = Store::open(&d.path().join("db")).unwrap();
    s.conn
        .execute("INSERT INTO content VALUES('expired','{}',1,1)", [])
        .unwrap();
    let before = s.conn.total_changes();
    assert!(
        publish_optional::<()>(&mut s.conn, now(), |_| Ok(None))
            .unwrap()
            .is_none()
    );
    assert_eq!(s.conn.total_changes(), before);
    assert_eq!(n(&s.conn, "SELECT COUNT(*) FROM content"), 1);
}

#[test]
fn compact_shared_snapshot_reclamation_matches_exact_revision_and_candidate_prefix() {
    let d = tempfile::tempdir().unwrap();
    let mut s = Store::open(&d.path().join("db")).unwrap();
    let time = now() as i64;
    s.conn
        .execute(
            "INSERT INTO check_state VALUES('P','sha',?,1,NULL,?)",
            [time - 100; 2],
        )
        .unwrap();
    for i in 0..64 {
        s.conn
            .execute(
                "INSERT INTO content VALUES(?,'true',?,?)",
                params![format!("small-{i:03}"), time - 200, time - 200],
            )
            .unwrap();
        s.conn
            .execute(
                "INSERT INTO check_context VALUES('P','sha',?,'true',0)",
                [format!("small-{i:03}")],
            )
            .unwrap();
    }
    let revision = s.revision().unwrap();
    let write = |tx: &Transaction<'_>| -> Result<()> {
        tx.execute(
            "INSERT INTO content VALUES('new',?,4070908800,4070908800)",
            [format!("\"{}\"", "x".repeat(382))],
        )?;
        tx.execute("INSERT INTO resource_state VALUES('new',1,1,NULL)", [])?;
        crate::store::bump(tx)?;
        Ok(())
    };
    s.conn.execute_batch("CREATE TRIGGER fail_late BEFORE DELETE ON content WHEN OLD.key='small-061' BEGIN SELECT RAISE(ABORT,'late'); END;").unwrap();
    assert!(publish_inner(&mut s.conn, now(), 512, write, || {}, |_| true, true).is_err());
    assert_eq!(s.revision().unwrap(), revision);
    assert_eq!(n(&s.conn, "SELECT COUNT(*) FROM content"), 64);
    assert_eq!(n(&s.conn, "SELECT COUNT(*) FROM check_context"), 64);
    crate::admission_query::tests::assert_exact(&s.conn);
    s.conn.execute_batch("DROP TRIGGER fail_late").unwrap();
    let observer = Connection::open(d.path().join("db")).unwrap();
    publish_inner(
        &mut s.conn,
        now(),
        512,
        write,
        || {
            assert_eq!(n(&observer, "SELECT COUNT(*) FROM content"), 13);
            assert_eq!(n(&observer, "SELECT COUNT(*) FROM check_context"), 64);
            assert_eq!(
                n(
                    &observer,
                    "SELECT CAST(value AS INTEGER) FROM meta WHERE key='revision'"
                ),
                revision as i64 + 5
            );
            assert_eq!(disposable_bytes(&observer).unwrap(), 308);
        },
        |_| true,
        true,
    )
    .unwrap();
    assert_eq!(s.revision().unwrap(), revision + 6);
    assert_eq!(
        n(
            &s.conn,
            "SELECT COUNT(*) FROM content WHERE key GLOB 'small-*'"
        ),
        0
    );
    assert_eq!(
        n(
            &s.conn,
            "SELECT COUNT(*) FROM check_context WHERE id<'small-032'"
        ),
        0
    );
    assert_eq!(n(&s.conn, "SELECT COUNT(*) FROM check_context"), 32);
    assert_eq!(disposable_bytes(&s.conn).unwrap(), 512);
    crate::admission_query::tests::assert_exact(&s.conn);
}
