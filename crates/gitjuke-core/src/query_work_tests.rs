use crate::{
    now, publication, retention,
    retention_window::{self as window, Class},
    store::Store,
};
use rusqlite::{Connection, params};
use std::path::Path;
const CLASSES: [Class; 5] = [
    Class::Orphans,
    Class::Checks,
    Class::Validators,
    Class::Resources,
    Class::States,
];
fn n(c: &Connection, sql: &str) -> i64 {
    c.query_row(sql, [], |r| r.get(0)).unwrap()
}
fn fixture(path: &Path, count: usize) -> Store {
    let mut s = Store::open(path).unwrap();
    let time = now() as i64;
    retention::tests::seed(&mut s, 1, time as u64);
    s.bind(&crate::model::Identity {
        id: 42,
        login: "reader".into(),
    })
    .unwrap();
    let mut settings = s.settings().unwrap();
    settings.paused = true;
    s.save_settings(&settings).unwrap();
    s.put(
        "safe",
        &serde_json::json!({"cached":"retained"}),
        s.generation().unwrap(),
    )
    .unwrap();
    let tx = s.conn.transaction().unwrap();
    tx.execute_batch("INSERT INTO due VALUES('activity:notifications',4070908800);INSERT INTO activity_cycle VALUES(1,'recovery',123,0);").unwrap();
    for i in 0..count {
        let id = format!("keep-{i:06}");
        let old = format!("old-{i:06}");
        let record = crate::activity_crash::record(&id, time as u64);
        tx.execute(
            "INSERT INTO activity VALUES(?,'a/b',1,'comment',?,1,'eligible',?)",
            params![id, time, time],
        )
        .unwrap();
        tx.execute("INSERT INTO seen VALUES(?,0)", [&id]).unwrap();
        tx.execute("INSERT INTO dedup VALUES(?)", [&id]).unwrap();
        tx.execute(
            "INSERT INTO content VALUES(?,?,?,?)",
            params![
                format!("activity-payload:{id}"),
                serde_json::to_string(&record).unwrap(),
                time,
                time
            ],
        )
        .unwrap();
        tx.execute(
            "INSERT INTO api_validator VALUES(?,'etag',NULL,4070908800)",
            [&id],
        )
        .unwrap();
        tx.execute(
            "INSERT INTO resource_state VALUES(?,1,1,NULL)",
            [format!("api:{id}")],
        )
        .unwrap();
        tx.execute(
            "INSERT INTO check_state VALUES(?,'current',1,1,NULL,?)",
            params![id, time],
        )
        .unwrap();
        tx.execute(
            "INSERT INTO check_context VALUES(?,'current','ctx','{}',0)",
            [&id],
        )
        .unwrap();
        tx.execute(
            "INSERT INTO check_state VALUES(?,'current',1,1,NULL,1)",
            [&old],
        )
        .unwrap();
        tx.execute(
            "INSERT INTO watch_locator(id,repo,number,head) VALUES(?,'old/repo',?,'current')",
            params![old, i as i64],
        )
        .unwrap();
        tx.execute("INSERT INTO watch_source VALUES(?,'direct')", [old])
            .unwrap();
    }
    tx.commit().unwrap();
    s
}
fn tails(s: &mut Store) {
    s.conn.execute_batch("INSERT INTO content VALUES('activity-payload:zzz','{}',4070908800,4070908800);INSERT INTO api_validator VALUES('zzz','etag',NULL,0);INSERT INTO resource_state VALUES('api:zzz',1,1,NULL);INSERT INTO check_state VALUES('zzz','sha',1,1,NULL,1);INSERT INTO check_context VALUES('zzz','sha','ctx','{}',0);").unwrap();
}
fn remaining(c: &Connection) -> [i64; 5] {
    [
        n(
            c,
            "SELECT COUNT(*) FROM content WHERE key='activity-payload:zzz'",
        ),
        n(c, "SELECT COUNT(*) FROM check_context WHERE pr_id='zzz'"),
        n(c, "SELECT COUNT(*) FROM api_validator WHERE url='zzz'"),
        n(c, "SELECT COUNT(*) FROM resource_state WHERE key='api:zzz'"),
        n(c, "SELECT COUNT(*) FROM check_state WHERE pr_id='zzz'"),
    ]
}
fn cursors(c: &Connection) -> Vec<(String, String, String)> {
    c.prepare("SELECT class,position,ceiling FROM retention_cursor ORDER BY class")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap()
}
fn history(c: &Connection) -> Vec<Vec<Vec<rusqlite::types::Value>>> {
    [
        "seen",
        "dedup",
        "activity_intent",
        "intent_member",
        "alert_attempt",
        "due",
        "identity",
        "settings",
        "watch_source",
        "activity_cycle",
        "progress WHERE key NOT GLOB 'catchup:*'",
    ]
    .iter()
    .map(|t| {
        let mut q = c
            .prepare(&format!("SELECT * FROM {t} ORDER BY rowid"))
            .unwrap();
        let cols = q.column_count();
        q.query_map([], |r| (0..cols).map(|i| r.get(i)).collect())
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap()
    })
    .collect()
}
fn bounded() {
    for m in window::metrics() {
        assert!(m.rows <= 128, "{m:?}");
        assert!(m.vm < 50_000, "{m:?}");
        assert!(m.scans <= 254, "{m:?}");
        assert_eq!(m.sorts, 0, "{m:?}");
        assert_eq!(m.autoindexes, 0, "{m:?}");
    }
}
#[test]
fn indexed_windows_bound_scans_and_sorts_across_large_protected_prefixes() {
    let mut reference = None;
    for count in [1024, 32768] {
        let d = tempfile::tempdir().unwrap();
        let mut s = fixture(&d.path().join("db"), count);
        {
            use rusqlite::StatementStatus::*;
            let mut q = s.conn.prepare(retention::ACTIVITY_CANDIDATES).unwrap();
            let rows = q
                .query_map(
                    params![now().saturating_sub(retention::ACTIVITY_AGE) as i64, 42],
                    |r| r.get::<_, String>(0),
                )
                .unwrap()
                .collect::<rusqlite::Result<Vec<_>>>()
                .unwrap();
            assert_eq!(rows.len(), 1);
            assert_eq!(q.get_status(Sort), 0);
            assert_eq!(q.get_status(AutoIndex), 0);
            assert!(q.get_status(VmStep) < 1000);
            assert!(q.get_status(FullscanStep) <= 42);
            let mut q = s.conn.prepare(retention::EXPIRED_CONTENT).unwrap();
            assert_eq!(
                q.execute(params![
                    now().saturating_sub(retention::CONTENT_AGE) as i64,
                    127
                ])
                .unwrap(),
                0
            );
            assert_eq!(q.get_status(Sort), 0);
            assert_eq!(q.get_status(AutoIndex), 0);
            assert!(q.get_status(VmStep) < 1000);
            assert_eq!(q.get_status(FullscanStep), 0);
        }
        window::metrics();
        let mut passes = Vec::new();
        for pass in 0..3 {
            publication::start_counts();
            s.maintain_retention_at(now()).unwrap();
            assert!(publication::take_counts().iter().all(|n| *n <= 128));
            let metrics = window::metrics();
            assert_eq!(metrics.len(), if pass == 0 { 10 } else { 5 });
            println!("candidates={count} pass={pass} metrics={metrics:?}");
            passes.push(metrics.clone());
            for m in metrics {
                assert!(m.rows <= 128);
                assert!(m.vm < 50_000, "{m:?}");
                assert!(m.scans <= 254, "{m:?}");
                assert_eq!(m.sorts, 0, "{m:?}");
                assert_eq!(m.autoindexes, 0, "{m:?}");
            }
        }
        assert_eq!(
            passes[1], passes[2],
            "keyset seeks must not rescan a growing prefix"
        );
        if let Some(ref previous) = reference {
            assert_eq!(
                &passes, previous,
                "work counters must not grow with retained cardinality"
            );
        } else {
            reference = Some(passes);
        }
        for class in CLASSES {
            for after in [false, true] {
                let sql = format!("EXPLAIN QUERY PLAN {}", class.sql(after));
                let mut q = s.conn.prepare(&sql).unwrap();
                if let Some(i) = q.parameter_index(":ceiling").unwrap() {
                    q.raw_bind_parameter(i, "[4070908800,\"zzz\",\"zzz\"]")
                        .unwrap();
                }
                if let Some(i) = q.parameter_index(":after").unwrap() {
                    q.raw_bind_parameter(i, "[1,\"keep-000100\",\"current\"]")
                        .unwrap();
                }
                if let Some(i) = q.parameter_index(":cutoff").unwrap() {
                    q.raw_bind_parameter(i, now() as i64).unwrap();
                }
                let mut rows = q.raw_query();
                while let Some(r) = rows.next().unwrap() {
                    let detail: String = r.get(3).unwrap();
                    assert!(
                        !detail.contains("TEMP B-TREE") && !detail.contains("AUTOMATIC"),
                        "{class:?}: {detail}"
                    );
                }
            }
        }
        assert_eq!(
            n(&s.conn, "SELECT COUNT(*) FROM watch_source"),
            count as i64
        );
        assert_eq!(
            n(&s.conn, "SELECT COUNT(*) FROM seen WHERE seen=0"),
            count as i64 + 1
        );
    }
}
#[test]
fn cursor_fault_rolls_back_companion_deletes_and_restart_resumes_exact_tail() {
    for class in ["orphans", "checks", "validators", "resources", "states"] {
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("db");
        let mut s = fixture(&path, 1024);
        tails(&mut s);
        let original = history(&s.conn);
        for _ in 0..8 {
            s.maintain_retention_at(now()).unwrap();
            bounded();
        }
        let before = cursors(&s.conn);
        let revision = s.revision().unwrap();
        assert_eq!(remaining(&s.conn), [1; 5]);
        s.conn.execute_batch(&format!("CREATE TRIGGER fail_cursor BEFORE DELETE ON retention_cursor WHEN OLD.class='{class}' BEGIN SELECT RAISE(ABORT,'late cursor checkpoint'); END;")).unwrap();
        assert!(s.maintain_retention_at(now()).is_err());
        assert_eq!(remaining(&s.conn), [1; 5]);
        assert_eq!(cursors(&s.conn), before);
        assert_eq!(s.revision().unwrap(), revision);
        assert_eq!(history(&s.conn), original);
        drop(s);
        let mut cached = Store::open_cached(&path).unwrap();
        assert_eq!(
            cached
                .get::<serde_json::Value>("safe")
                .unwrap()
                .data
                .unwrap()["cached"],
            "retained"
        );
        assert_eq!(history(&cached.conn), original);
        drop(cached);
        let c = Connection::open(&path).unwrap();
        assert_eq!(cursors(&c), before);
        c.execute_batch("DROP TRIGGER fail_cursor").unwrap();
        drop(c);
        let mut s = Store::open(&path).unwrap();
        s.maintain_retention_at(now()).unwrap();
        bounded();
        assert_eq!(remaining(&s.conn), [0; 5]);
        assert_eq!(history(&s.conn), original);
        assert_eq!(s.revision().unwrap(), revision + 1);
        s.maintain_retention_at(now()).unwrap();
        assert_eq!(s.revision().unwrap(), revision + 1);
    }
}
#[test]
fn cursors_wrap_and_revisit_newly_expired_rows_behind_the_frontier() {
    let d = tempfile::tempdir().unwrap();
    let mut s = fixture(&d.path().join("db"), 256);
    s.maintain_retention_at(now()).unwrap();
    s.conn
        .execute(
            "UPDATE api_validator SET next_at=0 WHERE url='keep-000000'",
            [],
        )
        .unwrap();
    for pass in 0..4 {
        let tx = s.conn.transaction().unwrap();
        for n in 0..256 {
            tx.execute(
                "INSERT INTO api_validator(url,next_at) VALUES(?,4070908800)",
                [format!("zz-append-{pass}-{n:04}")],
            )
            .unwrap();
        }
        tx.commit().unwrap();
        s.maintain_retention_at(now()).unwrap();
        bounded();
    }
    assert_eq!(
        n(
            &s.conn,
            "SELECT COUNT(*) FROM api_validator WHERE url='keep-000000'"
        ),
        0
    );
    assert_eq!(
        n(
            &s.conn,
            "SELECT COUNT(*) FROM resource_state WHERE key='api:keep-000000'"
        ),
        0
    );
    assert_eq!(
        n(
            &s.conn,
            "SELECT COUNT(*) FROM seen WHERE source_id='keep-000000' AND seen=0"
        ),
        1
    );
}
#[test]
fn publication_application_skips_large_applied_prefix_without_sorting() {
    use rusqlite::StatementStatus::*;
    let d = tempfile::tempdir().unwrap();
    let s = Store::open(&d.path().join("db")).unwrap();
    s.conn.execute_batch("WITH RECURSIVE x(n) AS(VALUES(1) UNION ALL SELECT n+1 FROM x WHERE n<65548) INSERT INTO publication_row(tbl,k,applied) SELECT 'seen',json_array(printf('K%06d',n)),n<=65536 FROM x;").unwrap();
    let mut q = s.conn.prepare(&publication::application_sql()).unwrap();
    let keys = q
        .query_map([], |r| r.get::<_, i64>(0))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    assert_eq!(keys, (65537..=65548).collect::<Vec<_>>());
    assert_eq!(q.get_status(Sort), 0);
    assert_eq!(q.get_status(FullscanStep), 11);
    assert_eq!(q.get_status(AutoIndex), 0);
    assert!(q.get_status(VmStep) < 1000);
}
#[test]
fn crash_child_entry() {
    let Some(path) = std::env::var_os("GITJUKE_TEST_QUERY_DB") else {
        return;
    };
    let mut s = Store {
        conn: Connection::open(path).unwrap(),
        cache_writes: true,
    };
    s.maintain_retention_at(now()).unwrap();
    panic!("missed query barrier");
}
#[test]
fn sigkill_cursor_and_dependent_tail_cleanup_commit_together() {
    use std::{
        os::unix::process::ExitStatusExt,
        process::{Child, Command, Stdio},
        time::{Duration, Instant},
    };
    struct Owned(Child);
    impl Drop for Owned {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    for journal in ["DELETE", "WAL"] {
        for after in [false, true] {
            let d = tempfile::tempdir().unwrap();
            let path = d.path().join("db");
            let marker = d.path().join("marker");
            let mut s = fixture(&path, 1024);
            tails(&mut s);
            for _ in 0..8 {
                s.maintain_retention_at(now()).unwrap();
                bounded();
            }
            let original = history(&s.conn);
            let before = cursors(&s.conn);
            let revision = s.revision().unwrap();
            s.conn.pragma_update(None, "journal_mode", journal).unwrap();
            drop(s);
            let mut child = Owned(
                Command::new(std::env::current_exe().unwrap())
                    .args([
                        "--exact",
                        "query_work_tests::crash_child_entry",
                        "--nocapture",
                    ])
                    .env("GITJUKE_TEST_QUERY_DB", &path)
                    .env(
                        "GITJUKE_TEST_MAINTENANCE_PHASE",
                        if after {
                            "retention_after"
                        } else {
                            "retention_before"
                        },
                    )
                    .env("GITJUKE_TEST_MAINTENANCE_MARKER", &marker)
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
            assert_eq!(child.0.wait().unwrap().signal(), Some(libc::SIGKILL));
            let c = Connection::open(&path).unwrap();
            assert_eq!(
                c.query_row("PRAGMA quick_check(1)", [], |r| r.get::<_, String>(0))
                    .unwrap(),
                "ok"
            );
            assert_eq!(remaining(&c), [i64::from(!after); 5]);
            assert_eq!(history(&c), original);
            if !after {
                assert_eq!(cursors(&c), before);
            }
            assert_eq!(
                n(
                    &c,
                    "SELECT CAST(value AS INTEGER) FROM meta WHERE key='revision'"
                ),
                revision as i64 + i64::from(after)
            );
            drop(c);
            let mut cached = Store::open_cached(&path).unwrap();
            assert!(
                cached
                    .get::<serde_json::Value>("safe")
                    .unwrap()
                    .data
                    .is_some()
            );
            assert_eq!(history(&cached.conn), original);
            drop(cached);
            let mut s = Store::open(&path).unwrap();
            s.maintain_retention_at(now()).unwrap();
            bounded();
            assert_eq!(remaining(&s.conn), [0; 5]);
            assert_eq!(history(&s.conn), original);
            assert_eq!(s.revision().unwrap(), revision + 1);
        }
    }
}
