use crate::{control_sweep, now, publication, retention, store::Store};
use rusqlite::{Connection, params};
use serde_json::json;
use std::path::Path;
fn n(c: &Connection, sql: &str) -> i64 {
    c.query_row(sql, [], |r| r.get(0)).unwrap()
}
fn fixture(path: &Path, kind: &str) -> Store {
    let mut s = Store::open(path).unwrap();
    s.bind(&crate::model::Identity {
        id: 42,
        login: "reader".into(),
    })
    .unwrap();
    let mut settings = s.settings().unwrap();
    settings.paused = true;
    s.save_settings(&settings).unwrap();
    retention::tests::seed(&mut s, if kind == "retention" { 257 } else { 1 }, now());
    s.conn.execute_batch("INSERT INTO activity_cycle VALUES(1,'recovery',123,0); INSERT INTO due VALUES('activity:notifications',4070908800); INSERT INTO api_validator VALUES('deadline','etag',NULL,4070908800);").unwrap();
    let tx = s.conn.transaction().unwrap();
    for i in 0..257 {
        tx.execute(
            "INSERT INTO pending(key,job,manual,foreground,at) VALUES(?,?,0,0,0)",
            params![
                format!("heads-{i:04}"),
                json!({"Heads":[{"id":format!("P{i:04}")}]}).to_string()
            ],
        )
        .unwrap();
        tx.execute(
            "INSERT INTO activity_stage VALUES('a/b',1,?,'{}')",
            [format!("S{i:04}")],
        )
        .unwrap();
        tx.execute(
            "INSERT INTO watch_locator(id,repo,number,due) VALUES(?,'a/b',?,999)",
            params![format!("P{i:04}"), i],
        )
        .unwrap();
        tx.execute(
            "INSERT INTO watch_source VALUES(?,'direct')",
            [format!("P{i:04}")],
        )
        .unwrap();
        if kind == "retention" {
            tx.execute(
                "INSERT INTO api_validator VALUES(?,'old',NULL,0)",
                [format!("old-{i:04}")],
            )
            .unwrap();
            tx.execute(
                "INSERT INTO resource_state VALUES(?,1,1,NULL)",
                [format!("api:old-{i:04}")],
            )
            .unwrap();
            tx.execute(
                "INSERT INTO check_state VALUES('obsolete',?,1,1,NULL,1)",
                [format!("sha-{i:04}")],
            )
            .unwrap();
            tx.execute(
                "INSERT INTO check_context VALUES('obsolete',?,'ctx','{}',0)",
                [format!("sha-{i:04}")],
            )
            .unwrap();
        }
    }
    tx.execute_batch("INSERT INTO pending(key,job,manual,foreground,at) VALUES('unrelated','{}',0,0,0); INSERT INTO activity_stage VALUES('a_/b_',1,'other','{}'); INSERT INTO watch_locator(id,repo,number,due) VALUES('unwatched','a/b',999,999);").unwrap();
    tx.commit().unwrap();
    s
}
fn remaining(c: &Connection, kind: &str) -> i64 {
    n(
        c,
        match kind {
            "heads" => "SELECT COUNT(*) FROM pending WHERE key GLOB 'heads-*'",
            "activity" => "SELECT COUNT(*) FROM activity_stage WHERE repo='a/b'",
            "manual" => "SELECT COUNT(*) FROM watch_locator WHERE id!='unwatched' AND due!=0",
            _ => "SELECT COUNT(*) FROM activity",
        },
    )
}
fn history(c: &Connection) -> Vec<Vec<Vec<rusqlite::types::Value>>> {
    [
        "seen",
        "dedup",
        "intent_member",
        "activity_intent",
        "alert_attempt",
        "identity",
        "settings",
        "due",
        "activity_cycle",
        "watch_source",
        "progress WHERE key NOT GLOB 'catchup:*'",
        "api_validator WHERE url='deadline'",
    ]
    .iter()
    .map(|t| {
        let mut q = c
            .prepare(&format!("SELECT * FROM {t} ORDER BY rowid"))
            .unwrap();
        let cols = q.column_count();
        q.query_map([], |r| (0..cols).map(|i| r.get(i)).collect())
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap()
    })
    .collect()
}
fn run(s: &mut Store, kind: &str) {
    if kind == "retention" {
        s.maintain_retention_at(now()).unwrap();
    } else {
        control_sweep::run(&mut s.conn, kind, json!({"repo":"a/b","number":1})).unwrap();
    }
}
#[test]
fn mixed_retention_shares_one_allowance_and_preserves_dependent_units_and_history() {
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("db");
    let mut s = fixture(&path, "retention");
    let original = history(&s.conn);
    let mut passes = 0;
    loop {
        publication::start_counts();
        s.maintain_retention_at(now()).unwrap();
        let counts = publication::take_counts();
        assert_eq!(counts.len(), 1);
        assert!(counts[0] <= 128, "{counts:?}");
        assert_eq!(history(&s.conn), original);
        assert_eq!(
            n(
                &s.conn,
                "SELECT COUNT(*) FROM progress WHERE key GLOB 'catchup:*'"
            ),
            remaining(&s.conn, "retention")
        );
        assert_eq!(
            n(&s.conn, "SELECT COUNT(*) FROM content"),
            remaining(&s.conn, "retention")
        );
        assert_eq!(
            n(
                &s.conn,
                "SELECT COUNT(*) FROM check_state s WHERE s.complete=1 AND NOT EXISTS(SELECT 1 FROM check_context d WHERE d.pr_id=s.pr_id AND d.commit_sha=s.commit_sha)"
            ),
            0
        );
        passes += 1;
        assert!(passes < 40);
        if counts[0] == 0 {
            break;
        }
        if passes == 1 {
            assert_eq!(remaining(&s.conn, "retention"), 215);
            assert_eq!(counts[0], 127);
        }
        drop(s);
        s = Store::open(&path).unwrap();
    }
    assert_eq!(remaining(&s.conn, "retention"), 0);
    assert_eq!(n(&s.conn, "SELECT COUNT(*) FROM api_validator"), 1);
    assert_eq!(n(&s.conn, "SELECT COUNT(*) FROM check_state"), 0);
    assert_eq!(n(&s.conn, "SELECT COUNT(*) FROM resource_state"), 0);
    assert_eq!(n(&s.conn, "SELECT COUNT(*) FROM check_context"), 0);
}
#[test]
fn late_sweep_fault_keeps_exact_prefix_scope_and_restart_blocks_recovery() {
    for kind in ["heads", "activity", "manual"] {
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("db");
        let mut s = fixture(&path, kind);
        let original = history(&s.conn);
        let (table, action, pred) = match kind {
            "heads" => ("pending", "DELETE", "OLD.key='heads-0200'"),
            "activity" => ("activity_stage", "DELETE", "OLD.id='S0200'"),
            _ => ("watch_locator", "UPDATE", "OLD.id='P0200'"),
        };
        s.conn.execute_batch(&format!("CREATE TRIGGER fail_sweep BEFORE {action} ON {table} WHEN {pred} BEGIN SELECT RAISE(ABORT,'late sweep'); END;")).unwrap();
        publication::start_counts();
        assert!(control_sweep::run(&mut s.conn, kind, json!({"repo":"a/b","number":1})).is_err());
        assert_eq!(publication::take_counts(), vec![1, 127]);
        assert_eq!(remaining(&s.conn, kind), 130);
        assert!(control_sweep::pending(&s.conn).unwrap());
        assert_eq!(history(&s.conn), original);
        drop(s);
        assert!(Store::open(&path).is_err());
        let mut cached = Store::open_cached(&path).unwrap();
        assert!(cached.activity_page(0).unwrap().items.is_empty());
        assert_eq!(history(&cached.conn), original);
        drop(cached);
        let c = Connection::open(&path).unwrap();
        assert_eq!(remaining(&c, kind), 130);
        c.execute_batch("DROP TRIGGER fail_sweep").unwrap();
        drop(c);
        let mut s = Store::open(&path).unwrap();
        assert!(!control_sweep::pending(&s.conn).unwrap());
        assert_eq!(remaining(&s.conn, kind), 0);
        assert_eq!(history(&s.conn), original);
        run(&mut s, kind);
        assert_eq!(remaining(&s.conn, kind), 0);
        assert_eq!(history(&s.conn), original);
        assert_eq!(
            n(
                &s.conn,
                "SELECT COUNT(*) FROM pending WHERE key='unrelated'"
            ),
            1
        );
        assert_eq!(
            n(
                &s.conn,
                "SELECT due FROM watch_locator WHERE id='unwatched'"
            ),
            999
        );
    }
}
#[test]
fn late_retention_fault_rolls_back_only_failing_unit_and_retries_without_seen_changes() {
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("db");
    let mut s = fixture(&path, "retention");
    let original = history(&s.conn);
    s.maintain_retention_at(now()).unwrap();
    let revision = s.revision().unwrap();
    s.conn.execute_batch("CREATE TRIGGER fail_late BEFORE DELETE ON activity WHEN OLD.id='old-0080' BEGIN SELECT RAISE(ABORT,'late retention'); END;").unwrap();
    assert!(s.maintain_retention_at(now()).is_err());
    assert_eq!(remaining(&s.conn, "retention"), 215);
    assert_eq!(s.revision().unwrap(), revision);
    assert_eq!(
        n(
            &s.conn,
            "SELECT COUNT(*) FROM content WHERE key='activity-payload:old-0080'"
        ),
        1
    );
    assert_eq!(
        n(
            &s.conn,
            "SELECT COUNT(*) FROM progress WHERE key='catchup:old-0080'"
        ),
        1
    );
    s.activity_seen(&["old-0080".into()]).unwrap();
    assert_eq!(history(&s.conn), original);
    drop(s);
    let c = Connection::open(&path).unwrap();
    c.execute_batch("DROP TRIGGER fail_late").unwrap();
    drop(c);
    let mut s = Store::open(&path).unwrap();
    s.maintain_retention_at(now()).unwrap();
    assert_eq!(remaining(&s.conn, "retention"), 173);
    assert_eq!(history(&s.conn), original);
}
#[test]
fn crash_child_entry() {
    let Some(path) = std::env::var_os("GITJUKE_TEST_MAINTENANCE_DB") else {
        return;
    };
    let mut s = Store {
        conn: Connection::open(path).unwrap(),
        cache_writes: true,
    };
    let kind = std::env::var("GITJUKE_TEST_MAINTENANCE_KIND").unwrap();
    for _ in 0..3 {
        run(&mut s, &kind);
    }
    panic!("child missed barrier");
}
#[test]
fn sigkill_maintenance_before_and_after_first_and_late_commits() {
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
    for kind in ["retention", "heads", "activity", "manual"] {
        for hit in [1, 2, 3] {
            for after in [false, true] {
                let d = tempfile::tempdir().unwrap();
                let path = d.path().join("db");
                let marker = d.path().join("marker");
                let s = fixture(&path, kind);
                let original = history(&s.conn);
                let revision = s.revision().unwrap();
                drop(s);
                let phase = format!(
                    "{}_{}",
                    if kind == "retention" {
                        "retention"
                    } else {
                        "sweep"
                    },
                    if after { "after" } else { "before" }
                );
                let mut child = Owned(
                    Command::new(std::env::current_exe().unwrap())
                        .args([
                            "--exact",
                            "maintenance_tests::crash_child_entry",
                            "--nocapture",
                        ])
                        .env("GITJUKE_TEST_MAINTENANCE_DB", &path)
                        .env("GITJUKE_TEST_MAINTENANCE_KIND", kind)
                        .env("GITJUKE_TEST_MAINTENANCE_PHASE", phase)
                        .env("GITJUKE_TEST_MAINTENANCE_HIT", hit.to_string())
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
                let committed = hit - 1 + usize::from(after);
                assert_eq!(
                    remaining(&c, kind),
                    if kind == "retention" {
                        257 - (committed * 42) as i64
                    } else {
                        (257 - (committed * 127) as i64).max(0)
                    }
                );
                assert_eq!(history(&c), original);
                assert_eq!(
                    n(
                        &c,
                        "SELECT CAST(value AS INTEGER) FROM meta WHERE key='revision'"
                    ),
                    revision as i64
                        + if kind == "retention" {
                            committed as i64
                        } else {
                            0
                        }
                );
                if kind != "retention" {
                    assert_eq!(control_sweep::pending(&c).unwrap(), committed < 3);
                }
                let mut cached = Store::open_cached(&path).unwrap();
                assert!(cached.activity_page(0).unwrap().items.is_empty());
                assert_eq!(history(&cached.conn), original);
                drop(cached);
                drop(c);
                let mut s = Store::open(&path).unwrap();
                for _ in 0..30 {
                    run(&mut s, kind);
                }
                assert_eq!(remaining(&s.conn, kind), 0);
                assert_eq!(history(&s.conn), original);
                assert!(!control_sweep::pending(&s.conn).unwrap());
            }
        }
    }
}
#[test]
fn cached_expired_checks_do_not_gain_completeness_while_cleanup_waits() {
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("db");
    let s = fixture(&path, "retention");
    s.conn.execute_batch("UPDATE watch_locator SET head='sha-0000' WHERE id='P0000'; UPDATE check_state SET pr_id='P0000' WHERE commit_sha='sha-0000'; UPDATE check_context SET pr_id='P0000' WHERE commit_sha='sha-0000';").unwrap();
    drop(s);
    let s = Store::open_cached(&path).unwrap();
    let p = s.check_page("a/b", 0, 0).unwrap();
    assert!(p.items.is_empty());
    assert!(p.commits.iter().all(|c| !c.complete));
}

#[test]
fn every_control_transaction_includes_final_marker_and_preserves_unrelated_scope() {
    for kind in ["heads", "activity", "manual"] {
        let d = tempfile::tempdir().unwrap();
        let mut s = fixture(&d.path().join("db"), kind);
        let original = history(&s.conn);
        publication::start_counts();
        run(&mut s, kind);
        assert_eq!(publication::take_counts(), vec![1, 127, 127, 4]);
        assert_eq!(history(&s.conn), original);
        assert_eq!(remaining(&s.conn, kind), 0);
        assert_eq!(
            n(
                &s.conn,
                "SELECT COUNT(*) FROM activity_stage WHERE repo='a_/b_'"
            ),
            1
        );
        assert_eq!(
            n(
                &s.conn,
                "SELECT COUNT(*) FROM pending WHERE key='unrelated'"
            ),
            1
        );
        assert_eq!(
            n(
                &s.conn,
                "SELECT due FROM watch_locator WHERE id='unwatched'"
            ),
            999
        );
    }
}
#[test]
fn pending_control_work_cannot_falsely_complete_ready_recovery() {
    let d = tempfile::tempdir().unwrap();
    let mut s = Store::open(&d.path().join("db")).unwrap();
    s.conn.execute_batch("INSERT INTO activity_cycle VALUES(1,'recovery',123,1); INSERT INTO resource_state VALUES('authored',1,1,NULL),('direct',1,1,NULL),('teams',1,1,NULL); INSERT INTO control_sweep VALUES(1,'heads','{}');").unwrap();
    let revision = s.revision().unwrap();
    s.activity_finish_recovery(false).unwrap();
    assert_eq!(s.revision().unwrap(), revision);
    assert_eq!(
        n(
            &s.conn,
            "SELECT boundary FROM activity_cycle WHERE mode='recovery'"
        ),
        123
    );
    assert_eq!(s.activity_progress("activity:baseline").unwrap(), 0);
    control_sweep::resume(&mut s.conn).unwrap();
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
fn late_metadata_and_check_faults_restore_complete_batch_after_committed_prefix() {
    fn snapshot(c: &Connection) -> Vec<Vec<Vec<rusqlite::types::Value>>> {
        [
            "activity",
            "content",
            "progress",
            "check_context",
            "check_state",
            "api_validator",
            "resource_state",
            "meta",
        ]
        .iter()
        .map(|t| {
            let mut q = c
                .prepare(&format!("SELECT * FROM {t} ORDER BY rowid"))
                .unwrap();
            let cols = q.column_count();
            q.query_map([], |r| (0..cols).map(|i| r.get(i)).collect())
                .unwrap()
                .collect::<rusqlite::Result<Vec<_>>>()
                .unwrap()
        })
        .collect()
    }
    for (table, pred) in [
        ("api_validator", "OLD.url='old-0200'"),
        ("resource_state", "OLD.key='api:old-0200'"),
        ("check_context", "OLD.commit_sha='sha-0200'"),
        ("check_state", "OLD.commit_sha='sha-0200'"),
    ] {
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("db");
        let mut s = fixture(&path, "retention");
        let original = history(&s.conn);
        s.conn.execute_batch(&format!("CREATE TRIGGER fail_metadata_late BEFORE DELETE ON {table} WHEN {pred} BEGIN SELECT RAISE(ABORT,'late metadata/check'); END;")).unwrap();
        let mut committed = 0;
        loop {
            let previous = snapshot(&s.conn);
            if s.maintain_retention_at(now()).is_err() {
                assert_eq!(snapshot(&s.conn), previous);
                break;
            }
            committed += 1;
            assert!(committed < 40);
        }
        assert!(committed > 1);
        assert_eq!(history(&s.conn), original);
        drop(s);
        let c = Connection::open(&path).unwrap();
        c.execute_batch("DROP TRIGGER fail_metadata_late").unwrap();
        drop(c);
        let mut s = Store::open(&path).unwrap();
        for _ in 0..30 {
            s.maintain_retention_at(now()).unwrap();
        }
        assert_eq!(history(&s.conn), original);
        assert_eq!(n(&s.conn, "SELECT COUNT(*) FROM activity"), 0);
        assert_eq!(n(&s.conn, "SELECT COUNT(*) FROM check_state"), 0);
        assert_eq!(n(&s.conn, "SELECT COUNT(*) FROM resource_state"), 0);
        assert_eq!(n(&s.conn, "SELECT COUNT(*) FROM api_validator"), 1);
    }
}
