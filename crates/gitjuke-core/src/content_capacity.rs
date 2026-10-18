//! Full production-ceiling qualification. All fixtures/hooks are test-only.
use crate::{
    activity::ActivityRecord,
    content_budget::disposable_bytes,
    model::Identity,
    now,
    store::{CONTENT_LIMIT, Store, evict},
};
use rusqlite::{Connection, params};
use std::path::Path;

// Keep these disk-backed fixtures sequential within the workspace test process.
// No production ceiling, page size, synchronization or cache policy is reduced.
pub(crate) static CAPACITY: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
const MIB: usize = 1024 * 1024;
const SMALL: usize = 4096;
const NEW_BYTES: usize = 384 * 1024;

pub(crate) fn point(phase: &str) {
    if std::env::var("GITJUKE_TEST_CAPACITY_PHASE").as_deref() == Ok(phase) {
        std::fs::write(
            std::env::var_os("GITJUKE_TEST_CAPACITY_MARKER").unwrap(),
            phase,
        )
        .unwrap();
        loop {
            std::thread::park();
        }
    }
}

pub(crate) fn count(c: &Connection, sql: &str) -> i64 {
    c.query_row(sql, [], |r| r.get(0)).unwrap()
}
fn text(bytes: usize) -> String {
    // A complete JSON string of exactly `bytes` UTF-8 bytes, including quotes.
    // Multibyte text makes a character-count implementation fail the boundary.
    format!("\"{}\"", "é".repeat((bytes - 2) / 2))
}
fn value() -> String {
    "x".repeat(NEW_BYTES - 2)
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
    s.activity_recovery(123).unwrap();
    s.conn.execute_batch("INSERT INTO watch_locator(id,repo,number,head) VALUES('P','a/b',1,'sha');
        INSERT INTO watch_source VALUES('P','authored');
        INSERT INTO activity VALUES('A','a/b',1,'comment',1,1,'eligible',1);
        INSERT INTO seen VALUES('A',0); INSERT INTO dedup VALUES('A');
        INSERT INTO progress VALUES('activity:baseline','1'),('catchup:A','A');
        INSERT INTO activity_intent VALUES(1,'pr','attempted',1),(2,'summary','unavailable',1);
        INSERT INTO intent_member VALUES(1,'A'),(2,'A'); INSERT INTO alert_attempt VALUES('attempt',1);
        INSERT INTO api_validator VALUES('notifications','etag',NULL,4070908800);
        INSERT INTO due VALUES('activity:notifications',4070908800);").unwrap();
}
fn preserved(c: &Connection, revision: u64) {
    assert_eq!(count(c, "SELECT user_id FROM identity"), 42);
    assert_eq!(
        count(c, "SELECT json_extract(json,'$.paused') FROM settings"),
        1
    );
    assert_eq!(
        count(
            c,
            "SELECT CAST(value AS INTEGER) FROM meta WHERE key='revision'"
        ),
        revision as i64
    );
    assert_eq!(
        count(
            c,
            "SELECT COUNT(*) FROM watch_source WHERE source='authored'"
        ),
        1
    );
    assert_eq!(
        count(c, "SELECT COUNT(*) FROM watch_locator WHERE head='sha'"),
        1
    );
    assert_eq!(count(c, "SELECT seen FROM seen WHERE source_id='A'"), 0);
    assert_eq!(
        count(
            c,
            "SELECT boundary FROM activity_cycle WHERE mode='recovery'"
        ),
        123
    );
    for (table, n) in [
        ("activity", 1),
        ("seen", 1),
        ("dedup", 1),
        ("progress", 2),
        ("intent_member", 2),
        ("alert_attempt", 1),
    ] {
        assert_eq!(
            count(c, &format!("SELECT COUNT(*) FROM {table}")),
            n,
            "{table}"
        );
    }
    assert_eq!(
        count(
            c,
            "SELECT COUNT(*) FROM activity_intent WHERE state IN ('attempted','unavailable')"
        ),
        2
    );
    assert_eq!(count(c, "SELECT next_at FROM api_validator"), 4070908800);
    assert_eq!(
        count(c, "SELECT at FROM due WHERE key='activity:notifications'"),
        4070908800
    );
}

fn mixed(s: &mut Store) {
    history(s);
    mixed_payloads(s);
}
pub(crate) fn mixed_payloads(s: &mut Store) {
    let time = now() as i64;
    let tx = s.conn.transaction().unwrap();
    let bulk = text(MIB);
    for n in 0..499 {
        let key = format!("bulk-{n:03}");
        match n % 5 {
            0 => {
                tx.execute(
                    "INSERT INTO content VALUES(?,?,?,?)",
                    params![key, bulk, time - 10, time - 10],
                )
                .unwrap();
            }
            1 => {
                tx.execute("INSERT INTO stage VALUES(?,?)", params![key, bulk])
                    .unwrap();
            }
            2 => {
                tx.execute(
                    "INSERT INTO source_stage VALUES('direct',?,?)",
                    params![key, bulk],
                )
                .unwrap();
            }
            3 => {
                tx.execute(
                    "INSERT INTO check_context VALUES('P','sha',?,?,1)",
                    params![key, bulk],
                )
                .unwrap();
            }
            _ => {
                tx.execute(
                    "INSERT INTO activity_stage VALUES('a/b',1,?,?)",
                    params![key, bulk],
                )
                .unwrap();
            }
        }
    }
    tx.execute(
        "INSERT INTO content VALUES('half',?,?,?)",
        params![text(MIB / 2), time - 10, time - 10],
    )
    .unwrap();
    tx.execute(
        "INSERT INTO check_state VALUES('P','sha',?,1,NULL,?)",
        params![time - 100, time - 100],
    )
    .unwrap();
    let small = text(SMALL);
    for n in 0..64 {
        tx.execute(
            "INSERT INTO content VALUES(?,?,?,?)",
            params![format!("small-{n:03}"), small, time - 200, time - 200],
        )
        .unwrap();
        tx.execute(
            "INSERT INTO check_context VALUES('P','sha',?,?,0)",
            params![format!("small-{n:03}"), small],
        )
        .unwrap();
    }
    tx.commit().unwrap();
    assert_eq!(disposable_bytes(&s.conn).unwrap(), CONTENT_LIMIT);
}
fn small_content(c: &Connection) -> i64 {
    count(c, "SELECT COUNT(*) FROM content WHERE key GLOB 'small-*'")
}
pub(crate) fn small_checks(c: &Connection) -> i64 {
    count(c, "SELECT COUNT(*) FROM check_context WHERE staged=0")
}
fn publication(c: &Connection, committed: bool) {
    assert_eq!(small_content(c), if committed { 0 } else { 64 });
    assert_eq!(small_checks(c), if committed { 32 } else { 64 });
    assert_eq!(
        count(
            c,
            "SELECT complete FROM check_state WHERE pr_id='P' AND commit_sha='sha'"
        ),
        i64::from(!committed)
    );
    assert_eq!(
        count(c, "SELECT COUNT(*) FROM content WHERE key='new'"),
        i64::from(committed)
    );
    assert_eq!(
        count(
            c,
            "SELECT COUNT(*) FROM resource_state WHERE key='new' AND complete=1"
        ),
        i64::from(committed)
    );
    // Deterministic tie order removes the first 32 check IDs only.
    if committed {
        assert_eq!(
            count(
                c,
                "SELECT COUNT(*) FROM check_context WHERE staged=0 AND id<'small-032'"
            ),
            0
        );
        let error: String = c
            .query_row("SELECT error FROM check_state", [], |r| r.get(0))
            .unwrap();
        assert_eq!(error, "Cached checks evicted");
        let bytes: i64 = c
            .query_row(
                "SELECT length(CAST(json AS BLOB)) FROM content WHERE key='new'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(bytes, NEW_BYTES as i64);
    }
}
fn published_bytes(c: &Connection) -> i64 {
    count(
        c,
        "SELECT (SELECT COALESCE(SUM(length(CAST(json AS BLOB))),0) FROM content)+(SELECT COALESCE(SUM(length(CAST(json AS BLOB))),0) FROM check_context WHERE staged=0)",
    )
}
pub(crate) fn disk_backed(s: &Store, path: &Path) {
    let pages = count(&s.conn, "PRAGMA page_count");
    let size = count(&s.conn, "PRAGMA page_size");
    assert!(pages * size >= CONTENT_LIMIT);
    s.checkpoint().unwrap();
    assert!(std::fs::metadata(path).unwrap().len() >= CONTENT_LIMIT as u64);
    assert_eq!(count(&s.conn, "PRAGMA synchronous"), 2);
}

#[test]
fn full_ceiling_mixed_lru_is_inclusive_disk_backed_and_survives_restart() {
    let _serial = CAPACITY.blocking_lock();
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("db");
    let mut s = Store::open(&path).unwrap();
    mixed(&mut s);
    disk_backed(&s, &path);
    let revision = s.revision().unwrap();
    let changes = s.conn.total_changes();
    {
        let tx = s.conn.transaction().unwrap();
        evict(&tx, now()).unwrap();
        tx.commit().unwrap();
    }
    assert_eq!(
        s.conn.total_changes(),
        changes,
        "exact fit must not evict/invalidate"
    );
    publication(&s.conn, false);
    assert_eq!(
        s.storage_usage(&path).unwrap().disposable_bytes,
        CONTENT_LIMIT as u64
    );
    let generation = s.generation().unwrap();
    assert!(s.put("new", &value(), generation).unwrap());
    publication(&s.conn, true);
    assert_eq!(disposable_bytes(&s.conn).unwrap(), CONTENT_LIMIT);
    preserved(&s.conn, revision + 6);
    let payload = s.get::<String>("new").unwrap().data.unwrap();
    assert_eq!(payload, value());
    let published = published_bytes(&s.conn);
    drop(s);
    let mut s = Store::open(&path).unwrap();
    publication(&s.conn, true);
    preserved(&s.conn, revision + 6);
    // Startup intentionally discards abandoned staging, never published data.
    assert_eq!(disposable_bytes(&s.conn).unwrap(), published);
    assert_eq!(s.get::<String>("new").unwrap().data.unwrap(), value());
}

#[test]
fn full_ceiling_late_delete_fault_restores_payloads_completeness_and_revision() {
    let _serial = CAPACITY.blocking_lock();
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("db");
    let mut s = Store::open(&path).unwrap();
    mixed(&mut s);
    disk_backed(&s, &path);
    let revision = s.revision().unwrap();
    let generation = s.generation().unwrap();
    // Fail late in the first candidate, before independent reclamation.
    // Payloads, charge updates and the new resource must roll back together.
    s.conn.execute_batch("CREATE TRIGGER fail_capacity BEFORE DELETE ON content WHEN OLD.key='small-061' BEGIN SELECT RAISE(ABORT,'fixture full-capacity fault'); END;").unwrap();
    let changes = s.conn.total_changes();
    assert!(matches!(
        s.put("new", &value(), generation),
        Err(crate::Error::Storage(_))
    ));
    // Earlier payload/charge writes ran, although the transaction rolls back.
    assert!(s.conn.total_changes() - changes >= 35);
    publication(&s.conn, false);
    preserved(&s.conn, revision);
    assert_eq!(disposable_bytes(&s.conn).unwrap(), CONTENT_LIMIT);
    s.conn.execute_batch("DROP TRIGGER fail_capacity").unwrap();
    assert!(s.put("new", &value(), generation).unwrap());
    publication(&s.conn, true);
    preserved(&s.conn, revision + 6);
    assert_eq!(disposable_bytes(&s.conn).unwrap(), CONTENT_LIMIT);
}

#[test]
fn full_ceiling_non_evictable_staging_refuses_real_activity_ingestion_atomically() {
    let _serial = CAPACITY.blocking_lock();
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("db");
    let mut s = Store::open(&path).unwrap();
    history(&mut s);
    let tx = s.conn.transaction().unwrap();
    let bulk = text(MIB);
    for n in 0..500 {
        let key = format!("bulk-{n:03}");
        match n % 4 {
            0 => {
                tx.execute("INSERT INTO stage VALUES(?,?)", params![key, bulk])
                    .unwrap();
            }
            1 => {
                tx.execute(
                    "INSERT INTO source_stage VALUES('direct',?,?)",
                    params![key, bulk],
                )
                .unwrap();
            }
            2 => {
                tx.execute(
                    "INSERT INTO check_context VALUES('P','sha',?,?,1)",
                    params![key, bulk],
                )
                .unwrap();
            }
            _ => {
                tx.execute(
                    "INSERT INTO activity_stage VALUES('a/b',1,?,?)",
                    params![key, bulk],
                )
                .unwrap();
            }
        }
    }
    tx.commit().unwrap();
    disk_backed(&s, &path);
    let revision = s.revision().unwrap();
    assert_eq!(disposable_bytes(&s.conn).unwrap(), CONTENT_LIMIT);
    // A one-byte excess is rejected at the real ceiling, without deleting any
    // incomplete traversal. The app's Activity writer is tested next as well.
    {
        let tx = s.conn.transaction().unwrap();
        tx.execute("UPDATE stage SET json=json||' ' WHERE id='bulk-000'", [])
            .unwrap();
        assert_eq!(disposable_bytes(&tx).unwrap(), CONTENT_LIMIT + 1);
        assert!(matches!(evict(&tx, now()), Err(crate::Error::Safe(_))));
    }
    assert_eq!(disposable_bytes(&s.conn).unwrap(), CONTENT_LIMIT);
    let record = ActivityRecord {
        id: "rejected".into(),
        repo: "c/d".into(),
        number: 2,
        kind: "comment".into(),
        occurred: now(),
        actor: Some("other".into()),
        url: "https://github.com/c/d/pull/2#issuecomment-1".into(),
        text: Some("source".into()),
        eligible: true,
        reason: "eligible".into(),
        seen: false,
        head: None,
    };
    let err = s.activity_stage("c/d", 2, &[record], false).unwrap_err();
    assert!(matches!(err, crate::Error::Safe(ref m) if m.contains("content budget")));
    assert_eq!(disposable_bytes(&s.conn).unwrap(), CONTENT_LIMIT);
    assert_eq!(
        count(
            &s.conn,
            "SELECT COUNT(*) FROM activity_stage WHERE repo='c/d'"
        ),
        0
    );
    preserved(&s.conn, revision);
    for table in ["stage", "source_stage", "check_context", "activity_stage"] {
        assert_eq!(
            count(&s.conn, &format!("SELECT COUNT(*) FROM {table}")),
            125
        );
    }
    drop(s);
    let s = Store::open(&path).unwrap();
    assert_eq!(disposable_bytes(&s.conn).unwrap(), 0);
    preserved(&s.conn, revision);
}

#[test]
fn crash_child_entry() {
    let Some(path) = std::env::var_os("GITJUKE_TEST_CAPACITY_DB") else {
        return;
    };
    let mut s = Store::open(Path::new(&path)).unwrap();
    // Startup discards staging; the crash fixtures use published-only filler.
    // Small child cache proves writes reach WAL before pre-commit SIGKILL.
    s.conn.pragma_update(None, "cache_size", 8).unwrap();
    let generation = s.generation().unwrap();
    s.put("new", &value(), generation).unwrap();
    panic!("child must stop at capacity barrier");
}

#[test]
fn full_ceiling_sigkill_keeps_atomic_candidate_after_bounded_reclamation() {
    use std::{
        process::{Child, Command, Stdio},
        time::{Duration, Instant},
    };
    let _serial = CAPACITY.blocking_lock();
    struct OwnedChild(Child);
    impl Drop for OwnedChild {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    for (phase, committed) in [("before_commit", false), ("after_commit", true)] {
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("db");
        let marker = d.path().join("marker");
        let mut s = Store::open(&path).unwrap();
        mixed(&mut s);
        // Convert fixture staging into ordinary published filler before closing.
        // Production startup must not shrink the tested ceiling.
        s.conn.execute_batch("INSERT INTO content SELECT 'legacy:'||id,json,4070908800,4070908800 FROM stage;
            INSERT INTO content SELECT 'source:'||id,json,4070908800,4070908800 FROM source_stage;
            INSERT INTO content SELECT 'check:'||id,json,4070908800,4070908800 FROM check_context WHERE staged=1;
            INSERT INTO content SELECT 'activity:'||id,json,4070908800,4070908800 FROM activity_stage;
            DELETE FROM stage; DELETE FROM source_stage; DELETE FROM check_context WHERE staged=1; DELETE FROM activity_stage;").unwrap();
        assert_eq!(disposable_bytes(&s.conn).unwrap(), CONTENT_LIMIT);
        disk_backed(&s, &path);
        let revision = s.revision().unwrap();
        drop(s);
        let mut child = OwnedChild(
            Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "content_capacity::crash_child_entry",
                    "--nocapture",
                ])
                .env("GITJUKE_TEST_CAPACITY_DB", &path)
                .env("GITJUKE_TEST_CAPACITY_PHASE", phase)
                .env("GITJUKE_TEST_CAPACITY_MARKER", &marker)
                .stdout(Stdio::null())
                .spawn()
                .unwrap(),
        );
        let start = Instant::now();
        while !marker.exists() {
            assert!(start.elapsed() < Duration::from_secs(30));
            assert!(child.0.try_wait().unwrap().is_none());
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(std::fs::metadata(d.path().join("db-wal")).unwrap().len() > 32);
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
        if committed {
            publication(&raw, true);
        } else {
            reclaimed_before_candidate(&raw);
        }
        preserved(&raw, revision + 5 + u64::from(committed));
        assert_eq!(
            disposable_bytes(&raw).unwrap(),
            CONTENT_LIMIT - if committed { 0 } else { 51 * SMALL as i64 }
        );
        drop(raw);
        let mut s = Store::open(&path).unwrap();
        if committed {
            publication(&s.conn, true);
        } else {
            reclaimed_before_candidate(&s.conn);
        }
        preserved(&s.conn, revision + 5 + u64::from(committed));
        assert_eq!(
            disposable_bytes(&s.conn).unwrap(),
            CONTENT_LIMIT - if committed { 0 } else { 51 * SMALL as i64 }
        );
        let generation = s.generation().unwrap();
        assert!(s.put("new", &value(), generation).unwrap());
        publication(&s.conn, true);
        assert_eq!(disposable_bytes(&s.conn).unwrap(), CONTENT_LIMIT);
    }
}

// Derived and checked by the compact shared-snapshot fixture. The candidate
// remains atomic; five prior disposable-only reclamation commits survive it.
fn reclaimed_before_candidate(c: &Connection) {
    assert_eq!(small_content(c), 13);
    assert_eq!(small_checks(c), 64);
    assert_eq!(
        count(
            c,
            "SELECT complete FROM check_state WHERE pr_id='P' AND commit_sha='sha'"
        ),
        1
    );
    assert_eq!(count(c, "SELECT COUNT(*) FROM content WHERE key='new'"), 0);
    assert_eq!(
        count(c, "SELECT COUNT(*) FROM resource_state WHERE key='new'"),
        0
    );
}
