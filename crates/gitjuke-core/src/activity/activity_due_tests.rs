use super::*;
use crate::{model::Identity, store::Store};
use rusqlite::types::Value;
use std::path::Path;

const ORACLE: &str = "SELECT repo,number FROM activity_target t WHERE due<=? AND NOT EXISTS(SELECT 1 FROM pending p WHERE p.key='activity:pr:'||t.repo||':'||t.number) AND (done=0 OR EXISTS(SELECT 1 FROM watch_locator l JOIN watch_source s ON s.id=l.id WHERE l.repo=t.repo AND l.number=t.number AND s.source!='selected')) ORDER BY due,repo,number LIMIT 128";
fn oracle(c: &Connection, time: u64) -> Vec<(String, u64)> {
    c.prepare(ORACLE)
        .unwrap()
        .query_map([time as i64], |r| {
            Ok((r.get(0)?, r.get::<_, i64>(1)? as u64))
        })
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap()
}
fn state(c: &Connection, tables: &[&str]) -> Vec<Vec<Vec<Value>>> {
    tables
        .iter()
        .map(|table| {
            let mut q = c
                .prepare(&format!("SELECT * FROM main.{table} ORDER BY rowid"))
                .unwrap();
            let columns = q.column_count();
            q.query_map([], |r| (0..columns).map(|i| r.get(i)).collect())
                .unwrap()
                .collect::<rusqlite::Result<_>>()
                .unwrap()
        })
        .collect()
}
const HISTORY: &[&str] = &[
    "identity",
    "settings",
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
    "meta",
];
const INPUTS: &[&str] = &[
    "activity_target",
    "watch_locator",
    "watch_source",
    "pending",
    "content",
    "shared_bytes",
    "publication",
    "publication_row",
];
fn seed(path: &Path) -> Store {
    let mut s = Store::open(path).unwrap();
    s.bind(&Identity {
        id: 42,
        login: "reader".into(),
    })
    .unwrap();
    let mut settings = s.settings().unwrap();
    settings.paused = true;
    s.save_settings(&settings).unwrap();
    s.conn.execute_batch("INSERT INTO activity VALUES('A','a/b',1,'comment',1,1,'eligible',1);
        INSERT INTO seen VALUES('A',0);INSERT INTO dedup VALUES('A');
        INSERT INTO progress VALUES('activity:baseline','1'),('catchup:A','A');
        INSERT INTO activity_cycle VALUES(1,'recovery',123,1);
        INSERT INTO activity_intent VALUES(1,'pr','attempted',1),(2,'summary','unavailable',1);
        INSERT INTO intent_member VALUES(1,'A'),(2,'A');INSERT INTO alert_attempt VALUES('attempt',1);
        INSERT INTO api_validator VALUES('notifications','etag',NULL,4070908800);
        INSERT INTO due VALUES('activity:notifications',4070908800);").unwrap();
    s.put(
        "safe",
        &serde_json::json!({"cached":true}),
        s.generation().unwrap(),
    )
    .unwrap();
    s
}
fn bounded() -> (usize, i32) {
    let metrics = crate::publication_completion::metrics();
    assert!(!metrics.is_empty());
    let mut maximum = 0;
    for m in &metrics {
        assert!(m.rows <= 128 && m.scans <= 128 && m.vm < 10000, "{m:?}");
        assert_eq!(m.sorts, 0, "{m:?}");
        assert_eq!(m.autoindexes, 0, "{m:?}");
        maximum = maximum.max(m.vm);
    }
    (metrics.len(), maximum)
}
#[test]
fn exact_oracle_and_bounded_statements_across_protected_targets_aliases_and_reasons() {
    let mut previous = None;
    for size in [1025, 8193] {
        let d = tempfile::tempdir().unwrap();
        let mut s = seed(&d.path().join("db"));
        let tx = s.conn.transaction().unwrap();
        for n in (0..size).rev() {
            for kind in 0..4 {
                let repo = format!("{}/prefix", kind + 1);
                tx.execute(
                    "INSERT INTO activity_target VALUES(?,?,?,?)",
                    params![
                        repo,
                        n,
                        if kind == 0 { 1000 } else { -10 },
                        if kind == 0 { 0 } else { 1 }
                    ],
                )
                .unwrap();
                if kind == 1 {
                    tx.execute(
                        "INSERT INTO pending(key,job,manual,foreground,at) VALUES(?,'{}',0,0,0)",
                        [format!("activity:pr:{repo}:{n}")],
                    )
                    .unwrap();
                }
                if kind == 2 {
                    let id = format!("S{n:05}");
                    tx.execute(
                        "INSERT INTO watch_locator(id,repo,number) VALUES(?,?,?)",
                        params![id, repo, n],
                    )
                    .unwrap();
                    tx.execute("INSERT INTO watch_source VALUES(?,'selected')", [id])
                        .unwrap();
                }
            }
            for repo in ["0/aliases", "0/rejected"] {
                let id = format!("{repo}-{n:05}");
                tx.execute(
                    "INSERT INTO watch_locator(id,repo,number) VALUES(?,?,1)",
                    params![id, repo],
                )
                .unwrap();
                if n % 2 == 0 {
                    tx.execute("INSERT INTO watch_source VALUES(?,'selected')", [id])
                        .unwrap();
                }
            }
            tx.execute(
                "INSERT INTO watch_source VALUES('zz-alias',?)",
                [format!("team:{n:05}")],
            )
            .unwrap();
        }
        tx.execute_batch(
            "INSERT INTO activity_target VALUES('0/aliases',1,-20,1),('0/rejected',1,-20,1);
            INSERT INTO watch_locator(id,repo,number) VALUES('zz-alias','0/aliases',1);",
        )
        .unwrap();
        for n in (0..385).rev() {
            tx.execute(
                "INSERT INTO activity_target VALUES('z/eligible',?,?,0)",
                params![n, (n * 71) % 101],
            )
            .unwrap();
        }
        // Collation, signed numeric endpoints, due equality and unfinished
        // notification-only targets all match the original SQL semantics.
        for (repo, number, due, done) in [
            ("", 0, i64::MIN, 0),
            ("A/b", i64::MIN, 0, 0),
            ("a/b", i64::MAX, 0, 0),
            ("é/repo", 1, 100, 0),
            ("z/eligible", 400, 101, 0),
        ] {
            tx.execute(
                "INSERT INTO activity_target VALUES(?,?,?,?)",
                params![repo, number, due, done],
            )
            .unwrap();
        }
        tx.execute_batch(
            "INSERT INTO activity_target VALUES('case/source',1,0,1);
            INSERT INTO watch_locator(id,repo,number) VALUES('case','case/source',1);
            INSERT INTO watch_source VALUES('case','Selected');",
        )
        .unwrap();
        tx.commit().unwrap();
        let original = state(&s.conn, HISTORY);
        let inputs = state(&s.conn, INPUTS);
        let before = s.conn.total_changes();
        crate::publication_completion::capture();
        assert_eq!(s.activity_due(100).unwrap(), oracle(&s.conn, 100));
        let (statements, max_vm) = bounded();
        assert_eq!(s.conn.total_changes(), before);
        assert_eq!(state(&s.conn, HISTORY), original);
        assert_eq!(state(&s.conn, INPUTS), inputs);
        if let Some(vm) = previous {
            assert_eq!(max_vm, vm);
        }
        previous = Some(max_vm);
        println!("protected-group={size} statements={statements} max-vm={max_vm}");
        for time in [0, 101, u64::MAX] {
            assert_eq!(s.activity_due(time).unwrap(), oracle(&s.conn, time));
        }
    }
}
fn small_fixture(path: &Path) -> Store {
    let mut s = seed(path);
    let tx = s.conn.transaction().unwrap();
    for n in 0..385 {
        tx.execute(
            "INSERT INTO activity_target VALUES('a/b',?,?,0)",
            params![n, 385 - n],
        )
        .unwrap();
    }
    tx.commit().unwrap();
    s
}
#[test]
fn all_windows_and_point_checks_share_one_snapshot_including_caller_transactions() {
    for caller_transaction in [false, true] {
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("db");
        let s = small_fixture(&path);
        let expected = oracle(&s.conn, 500);
        let writer = Connection::open(&path).unwrap();
        if caller_transaction {
            s.conn.execute_batch("BEGIN").unwrap();
        }
        ON_WINDOW.with(|hook| *hook.borrow_mut()=Some(Box::new(move |_,relation,later| {
            if relation=="targets" && !later {
                writer.execute_batch("BEGIN;UPDATE activity_target SET due=999 WHERE number>=257;
                    INSERT INTO pending(key,job,manual,foreground,at) VALUES('activity:pr:a/b:200','{}',0,0,0);
                    UPDATE activity_target SET done=1 WHERE number=199;COMMIT;").unwrap();
            }
            Ok(())
        })));
        assert_eq!(s.activity_due(500).unwrap(), expected);
        ON_WINDOW.with(|hook| hook.borrow_mut().take());
        assert_eq!(s.conn.is_autocommit(), !caller_transaction);
        if caller_transaction {
            s.conn.execute_batch("COMMIT").unwrap();
        }
        let next = oracle(&s.conn, 500);
        assert_ne!(next, expected);
        assert_eq!(s.activity_due(500).unwrap(), next);
    }
}
#[test]
fn late_read_failure_discards_partial_heap_and_retry_preserves_inputs_and_history() {
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("db");
    let s = small_fixture(&path);
    let original = state(&s.conn, HISTORY);
    let inputs = state(&s.conn, INPUTS);
    let before = s.conn.total_changes();
    ON_WINDOW.with(|hook| {
        *hook.borrow_mut() = Some(Box::new(|_, relation, later| {
            if relation == "targets" && later {
                return Err(Error::StorageUnavailable(
                    "late eligibility read failure".into(),
                ));
            }
            Ok(())
        }))
    });
    assert!(s.activity_due(500).is_err());
    assert!(s.conn.is_autocommit());
    ON_WINDOW.with(|hook| hook.borrow_mut().take());
    assert_eq!(s.conn.total_changes(), before);
    assert_eq!(state(&s.conn, HISTORY), original);
    assert_eq!(state(&s.conn, INPUTS), inputs);
    drop(s);
    let mut cached = Store::open_cached(&path).unwrap();
    assert_eq!(
        cached.get::<serde_json::Value>("safe").unwrap().data,
        Some(serde_json::json!({"cached":true}))
    );
    assert_eq!(cached.activity_due(500).unwrap(), oracle(&cached.conn, 500));
    drop(cached);
    let s = Store::open(&path).unwrap();
    assert_eq!(state(&s.conn, HISTORY), original);
    assert_eq!(s.activity_due(500).unwrap(), oracle(&s.conn, 500));
}
#[test]
fn pending_publication_blocks_native_queries_behind_protected_reader_views() {
    let d = tempfile::tempdir().unwrap();
    let s = small_fixture(&d.path().join("db"));
    for phase in ["planning", "applying", "committed"] {
        s.conn.execute("INSERT INTO publication(id,kind,args,phase) VALUES(1,'source','{\"source\":\"authored\",\"time\":1}',?)",[phase]).unwrap();
        s.conn.execute_batch("INSERT INTO publication_row(tbl,k,old_json,new_json,applied) VALUES('activity_target','[\"a/b\",0]','[\"a/b\",0,385,0]','[\"a/b\",0,0,0]',1);
            UPDATE main.activity_target SET due=0 WHERE repo='a/b' AND number=0;").unwrap();
        crate::publication::protect(&s.conn).unwrap();
        let visible_due: i64 = s
            .conn
            .query_row(
                "SELECT due FROM activity_target WHERE repo='a/b' AND number=0",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(visible_due, if phase == "committed" { 0 } else { 385 });
        let original = state(&s.conn, HISTORY);
        let inputs = state(&s.conn, INPUTS);
        let before = s.conn.total_changes();
        assert!(matches!(
            s.activity_due(500),
            Err(Error::StorageUnavailable(_))
        ));
        assert!(s.conn.is_autocommit());
        assert_eq!(s.conn.total_changes(), before);
        assert_eq!(state(&s.conn, HISTORY), original);
        assert_eq!(state(&s.conn, INPUTS), inputs);
        crate::publication::unprotect(&s.conn).unwrap();
        s.conn.execute("DELETE FROM publication", []).unwrap();
        s.conn.execute_batch("DELETE FROM publication_row; UPDATE activity_target SET due=385 WHERE repo='a/b' AND number=0;").unwrap();
    }
    assert_eq!(s.activity_due(500).unwrap(), oracle(&s.conn, 500));
}
#[test]
fn alias_windows_propagate_late_errors_and_preserve_membership_snapshot() {
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("db");
    let mut s = seed(&path);
    let tx = s.conn.transaction().unwrap();
    tx.execute_batch("INSERT INTO activity_target VALUES('0/notification',1,1,0),('a/b',1,0,1);")
        .unwrap();
    for n in 0..257 {
        let id = format!("A{n:03}");
        tx.execute(
            "INSERT INTO watch_locator(id,repo,number) VALUES(?,'a/b',1)",
            [&id],
        )
        .unwrap();
        tx.execute(
            "INSERT INTO watch_source VALUES(?,?)",
            params![id, if n == 256 { "direct" } else { "selected" }],
        )
        .unwrap();
    }
    tx.commit().unwrap();
    let history = state(&s.conn, HISTORY);
    let inputs = state(&s.conn, INPUTS);
    ON_WINDOW.with(|hook| {
        *hook.borrow_mut() = Some(Box::new(|_, relation, later| {
            if relation == "locators" && later {
                return Err(Error::StorageUnavailable("late alias read failure".into()));
            }
            Ok(())
        }))
    });
    assert!(s.activity_due(10).is_err());
    assert!(s.conn.is_autocommit());
    ON_WINDOW.with(|hook| hook.borrow_mut().take());
    assert_eq!(state(&s.conn, HISTORY), history);
    assert_eq!(state(&s.conn, INPUTS), inputs);
    let expected = oracle(&s.conn, 10);
    assert_eq!(expected.len(), 2);
    let writer = Connection::open(&path).unwrap();
    ON_WINDOW.with(|hook| {
        *hook.borrow_mut() = Some(Box::new(move |_, relation, later| {
            if relation == "locators" && !later {
                writer
                    .execute("DELETE FROM watch_source WHERE id='A256'", [])
                    .unwrap();
            }
            Ok(())
        }))
    });
    assert_eq!(s.activity_due(10).unwrap(), expected);
    ON_WINDOW.with(|hook| hook.borrow_mut().take());
    let next = oracle(&s.conn, 10);
    assert_eq!(next, vec![("0/notification".into(), 1)]);
    assert_eq!(s.activity_due(10).unwrap(), next);
}
#[test]
fn exact_deadline_pending_key_source_case_and_native_tie_order() {
    let d = tempfile::tempdir().unwrap();
    let s = seed(&d.path().join("db"));
    s.conn.execute_batch("INSERT INTO activity_target VALUES('a/b',1,10,0),('a/b',2,11,0),('A/b',1,10,1),('é/b',1,10,0),('a/b',-1,10,0),('a/b',3,10,0),('unwatched',1,10,1);
        INSERT INTO watch_locator(id,repo,number) VALUES('case','A/b',1),('selected','unwatched',1);
        INSERT INTO watch_source VALUES('case','Selected'),('selected','selected');
        INSERT INTO pending(key,job,manual,foreground,at) VALUES('activity:pr:a/b:1','{}',0,0,999),('activity:pr:a/b:30','{}',0,0,0);").unwrap();
    assert_eq!(s.activity_due(9).unwrap(), vec![]);
    assert_eq!(
        s.activity_due(10).unwrap(),
        vec![
            ("A/b".into(), 1),
            ("a/b".into(), u64::MAX),
            ("a/b".into(), 3),
            ("é/b".into(), 1)
        ]
    );
    assert_eq!(s.activity_due(11).unwrap(), oracle(&s.conn, 11));
}
#[test]
fn crash_child_entry() {
    let Some(path) = std::env::var_os("GITJUKE_TEST_HYDRATION_DB") else {
        return;
    };
    let c = Connection::open(path).unwrap();
    crate::shared_storage::register(&c).unwrap();
    select(&c, 500).unwrap();
    panic!("missed hydration read barrier");
}
#[test]
fn sigkill_late_read_has_no_persistent_selection_or_completion_prefix() {
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
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("db");
        let marker = d.path().join("marker");
        let s = small_fixture(&path);
        s.conn.pragma_update(None, "journal_mode", journal).unwrap();
        let history = state(&s.conn, HISTORY);
        let inputs = state(&s.conn, INPUTS);
        let expected = oracle(&s.conn, 500);
        drop(s);
        let mut child = Owned(
            Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "activity_due::tests::crash_child_entry",
                    "--nocapture",
                ])
                .env("GITJUKE_TEST_HYDRATION_DB", &path)
                .env("GITJUKE_TEST_HYDRATION_RELATION", "targets")
                .env("GITJUKE_TEST_HYDRATION_MARKER", &marker)
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
        assert_eq!(state(&c, HISTORY), history);
        assert_eq!(state(&c, INPUTS), inputs);
        assert_eq!(select(&c, 500).unwrap(), expected);
        drop(c);
        let mut cached = Store::open_cached(&path).unwrap();
        assert!(
            cached
                .get::<serde_json::Value>("safe")
                .unwrap()
                .data
                .is_some()
        );
        assert_eq!(cached.activity_due(500).unwrap(), expected);
        drop(cached);
        let s = Store::open(&path).unwrap();
        assert_eq!(state(&s.conn, HISTORY), history);
        assert_eq!(s.activity_due(500).unwrap(), expected);
    }
}
