use super::*;
use crate::model::Identity;
use rusqlite::{Connection, types::Value};
use std::path::Path;
type Image = (String, String, u64, u64);
fn image(v: Vec<WatchTarget>) -> Vec<Image> {
    v.into_iter()
        .map(|t| (t.id, t.repo, t.number, t.generation))
        .collect()
}
fn oracle(
    s: &Store,
    time: u64,
    limit: usize,
    selected: Option<&(String, u64)>,
    manual: bool,
) -> rusqlite::Result<Vec<Image>> {
    let (repo, number) = selected
        .map(|(r, n)| (r.as_str(), *n as i64))
        .unwrap_or(("", 0));
    s.conn.prepare("SELECT id,repo,number,epoch FROM watch_locator WHERE due<=? AND NOT EXISTS(SELECT 1 FROM pr_purge p WHERE p.repo=watch_locator.repo COLLATE NOCASE AND p.number=watch_locator.number) AND id IN (SELECT id FROM watch_source) AND id NOT IN(SELECT json_extract(j.value,'$.id') FROM pending p,json_each(p.job,'$.Heads') j WHERE json_type(j.value,'$.id')='text') AND (?=0 OR last_success IS NULL OR last_success<?) ORDER BY (repo=? AND number=?) DESC,due,id LIMIT ?")?.query_map(params![time as i64,manual,s.due("manual-boundary").unwrap() as i64,repo,number,limit as i64],|r|Ok((r.get(0)?,r.get(1)?,r.get::<_,i64>(2)? as u64,r.get::<_,i64>(3)? as u64)))?.collect()
}
fn state(c: &Connection, tables: &[&str]) -> Vec<Vec<Vec<Value>>> {
    tables
        .iter()
        .map(|t| {
            let mut q = c
                .prepare(&format!("SELECT * FROM main.{t} ORDER BY rowid"))
                .unwrap();
            let n = q.column_count();
            q.query_map([], |r| (0..n).map(|i| r.get(i)).collect())
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
    "watch_locator",
    "watch_source",
    "pending",
    "pr_purge",
    "content",
    "shared_bytes",
    "publication",
    "publication_row",
    "sqlite_sequence",
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
    s.conn.execute_batch("INSERT INTO activity VALUES('A','a/b',1,'comment',1,1,'eligible',1);INSERT INTO seen VALUES('A',0);INSERT INTO dedup VALUES('A');INSERT INTO progress VALUES('activity:baseline','1'),('catchup:A','A');INSERT INTO activity_cycle VALUES(1,'recovery',123,1);INSERT INTO activity_intent VALUES(1,'pr','attempted',1),(2,'summary','unavailable',1);INSERT INTO intent_member VALUES(1,'A'),(2,'A');INSERT INTO alert_attempt VALUES('attempt',1);INSERT INTO api_validator VALUES('notifications','etag',NULL,4070908800);INSERT INTO due VALUES('activity:notifications',4070908800),('manual-boundary',100);").unwrap();
    s.put(
        "safe",
        &serde_json::json!({"cached":true}),
        s.generation().unwrap(),
    )
    .unwrap();
    s
}
fn add(
    c: &Connection,
    id: &str,
    repo: &str,
    number: i64,
    due: i64,
    success: Option<i64>,
    source: Option<&str>,
) {
    c.execute(
        "INSERT INTO watch_locator(id,repo,number,epoch,due,last_success) VALUES(?,?,?,7,?,?)",
        params![id, repo, number, due, success],
    )
    .unwrap();
    if let Some(source) = source {
        c.execute("INSERT INTO watch_source VALUES(?,?)", params![id, source])
            .unwrap();
    }
}
fn job(c: &Connection, key: &str, body: &str) {
    c.execute(
        "INSERT INTO pending(key,job,manual,foreground,at) VALUES(?,?,1,1,4070908800)",
        params![key, body],
    )
    .unwrap();
}
fn small(path: &Path) -> Store {
    let mut s = seed(path);
    let tx = s.conn.transaction().unwrap();
    for n in 0..257 {
        add(
            &tx,
            &format!("W{n:04}"),
            "a/b",
            n,
            0,
            None,
            Some("authored"),
        );
    }
    job(&tx, "persist:reserved", r#"{"Heads":[{"id":"W0000"}]}"#);
    tx.commit().unwrap();
    s
}
fn bounded() -> (usize, i32) {
    let metrics = crate::publication_completion::metrics();
    assert!(!metrics.is_empty());
    let mut max = 0;
    for m in &metrics {
        assert!(m.rows <= 128 && m.scans <= 128 && m.vm < 10000, "{m:?}");
        assert_eq!(m.sorts, 0, "{m:?}");
        assert_eq!(m.autoindexes, 0, "{m:?}");
        max = max.max(m.vm);
    }
    (metrics.len(), max)
}
#[test]
fn candidate_and_reservation_windows_match_full_oracle_through_large_protected_prefixes() {
    let mut previous = None;
    for size in [1025, 8193] {
        let d = tempfile::tempdir().unwrap();
        let mut s = seed(&d.path().join("db"));
        let tx = s.conn.transaction().unwrap();
        for n in 0..size {
            for kind in 0..4 {
                let id = format!("P{kind}-{n:05}");
                let repo = format!("prefix/{kind}");
                add(
                    &tx,
                    &id,
                    &repo,
                    n,
                    if kind == 0 { 1000 } else { 0 },
                    if kind == 3 { Some(100) } else { None },
                    if kind == 1 { None } else { Some("authored") },
                );
                if kind == 2 {
                    tx.execute("INSERT INTO pr_purge VALUES(?,?,1,0)", params![repo, n])
                        .unwrap();
                }
            }
            job(&tx, &format!("persist:{n:05}"), r#"{"Overview":["a/b",1]}"#);
        }
        for n in 0..385 {
            add(
                &tx,
                &format!("W{n:04}"),
                "a/b",
                n,
                (n * 71) % 101,
                None,
                Some("direct"),
            );
        }
        let mut body = String::from("{\"Heads\":[");
        for n in 0..size {
            if n != 0 {
                body.push(',');
            }
            body.push_str(&format!("{{\"id\":\"unknown-{n:05}\"}}"));
        }
        for n in 0..257 {
            body.push_str(&format!(",{{\"id\":\"W{n:04}\"}}"));
        }
        body.push_str("]}");
        job(&tx, "persist:large-heads", &body);
        tx.commit().unwrap();
        let selected = ("a/b".into(), 380);
        let history = state(&s.conn, HISTORY);
        let inputs = state(&s.conn, INPUTS);
        let before = s.conn.total_changes();
        crate::publication_completion::capture();
        let out = image(s.due_watches(100, 128, Some(&selected), true).unwrap());
        let (statements, max_vm) = bounded();
        assert_eq!(out, oracle(&s, 100, 128, Some(&selected), true).unwrap());
        assert_eq!(out.len(), 128);
        assert_eq!(out[0].0, "W0380");
        assert_eq!(s.conn.total_changes(), before);
        assert_eq!(state(&s.conn, HISTORY), history);
        assert_eq!(state(&s.conn, INPUTS), inputs);
        if let Some(vm) = previous {
            assert_eq!(max_vm, vm);
        }
        previous = Some(max_vm);
        println!("protected-group={size} statements={statements} max-vm={max_vm}");
        for limit in [0, 1, 10, 129, 500] {
            assert_eq!(
                image(s.due_watches(100, limit, Some(&selected), true).unwrap()),
                oracle(&s, 100, limit, Some(&selected), true).unwrap()
            );
        }
    }
}
#[test]
fn reservation_entries_preserve_sqlite_types_duplicate_keys_and_exact_identity() {
    let d = tempfile::tempdir().unwrap();
    let s = seed(&d.path().join("db"));
    for id in ["A", "a", "B", "C", "é", "nul\0id", ""] {
        add(&s.conn, id, "a/b", 1, 0, None, Some("selected"));
    }
    for body in [
        r#"{"Heads":[{"id":"A"},{"id":null},{},{"id":99},{"id":"é"},{"id":"nul\u0000id"}]}"#,
        r#"{"Heads":{"one":{"id":"B"},"two":{"id":false}}}"#,
        r#"{"Heads":["{\"id\":\"C\"}",null,0,false]}"#,
        r#"{"Heads":null,"Heads":[{"id":"A"}]}"#,
        r#"{"Heads":[{"id":null,"id":"A"},{"id":"a","id":"B"}]}"#,
        r#"{"heads":[{"id":"A"}],"Overview":["x",1]}"#,
        r#"{"He\u0061ds":[{"id":"A"}]}"#,
        "{Heads:[{id:'A'}], /* job */ ignored:Infinity}",
        r#"{"Heads":12}"#,
        "null",
        "[]",
        r#""text""#,
    ] {
        s.conn.execute("DELETE FROM pending", []).unwrap();
        job(&s.conn, "persist:types", body);
        assert_eq!(
            image(s.due_watches(100, 20, None, false).unwrap()),
            oracle(&s, 100, 20, None, false).unwrap(),
            "{body}"
        );
    }
    // A rejected non-JSON text entry errors in the preceding SQLite relation
    // too; never turn that malformed reservation into an empty successful set.
    s.conn
        .execute("UPDATE pending SET job=?", [r#"{"Heads":["not-json"]}"#])
        .unwrap();
    assert!(oracle(&s, 100, 20, None, false).is_err());
    assert!(s.due_watches(100, 20, None, false).is_err());
    assert!(s.conn.is_autocommit());
    s.conn
        .execute("UPDATE pending SET job='invalid-json'", [])
        .unwrap();
    let inputs = state(&s.conn, INPUTS);
    assert!(matches!(
        s.due_watches(100, 20, None, false),
        Err(Error::Storage(_))
    ));
    assert_eq!(state(&s.conn, INPUTS), inputs);
    // Late malformed input can be repaired without deleting membership/history.
    s.conn.execute("UPDATE pending SET job='{}'", []).unwrap();
    assert_eq!(
        image(s.due_watches(100, 20, None, false).unwrap()),
        oracle(&s, 100, 20, None, false).unwrap()
    );
}
#[test]
fn exact_selected_due_id_order_manual_boundary_case_insensitive_denial_and_limits() {
    let d = tempfile::tempdir().unwrap();
    let s = seed(&d.path().join("db"));
    for (id, repo, n, due, success, source) in [
        ("", "", 0, 10, None, Some("selected")),
        ("A", "a/b", 1, 0, None, Some("authored")),
        ("a", "A/b", 2, 0, Some(99), Some("direct")),
        ("é", "a/b", 3, 0, Some(100), Some("team:x")),
        ("Z", "a/b", 4, 101, None, Some("authored")),
        ("missing", "a/b", 5, 0, None, None),
        ("denied", "Owner/Repo", 6, 0, None, Some("authored")),
        (
            "min",
            "a/b",
            i64::MIN,
            i64::MIN,
            Some(i64::MIN),
            Some("direct"),
        ),
        (
            "max",
            "a/b",
            i64::MAX,
            i64::MAX,
            Some(i64::MAX),
            Some("direct"),
        ),
    ] {
        add(&s.conn, id, repo, n, due, success, source);
    }
    s.conn.execute_batch("INSERT INTO watch_source VALUES('A','selected'),('A','direct'); INSERT INTO pr_purge VALUES('owner/repo',6,1,0);").unwrap();
    for selected in [
        None,
        Some(("a/b".into(), 3)),
        Some(("A/b".into(), 2)),
        Some(("".into(), 0)),
    ] {
        for manual in [false, true] {
            for time in [0, 10, 100, 101, u64::MAX] {
                for limit in [0, 1, 10, 128, 129] {
                    assert_eq!(
                        image(
                            s.due_watches(time, limit, selected.as_ref(), manual)
                                .unwrap()
                        ),
                        oracle(&s, time, limit, selected.as_ref(), manual).unwrap()
                    );
                }
            }
        }
    }
    let out = image(s.due_watches(100, 10, None, false).unwrap());
    assert_eq!(out[0].0, "");
    assert!(!out.iter().any(|r| r.0 == "denied"));
}
#[test]
fn repeated_disjoint_reservation_and_success_rotation_preserve_exact_due_prefix() {
    let d = tempfile::tempdir().unwrap();
    let s = small(&d.path().join("db"));
    let history = state(&s.conn, HISTORY);
    assert_eq!(s.due_watches(100, 129, None, false).unwrap().len(), 129);
    let mut all = std::collections::HashSet::new();
    for batch in 0..26 {
        let selected = ("a/b".into(), 256);
        let targets = s.due_watches(100, 10, Some(&selected), false).unwrap();
        assert_eq!(
            image(targets.clone()),
            oracle(&s, 100, 10, Some(&selected), false).unwrap()
        );
        for t in &targets {
            assert!(all.insert(t.id.clone()));
        }
        if targets.is_empty() {
            break;
        }
        job(
            &s.conn,
            &format!("persist:batch-{batch}"),
            &serde_json::json!({"Heads":targets}).to_string(),
        );
    }
    assert_eq!(all.len(), 256);
    assert!(s.due_watches(100, 10, None, false).unwrap().is_empty());
    assert_eq!(state(&s.conn, HISTORY), history);
    s.conn
        .execute("DELETE FROM pending WHERE key!='persist:reserved'", [])
        .unwrap();
    let first = s.due_watches(100, 10, None, false).unwrap();
    for t in &first {
        s.conn
            .execute(
                "UPDATE watch_locator SET due=101,last_success=100 WHERE id=?",
                [&t.id],
            )
            .unwrap();
    }
    let next = s.due_watches(100, 10, None, false).unwrap();
    assert!(next.iter().all(|t| !first.iter().any(|f| f.id == t.id)));
    assert_eq!(image(next), oracle(&s, 100, 10, None, false).unwrap());
}
#[test]
fn windows_membership_denials_boundary_and_reservations_share_one_snapshot() {
    for caller in [false, true] {
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("db");
        let s = small(&path);
        let expected = oracle(&s, 100, 128, None, true).unwrap();
        let writer = Connection::open(&path).unwrap();
        if caller {
            s.conn.execute_batch("BEGIN").unwrap();
        }
        ON_WINDOW.with(|h|*h.borrow_mut()=Some(Box::new(move |_,phase,later|{if phase=="watches"&&!later{writer.execute_batch("BEGIN;UPDATE watch_locator SET due=999 WHERE id>='W0200';DELETE FROM watch_source WHERE id='W0001';INSERT INTO pr_purge VALUES('a/b',2,1,0);UPDATE due SET at=0 WHERE key='manual-boundary';UPDATE pending SET job='{\"Heads\":[{\"id\":\"W0003\"}]}';COMMIT;").unwrap();}Ok(())})));
        assert_eq!(
            image(s.due_watches(100, 128, None, true).unwrap()),
            expected
        );
        ON_WINDOW.with(|h| h.borrow_mut().take());
        assert_eq!(s.conn.is_autocommit(), !caller);
        if caller {
            s.conn.execute_batch("COMMIT").unwrap();
        }
        let next = oracle(&s, 100, 128, None, true).unwrap();
        assert_ne!(next, expected);
        assert_eq!(image(s.due_watches(100, 128, None, true).unwrap()), next);
    }
}
#[test]
fn late_watch_pending_and_reservation_failures_discard_partial_selection_then_retry() {
    for phase in ["watches", "pending", "reservations"] {
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("db");
        let s = small(&path);
        let tx = s.conn.unchecked_transaction().unwrap();
        for n in 0..129 {
            job(&tx, &format!("persist:{n}"), "{}");
        }
        tx.commit().unwrap();
        let history = state(&s.conn, HISTORY);
        let inputs = state(&s.conn, INPUTS);
        let before = s.conn.total_changes();
        let expected = oracle(&s, 100, 128, None, false).unwrap();
        ON_WINDOW.with(|h| {
            *h.borrow_mut() = Some(Box::new(move |_, p, later| {
                if p == phase && later {
                    return Err(Error::StorageUnavailable(
                        "late watch scheduling read failure".into(),
                    ));
                }
                Ok(())
            }))
        });
        assert!(s.due_watches(100, 128, None, false).is_err());
        ON_WINDOW.with(|h| h.borrow_mut().take());
        assert!(s.conn.is_autocommit());
        assert_eq!(s.conn.total_changes(), before);
        assert_eq!(state(&s.conn, HISTORY), history);
        assert_eq!(state(&s.conn, INPUTS), inputs);
        drop(s);
        let mut cached = Store::open_cached(&path).unwrap();
        assert!(
            cached
                .get::<serde_json::Value>("safe")
                .unwrap()
                .data
                .is_some()
        );
        assert_eq!(
            image(cached.due_watches(100, 128, None, false).unwrap()),
            expected
        );
        drop(cached);
        let s = Store::open(&path).unwrap();
        assert_eq!(state(&s.conn, HISTORY), history);
        assert_eq!(
            image(s.due_watches(100, 128, None, false).unwrap()),
            expected
        );
    }
}
#[test]
fn late_json_entry_failure_keeps_original_input_and_retry_preserves_reserved_prefix() {
    let d = tempfile::tempdir().unwrap();
    let s = small(&d.path().join("db"));
    let mut entries = vec![serde_json::json!({"id":"W0000"})];
    for n in 0..257 {
        entries.push(serde_json::json!({"id":format!("unknown-{n}")}));
    }
    entries.push(serde_json::json!("not-json"));
    s.conn
        .execute(
            "UPDATE pending SET job=?",
            [serde_json::json!({"Heads":entries}).to_string()],
        )
        .unwrap();
    let history = state(&s.conn, HISTORY);
    let inputs = state(&s.conn, INPUTS);
    let before = s.conn.total_changes();
    assert!(oracle(&s, 100, 129, None, false).is_err());
    assert!(matches!(
        s.due_watches(100, 129, None, false),
        Err(Error::StorageUnavailable(_))
    ));
    assert!(s.conn.is_autocommit());
    assert_eq!(s.conn.total_changes(), before);
    assert_eq!(state(&s.conn, HISTORY), history);
    assert_eq!(state(&s.conn, INPUTS), inputs);
    *entries.last_mut().unwrap() = serde_json::json!({"id":"W0128"});
    s.conn
        .execute(
            "UPDATE pending SET job=?",
            [serde_json::json!({"Heads":entries}).to_string()],
        )
        .unwrap();
    let out = image(s.due_watches(100, 129, None, false).unwrap());
    assert_eq!(out, oracle(&s, 100, 129, None, false).unwrap());
    assert_eq!(out.len(), 129);
    assert!(!out.iter().any(|r| r.0 == "W0000" || r.0 == "W0128"));
    assert_eq!(state(&s.conn, HISTORY), history);
}
#[test]
fn pending_publication_blocks_native_selection_behind_old_and_new_watch_views() {
    let d = tempfile::tempdir().unwrap();
    let s = small(&d.path().join("db"));
    for phase in ["planning", "applying", "committed"] {
        s.conn.execute("INSERT INTO publication(id,kind,args,phase) VALUES(1,'source','{\"source\":\"authored\",\"time\":1}',?)",[phase]).unwrap();
        s.conn.execute_batch("INSERT INTO publication_row(tbl,k,old_json,new_json,applied) VALUES('watch_source','[\"W0001\",\"authored\"]','[\"W0001\",\"authored\"]',NULL,1);DELETE FROM main.watch_source WHERE id='W0001';").unwrap();
        crate::publication::protect(&s.conn).unwrap();
        let visible = s
            .conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM watch_source WHERE id='W0001')",
                [],
                |r| r.get::<_, bool>(0),
            )
            .unwrap();
        assert_eq!(visible, phase != "committed");
        let history = state(&s.conn, HISTORY);
        let inputs = state(&s.conn, INPUTS);
        let before = s.conn.total_changes();
        assert!(matches!(
            s.due_watches(100, 10, None, false),
            Err(Error::StorageUnavailable(_))
        ));
        assert_eq!(s.conn.total_changes(), before);
        assert_eq!(state(&s.conn, HISTORY), history);
        assert_eq!(state(&s.conn, INPUTS), inputs);
        crate::publication::unprotect(&s.conn).unwrap();
        s.conn.execute_batch("DELETE FROM publication;DELETE FROM publication_row;INSERT INTO watch_source VALUES('W0001','authored');").unwrap();
    }
    assert_eq!(
        image(s.due_watches(100, 10, None, false).unwrap()),
        oracle(&s, 100, 10, None, false).unwrap()
    );
}
#[test]
fn crash_child_entry() {
    let Some(path) = std::env::var_os("GITJUKE_TEST_WATCH_DB") else {
        return;
    };
    let s = Store {
        conn: Connection::open(path).unwrap(),
        cache_writes: false,
    };
    crate::shared_storage::register(&s.conn).unwrap();
    s.due_watches(100, 128, None, false).unwrap();
    panic!("missed watch read barrier");
}
#[test]
fn sigkill_late_watch_and_reservation_reads_preserve_queue_accounting_and_history() {
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
        for phase in ["watches", "reservations"] {
            let d = tempfile::tempdir().unwrap();
            let path = d.path().join("db");
            let marker = d.path().join("marker");
            let s = small(&path);
            s.conn.pragma_update(None, "journal_mode", journal).unwrap();
            let history = state(&s.conn, HISTORY);
            let inputs = state(&s.conn, INPUTS);
            let expected = oracle(&s, 100, 128, None, false).unwrap();
            drop(s);
            let mut child = Owned(
                Command::new(std::env::current_exe().unwrap())
                    .args([
                        "--exact",
                        "watch_scheduling::tests::crash_child_entry",
                        "--nocapture",
                    ])
                    .env("GITJUKE_TEST_WATCH_DB", &path)
                    .env("GITJUKE_TEST_WATCH_PHASE", phase)
                    .env("GITJUKE_TEST_WATCH_MARKER", &marker)
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
            drop(c);
            let mut cached = Store::open_cached(&path).unwrap();
            assert!(
                cached
                    .get::<serde_json::Value>("safe")
                    .unwrap()
                    .data
                    .is_some()
            );
            assert_eq!(
                image(cached.due_watches(100, 128, None, false).unwrap()),
                expected
            );
            drop(cached);
            let s = Store::open(&path).unwrap();
            assert_eq!(state(&s.conn, HISTORY), history);
            assert_eq!(
                image(s.due_watches(100, 128, None, false).unwrap()),
                expected
            );
        }
    }
}
