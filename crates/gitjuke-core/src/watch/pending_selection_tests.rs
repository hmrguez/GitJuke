use super::*;
use crate::model::{Budget, Identity};
use rusqlite::{Connection, OptionalExtension, params, types::Value};
use std::path::Path;

fn oracle(s: &Store, time: u64) -> crate::Result<Vec<PendingRow>> {
    if !s.budget_eligible("core", time)? {
        return Ok(vec![]);
    }
    let resources = ["core", "search", "graphql"]
        .into_iter()
        .filter(|r| s.budget_eligible(r, time).unwrap_or(false))
        .map(|r| format!("'{r}'"))
        .collect::<Vec<_>>()
        .join(",");
    let filter = format!(
        "at<={time} AND resource IN ({resources}) {}",
        if s.settings()?.paused {
            "AND manual=1"
        } else {
            ""
        }
    );
    let row = |r: &rusqlite::Row<'_>| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?));
    let mut out:Vec<PendingRow>=s.conn.prepare(&format!("SELECT seq,key,job,manual,foreground FROM pending WHERE {filter} ORDER BY foreground DESC,seq LIMIT 128"))?.query_map([],row)?.collect::<rusqlite::Result<_>>()?;
    if out.len() == 128 && out.iter().all(|r| r.4) {
        let bg=s.conn.query_row(&format!("SELECT seq,key,job,manual,foreground FROM pending WHERE {filter} AND foreground=0 ORDER BY seq LIMIT 1"),[],row).optional()?;
        if let Some(bg) = bg {
            out.pop();
            out.push(bg);
        }
    }
    out.sort_by_key(|r| r.0);
    Ok(out)
}
fn state(c: &Connection, tables: &[&str]) -> Vec<Vec<Vec<Value>>> {
    tables
        .iter()
        .map(|table| {
            let mut q = c
                .prepare(&format!("SELECT * FROM main.{table} ORDER BY rowid"))
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
    "pending",
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
fn block(s: &Store, resource: &str, until: u64) {
    s.save_budget(&Budget {
        resource: resource.into(),
        remaining: 100,
        reset_at: 0,
        blocked_until: until,
        requests: 0,
        payload_bytes: 0,
        cost: 0,
    })
    .unwrap();
}
fn add(s: &Store, key: &str, at: i64, resource: &str, manual: i64, foreground: i64) {
    s.conn
        .execute(
            "INSERT INTO pending(key,job,at,resource,manual,foreground) VALUES(?,?2,?3,?4,?5,?6)",
            params![
                key,
                serde_json::json!({"Overview":["a/b",at.max(0)]}).to_string(),
                at,
                resource,
                manual,
                foreground
            ],
        )
        .unwrap();
}
fn small(path: &Path) -> Store {
    let s = seed(path);
    let tx = s.conn.unchecked_transaction().unwrap();
    for n in 0..257 {
        add(&s, &format!("persist:{n:03}"), 0, "core", 1, 1);
    }
    add(&s, "persist:background", 0, "core", 1, 0);
    tx.commit().unwrap();
    s
}
fn bounded() -> (usize, i32) {
    let metrics = crate::publication_completion::metrics();
    assert!(!metrics.is_empty());
    let mut max = 0;
    for m in &metrics {
        assert!(m.rows <= 128 && m.scans <= 128 && m.vm < 5000, "{m:?}");
        assert_eq!(m.sorts, 0, "{m:?}");
        assert_eq!(m.autoindexes, 0, "{m:?}");
        max = max.max(m.vm);
    }
    (metrics.len(), max)
}
#[test]
fn candidate_windows_skip_large_rejected_prefixes_and_match_full_sort_fairness_oracle() {
    let mut previous = None;
    for size in [1025, 8193] {
        let d = tempfile::tempdir().unwrap();
        let s = seed(&d.path().join("db"));
        block(&s, "graphql", 1000);
        let tx = s.conn.unchecked_transaction().unwrap();
        for n in 0..size {
            for kind in 0..4 {
                add(
                    &s,
                    &format!("reject:{kind}:{n}"),
                    if kind == 0 { 1000 } else { 0 },
                    if kind == 1 {
                        "graphql"
                    } else if kind == 3 {
                        "unknown"
                    } else {
                        "core"
                    },
                    if kind == 2 { 0 } else { 1 },
                    1,
                );
            }
        }
        // A rejected body with a non-text SQLite representation proves that
        // descriptor decoding does not happen during candidate traversal.
        tx.execute(
            "UPDATE pending SET job=zeroblob(32) WHERE resource='unknown'",
            [],
        )
        .unwrap();
        add(&s, "oldest-background", 0, "core", 1, 0);
        for n in 0..385 {
            add(&s, &format!("eligible:{n:03}"), 100, "core", 1, 1);
        }
        add(&s, "later-background", 0, "core", 1, 0);
        tx.commit().unwrap();
        let history = state(&s.conn, HISTORY);
        let inputs = state(&s.conn, INPUTS);
        let before = s.conn.total_changes();
        crate::publication_completion::capture();
        let selected = s.pending(100).unwrap();
        let (statements, max_vm) = bounded();
        assert_eq!(selected, oracle(&s, 100).unwrap());
        assert_eq!(selected.len(), 128);
        assert_eq!(selected[0].1, "oldest-background");
        assert_eq!(selected.last().unwrap().1, "eligible:126");
        assert_eq!(selected.iter().filter(|r| !r.4).count(), 1);
        assert!(selected.windows(2).all(|w| w[0].0 < w[1].0));
        assert_eq!(s.conn.total_changes(), before);
        assert_eq!(state(&s.conn, HISTORY), history);
        assert_eq!(state(&s.conn, INPUTS), inputs);
        if let Some(vm) = previous {
            assert_eq!(max_vm, vm);
        }
        previous = Some(max_vm);
        println!("protected-group={size} statements={statements} max-vm={max_vm}");
    }
}
#[test]
fn exact_deadlines_pause_resources_integer_priority_and_signed_sequences_match_oracle() {
    let d = tempfile::tempdir().unwrap();
    let mut s = seed(&d.path().join("db"));
    let tx = s.conn.unchecked_transaction().unwrap();
    for (seq, at, manual, foreground, resource) in [
        (i64::MIN, i64::MIN, 1, i64::MIN, "core"),
        (-1, 100, 0, 1, "core"),
        (0, 100, 1, 0, "core"),
        (1, 101, 1, 1, "core"),
        (2, 100, 2, 2, "search"),
        (3, 100, 1, i64::MAX, "graphql"),
        (4, 100, 1, 1, "Core"),
        (i64::MAX, i64::MAX, 1, 1, "core"),
    ] {
        tx.execute("INSERT INTO pending(seq,key,job,at,manual,foreground,resource) VALUES(?,?2,'{}',?3,?4,?5,?6)",params![seq,format!("seq:{seq}"),at,manual,foreground,resource]).unwrap();
    }
    tx.commit().unwrap();
    for paused in [true, false] {
        let mut settings = s.settings().unwrap();
        settings.paused = paused;
        s.save_settings(&settings).unwrap();
        for time in [0, 99, 100, 101, i64::MAX as u64, u64::MAX] {
            assert_eq!(s.pending(time).unwrap(), oracle(&s, time).unwrap());
        }
    }
    block(&s, "search", 101);
    s.set_due("backoff:graphql", 101).unwrap();
    assert_eq!(s.pending(100).unwrap(), oracle(&s, 100).unwrap());
    assert!(
        !s.pending(100)
            .unwrap()
            .iter()
            .any(|r| ["seq:2", "seq:3"].contains(&r.1.as_str()))
    );
    assert!(s.pending(101).unwrap().iter().any(|r| r.1 == "seq:3"));
    block(&s, "core", 102);
    assert!(s.pending(101).unwrap().is_empty());
    assert_eq!(s.pending(102).unwrap(), oracle(&s, 102).unwrap());
    s.conn
        .execute(
            "UPDATE progress SET value='invalid' WHERE key='budget:search'",
            [],
        )
        .unwrap();
    assert_eq!(s.pending(102).unwrap(), oracle(&s, 102).unwrap());
    s.conn
        .execute(
            "UPDATE progress SET value='invalid' WHERE key='budget:core'",
            [],
        )
        .unwrap();
    assert!(s.pending(102).is_err());
    assert!(s.conn.is_autocommit());
}
#[test]
fn fairness_page_boundaries_retry_coalescing_and_ack_preserve_exact_remainder() {
    for foregrounds in [0, 126, 127, 128, 140] {
        let d = tempfile::tempdir().unwrap();
        let mut s = seed(&d.path().join("db"));
        add(&s, "older-background", 999, "core", 1, 0);
        for n in 0..foregrounds {
            add(&s, &format!("fg:{n}"), 0, "core", 1, 1);
        }
        add(&s, "background", 0, "core", 1, 0);
        add(&s, "next-background", 0, "core", 1, 0);
        let selected = s.pending(1).unwrap();
        assert_eq!(selected, oracle(&s, 1).unwrap());
        assert!(selected.iter().any(|r| r.1 == "background"));
        for row in &selected {
            s.ack(row.0).unwrap();
        }
        assert_eq!(s.pending(1).unwrap(), oracle(&s, 1).unwrap());
        if foregrounds == 140 {
            assert_eq!(s.pending(1).unwrap().len(), 14);
        }
        let seq = s
            .conn
            .query_row(
                "SELECT seq FROM pending WHERE key='older-background'",
                [],
                |r| r.get::<_, i64>(0),
            )
            .unwrap();
        s.retry_task(seq, 0).unwrap();
        assert!(s.pending(1).unwrap().iter().any(|r| r.0 == seq));
        s.enqueue(
            "older-background",
            &serde_json::json!({"Overview":["a/b",1]}),
            true,
            true,
            200,
            false,
        )
        .unwrap();
        let row = s
            .pending(1)
            .unwrap()
            .into_iter()
            .find(|r| r.0 == seq)
            .unwrap();
        assert!(row.3 && row.4);
        assert_eq!(s.pending(1).unwrap(), oracle(&s, 1).unwrap());
    }
}
#[test]
fn policy_candidates_background_reservation_and_bodies_share_one_snapshot() {
    for caller in [false, true] {
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("db");
        let s = small(&path);
        let expected = oracle(&s, 10).unwrap();
        let mut settings = s.settings().unwrap();
        settings.paused = false;
        let settings = serde_json::to_string(&settings).unwrap();
        let writer = Connection::open(&path).unwrap();
        if caller {
            s.conn.execute_batch("BEGIN").unwrap();
        }
        ON_WINDOW.with(|hook| *hook.borrow_mut()=Some(Box::new(move |_,phase,later|{
            if phase=="candidates"&&!later {
                writer.execute("UPDATE settings SET json=? WHERE id=1",[&settings]).unwrap();
                writer.execute_batch("UPDATE pending SET at=999,job='changed' WHERE key='persist:background'; UPDATE pending SET job='new-body',foreground=0 WHERE key='persist:256'; INSERT INTO due VALUES('backoff:core',1000);").unwrap();
            }Ok(())
        })));
        assert_eq!(s.pending(10).unwrap(), expected);
        ON_WINDOW.with(|h| h.borrow_mut().take());
        assert_eq!(s.conn.is_autocommit(), !caller);
        if caller {
            s.conn.execute_batch("COMMIT").unwrap();
        }
        assert!(s.pending(10).unwrap().is_empty());
        assert_eq!(s.pending(1000).unwrap(), oracle(&s, 1000).unwrap());
    }
    // A writer committing after the first body lookup cannot replace later
    // descriptors that belong to the active snapshot.
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("db");
    let s = small(&path);
    let expected = oracle(&s, 10).unwrap();
    let writer = Connection::open(&path).unwrap();
    ON_WINDOW.with(|h| {
        *h.borrow_mut() = Some(Box::new(move |_, phase, later| {
            if phase == "descriptors" && !later {
                writer
                    .execute("UPDATE pending SET job='replaced-body'", [])
                    .unwrap();
            }
            Ok(())
        }))
    });
    assert_eq!(s.pending(10).unwrap(), expected);
    ON_WINDOW.with(|h| h.borrow_mut().take());
    assert_ne!(s.pending(10).unwrap(), expected);
    assert_eq!(s.pending(10).unwrap(), oracle(&s, 10).unwrap());
}
#[test]
fn late_candidate_and_descriptor_errors_return_no_partial_queue_and_retry_without_writes() {
    for phase in ["candidates", "descriptors"] {
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("db");
        let s = small(&path);
        let history = state(&s.conn, HISTORY);
        let inputs = state(&s.conn, INPUTS);
        let before = s.conn.total_changes();
        let expected = oracle(&s, 10).unwrap();
        ON_WINDOW.with(|h| {
            *h.borrow_mut() = Some(Box::new(move |_, p, later| {
                if p == phase && later {
                    return Err(Error::StorageUnavailable("late queue read failure".into()));
                }
                Ok(())
            }))
        });
        assert!(s.pending(10).is_err());
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
        assert_eq!(cached.pending(10).unwrap(), expected);
        drop(cached);
        let s = Store::open(&path).unwrap();
        assert_eq!(state(&s.conn, HISTORY), history);
        assert_eq!(s.pending(10).unwrap(), oracle(&s, 10).unwrap());
        assert_eq!(
            s.conn
                .query_row("SELECT COUNT(*) FROM pending", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            258
        );
    }
}
#[test]
fn selected_descriptor_decode_error_rolls_back_read_snapshot_and_can_retry() {
    let d = tempfile::tempdir().unwrap();
    let s = small(&d.path().join("db"));
    s.conn
        .execute(
            "UPDATE pending SET job=zeroblob(4) WHERE key='persist:100'",
            [],
        )
        .unwrap();
    let history = state(&s.conn, HISTORY);
    let inputs = state(&s.conn, INPUTS);
    let before = s.conn.total_changes();
    assert!(s.pending(10).is_err());
    assert!(s.conn.is_autocommit());
    assert_eq!(s.conn.total_changes(), before);
    assert_eq!(state(&s.conn, HISTORY), history);
    assert_eq!(state(&s.conn, INPUTS), inputs);
    s.conn
        .execute("UPDATE pending SET job='{}' WHERE key='persist:100'", [])
        .unwrap();
    assert_eq!(s.pending(10).unwrap(), oracle(&s, 10).unwrap());
}
#[test]
fn publication_barrier_blocks_queue_selection_while_readers_keep_old_or_new_images() {
    let d = tempfile::tempdir().unwrap();
    let s = seed(&d.path().join("db"));
    add(&s, "persist:one", 0, "core", 1, 1);
    for phase in ["planning", "applying", "committed"] {
        s.conn.execute("INSERT INTO publication(id,kind,args,phase) VALUES(1,'source','{\"source\":\"authored\",\"time\":1}',?)",[phase]).unwrap();
        s.conn.execute_batch("INSERT INTO publication_row(tbl,k,old_json,new_json,applied) VALUES('pending','[1]','[1,\"persist:one\",\"old\",1,1,0,\"core\"]','[1,\"persist:one\",\"new\",1,1,0,\"core\"]',1);UPDATE main.pending SET job='new' WHERE seq=1;").unwrap();
        crate::publication::protect(&s.conn).unwrap();
        assert_eq!(
            s.conn
                .query_row("SELECT job FROM pending WHERE seq=1", [], |r| r
                    .get::<_, String>(0))
                .unwrap(),
            if phase == "committed" { "new" } else { "old" }
        );
        let history = state(&s.conn, HISTORY);
        let inputs = state(&s.conn, INPUTS);
        let before = s.conn.total_changes();
        assert!(matches!(s.pending(10), Err(Error::StorageUnavailable(_))));
        assert!(s.conn.is_autocommit());
        assert_eq!(s.conn.total_changes(), before);
        assert_eq!(state(&s.conn, HISTORY), history);
        assert_eq!(state(&s.conn, INPUTS), inputs);
        crate::publication::unprotect(&s.conn).unwrap();
        s.conn
            .execute_batch("DELETE FROM publication;DELETE FROM publication_row;")
            .unwrap();
    }
    assert_eq!(s.pending(10).unwrap(), oracle(&s, 10).unwrap());
}
#[test]
fn crash_child_entry() {
    let Some(path) = std::env::var_os("GITJUKE_TEST_PENDING_DB") else {
        return;
    };
    let s = Store {
        conn: Connection::open(path).unwrap(),
        cache_writes: false,
    };
    crate::shared_storage::register(&s.conn).unwrap();
    s.pending(10).unwrap();
    panic!("missed queue read barrier");
}
#[test]
fn sigkill_late_candidate_and_descriptor_reads_preserve_queue_and_durable_history() {
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
        for phase in ["candidates", "descriptors"] {
            let d = tempfile::tempdir().unwrap();
            let path = d.path().join("db");
            let marker = d.path().join("marker");
            let s = small(&path);
            s.conn.pragma_update(None, "journal_mode", journal).unwrap();
            let history = state(&s.conn, HISTORY);
            let inputs = state(&s.conn, INPUTS);
            let expected = oracle(&s, 10).unwrap();
            drop(s);
            let mut child = Owned(
                Command::new(std::env::current_exe().unwrap())
                    .args([
                        "--exact",
                        "pending_selection::tests::crash_child_entry",
                        "--nocapture",
                    ])
                    .env("GITJUKE_TEST_PENDING_DB", &path)
                    .env("GITJUKE_TEST_PENDING_PHASE", phase)
                    .env("GITJUKE_TEST_PENDING_MARKER", &marker)
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
            assert_eq!(cached.pending(10).unwrap(), expected);
            drop(cached);
            let s = Store::open(&path).unwrap();
            assert_eq!(state(&s.conn, HISTORY), history);
            assert!(s.pending(10).unwrap().is_empty());
            assert_eq!(
                s.conn
                    .query_row(
                        "SELECT COUNT(*) FROM pending WHERE manual=0 AND foreground=0",
                        [],
                        |r| r.get::<_, i64>(0)
                    )
                    .unwrap(),
                258
            );
            s.conn
                .execute(
                    "UPDATE pending SET manual=1,foreground=1 WHERE key!='persist:background'",
                    [],
                )
                .unwrap();
            s.conn
                .execute(
                    "UPDATE pending SET manual=1 WHERE key='persist:background'",
                    [],
                )
                .unwrap();
            assert_eq!(s.pending(10).unwrap(), expected);
        }
    }
}
