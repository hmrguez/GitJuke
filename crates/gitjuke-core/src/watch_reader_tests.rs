use super::*;
use crate::model::Identity;
use rusqlite::types::Value;
use std::path::Path;
fn oracle(s: &Store, offset: usize) -> Result<WatchPage> {
    let mut q=s.conn.prepare("SELECT id,repo,number,head,merge_sha,last_success,due,CASE WHEN EXISTS(SELECT 1 FROM pr_purge p WHERE p.repo=watch_locator.repo COLLATE NOCASE AND p.number=watch_locator.number) THEN 'Resource unavailable' ELSE error END FROM watch_locator WHERE id IN (SELECT id FROM watch_source) ORDER BY id LIMIT 101 OFFSET ?")?;
    let mut items = q
        .query_map([offset as i64], read_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let more = items.len() > 100;
    items.truncate(100);
    for item in &mut items {
        item.reasons = s
            .conn
            .prepare("SELECT source FROM watch_source WHERE id=? ORDER BY source")?
            .query_map([&item.id], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?;
    }
    budget(serde_json::to_vec(&items)?.len())?;
    let next_offset = if more {
        Some(offset + items.len())
    } else {
        None
    };
    Ok(WatchPage {
        items,
        next_offset,
        revision: s.revision()?,
    })
}
fn same(a: WatchPage, b: WatchPage) {
    assert_eq!(
        serde_json::to_value(a).unwrap(),
        serde_json::to_value(b).unwrap()
    );
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
    s.conn.execute_batch("INSERT INTO activity VALUES('A','a/b',1,'comment',1,1,'eligible',1);INSERT INTO seen VALUES('A',0);INSERT INTO dedup VALUES('A');INSERT INTO progress VALUES('activity:baseline','1'),('catchup:A','A');INSERT INTO activity_cycle VALUES(1,'recovery',123,1);INSERT INTO activity_intent VALUES(1,'pr','attempted',1),(2,'summary','unavailable',1);INSERT INTO intent_member VALUES(1,'A'),(2,'A');INSERT INTO alert_attempt VALUES('attempt',1);INSERT INTO api_validator VALUES('notifications','etag',NULL,4070908800);INSERT INTO due VALUES('activity:notifications',4070908800);").unwrap();
    s.put(
        "safe",
        &serde_json::json!({"cached":true}),
        s.generation().unwrap(),
    )
    .unwrap();
    s
}
fn add(c: &Connection, id: &str, number: i64, watched: bool) {
    c.execute("INSERT INTO watch_locator(id,repo,number,head,merge_sha,last_success,due,error) VALUES(?,'Owner/Repo',?,'head','merge',17,23,'old-error')",params![id,number]).unwrap();
    if watched {
        c.execute("INSERT INTO watch_source VALUES(?,'authored')", [id])
            .unwrap();
    }
}
fn small(path: &Path) -> Store {
    let mut s = seed(path);
    let tx = s.conn.transaction().unwrap();
    for n in 0..257 {
        add(&tx, &format!("W{n:04}"), n, true);
    }
    tx.commit().unwrap();
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
fn native_windows_preserve_deep_offset_membership_order_and_many_reasons() {
    let mut previous = None;
    for size in [1025, 8193] {
        let d = tempfile::tempdir().unwrap();
        let mut s = seed(&d.path().join("db"));
        let tx = s.conn.transaction().unwrap();
        for n in (0..size).rev() {
            add(&tx, &format!("A{n:05}"), n, false);
        }
        for n in (0..385).rev() {
            add(&tx, &format!("W{n:04}"), n, true);
            tx.execute(
                "INSERT INTO watch_source VALUES(?,'selected')",
                [format!("W{n:04}")],
            )
            .unwrap();
        }
        for n in (0..size).rev() {
            tx.execute(
                "INSERT INTO watch_source VALUES('W0200',?)",
                [format!("team:{n:05}")],
            )
            .unwrap();
        }
        tx.execute("INSERT INTO pr_purge VALUES('owner/repo',201,1,0)", [])
            .unwrap();
        tx.commit().unwrap();
        let history = state(&s.conn, HISTORY);
        let inputs = state(&s.conn, INPUTS);
        let before = s.conn.total_changes();
        crate::publication_completion::capture();
        let page = s.watch_page(200).unwrap();
        let (statements, max_vm) = bounded();
        same(page, oracle(&s, 200).unwrap());
        assert_eq!(s.conn.total_changes(), before);
        assert_eq!(state(&s.conn, HISTORY), history);
        assert_eq!(state(&s.conn, INPUTS), inputs);
        if let Some(vm) = previous {
            assert_eq!(max_vm, vm);
        }
        previous = Some(max_vm);
        println!("protected-prefix={size} statements={statements} max-vm={max_vm}");
        let mut ids = vec![];
        let mut offset = 0;
        loop {
            let page = s.watch_page(offset).unwrap();
            same(page.clone(), oracle(&s, offset).unwrap());
            ids.extend(page.items.iter().map(|i| i.id.clone()));
            if let Some(next) = page.next_offset {
                offset = next;
            } else {
                break;
            }
        }
        assert_eq!(
            ids,
            (0..385).map(|n| format!("W{n:04}")).collect::<Vec<_>>()
        );
        for offset in [99, 100, 284, 285, 384, 385, 10000] {
            same(s.watch_page(offset).unwrap(), oracle(&s, offset).unwrap());
        }
    }
}
fn retain(c: &Connection, table: &str, key: serde_json::Value, old: Option<serde_json::Value>) {
    c.execute(
        "INSERT INTO publication_row(tbl,k,old_json) VALUES(?,?,?)",
        params![table, key.to_string(), old.map(|v| v.to_string())],
    )
    .unwrap();
}
fn protected(s: &mut Store, prefix: usize) {
    let tx = s.conn.transaction().unwrap();
    tx.execute("INSERT INTO publication(id,kind,args,phase) VALUES(1,'source','{\"source\":\"authored\",\"time\":1}','applying')",[]).unwrap();
    for n in 0..prefix {
        retain(
            &tx,
            "seen",
            serde_json::json!([format!("keep-{n:05}")]),
            Some(serde_json::json!([format!("keep-{n:05}"), 0])),
        );
    }
    // Native IDs whose JSON-encoded key order differs from BINARY text order.
    for id in ["\n", "\"", "\\", "é", "old-only"] {
        retain(
            &tx,
            "watch_locator",
            serde_json::json!([id]),
            Some(serde_json::json!([
                id,
                "Owner/Repo",
                -1,
                null,
                "open",
                null,
                "old-head",
                "old-merge",
                0,
                8,
                7,
                "old-image-error",
                0
            ])),
        );
        retain(
            &tx,
            "watch_source",
            serde_json::json!([id, "direct"]),
            Some(serde_json::json!([id, "direct"])),
        );
    }
    retain(
        &tx,
        "watch_locator",
        serde_json::json!(["W0000"]),
        Some(serde_json::json!([
            "W0000",
            "Owner/Repo",
            0,
            null,
            "open",
            null,
            "old-head",
            null,
            0,
            10,
            9,
            null,
            0
        ])),
    );
    retain(
        &tx,
        "watch_source",
        serde_json::json!(["W0001", "authored"]),
        Some(serde_json::json!(["W0001", "authored"])),
    );
    tx.execute("DELETE FROM watch_source WHERE id='W0001'", [])
        .unwrap();
    add(&tx, "new-only", 999, true);
    retain(&tx, "watch_locator", serde_json::json!(["new-only"]), None);
    retain(
        &tx,
        "watch_source",
        serde_json::json!(["new-only", "authored"]),
        None,
    );
    tx.execute(
        "UPDATE watch_locator SET head='new-head' WHERE id='W0000'",
        [],
    )
    .unwrap();
    for n in 0..257 {
        let source = format!("A-hidden-{n:04}");
        tx.execute("INSERT INTO watch_source VALUES('W0002',?)", [&source])
            .unwrap();
        retain(
            &tx,
            "watch_source",
            serde_json::json!(["W0002", source]),
            None,
        );
    }
    retain(
        &tx,
        "watch_source",
        serde_json::json!(["W0002", "\nold-reason"]),
        Some(serde_json::json!(["W0002", "\nold-reason"])),
    );
    tx.execute("INSERT INTO pr_purge VALUES('owner/repo',-1,1,0)", [])
        .unwrap();
    tx.commit().unwrap();
    crate::publication::protect(&s.conn).unwrap();
}
#[test]
fn protected_pages_merge_deleted_images_hide_additions_and_keep_native_order() {
    let mut previous = None;
    for prefix in [1025, 8193] {
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("db");
        let mut s = small(&path);
        protected(&mut s, prefix);
        let history = state(&s.conn, HISTORY);
        let inputs = state(&s.conn, INPUTS);
        let before = s.conn.total_changes();
        crate::publication_completion::capture();
        same(s.watch_page(0).unwrap(), oracle(&s, 0).unwrap());
        let (statements, max_vm) = bounded();
        if let Some(vm) = previous {
            assert_eq!(max_vm, vm);
        }
        previous = Some(max_vm);
        println!("journal-prefix={prefix} statements={statements} max-vm={max_vm}");
        for phase in ["planning", "applying", "committed"] {
            s.conn
                .execute("UPDATE main.publication SET phase=?", [phase])
                .unwrap();
            for offset in [0, 100, 200, 261, 262, 500] {
                same(s.watch_page(offset).unwrap(), oracle(&s, offset).unwrap());
            }
        }
        s.conn
            .execute("UPDATE main.publication SET phase='applying'", [])
            .unwrap();
        assert_eq!(state(&s.conn, HISTORY), history);
        assert_eq!(state(&s.conn, INPUTS), inputs);
        assert_eq!(s.conn.total_changes(), before + 4);
        drop(s);
        let cached = Store::open_cached(&path).unwrap();
        same(cached.watch_page(0).unwrap(), oracle(&cached, 0).unwrap());
        assert_eq!(state(&cached.conn, HISTORY), history);
    }
}
#[test]
fn exact_page_boundaries_and_inclusive_byte_guard_preserve_error_and_history() {
    for count in [0, 99, 100, 101, 200, 201] {
        let d = tempfile::tempdir().unwrap();
        let mut s = seed(&d.path().join("db"));
        let tx = s.conn.transaction().unwrap();
        for n in 0..count {
            add(&tx, &format!("W{n:04}"), n, true);
        }
        tx.commit().unwrap();
        for offset in [0, 99, 100, 101, 200, 201] {
            same(s.watch_page(offset).unwrap(), oracle(&s, offset).unwrap());
        }
    }
    let d = tempfile::tempdir().unwrap();
    let s = seed(&d.path().join("db"));
    add(&s.conn, "one", 1, false);
    s.conn
        .execute("INSERT INTO watch_source VALUES('one','')", [])
        .unwrap();
    let page = s.watch_page(0).unwrap();
    let base = serde_json::to_vec(&page.items).unwrap().len();
    let source = "x".repeat(BYTES - base);
    s.conn
        .execute("UPDATE watch_source SET source=? WHERE id='one'", [&source])
        .unwrap();
    assert_eq!(
        serde_json::to_vec(&s.watch_page(0).unwrap().items)
            .unwrap()
            .len(),
        BYTES
    );
    let history = state(&s.conn, HISTORY);
    s.conn
        .execute("UPDATE watch_source SET source=source||'x'", [])
        .unwrap();
    let inputs = state(&s.conn, INPUTS);
    let before = s.conn.total_changes();
    assert!(
        matches!(s.watch_page(0),Err(Error::Safe(message)) if message=="Watch page exceeds the IPC byte budget.")
    );
    assert!(oracle(&s, 0).is_err());
    assert!(s.conn.is_autocommit());
    assert_eq!(s.conn.total_changes(), before);
    assert_eq!(state(&s.conn, HISTORY), history);
    assert_eq!(state(&s.conn, INPUTS), inputs);
}
#[test]
fn row_reason_denial_and_revision_reads_share_snapshot_including_caller_transaction() {
    for caller in [false, true] {
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("db");
        let s = small(&path);
        let expected = oracle(&s, 0).unwrap();
        let writer = Connection::open(&path).unwrap();
        if caller {
            s.conn.execute_batch("BEGIN").unwrap();
        }
        ON_WINDOW.with(|h|*h.borrow_mut()=Some(Box::new(move |_,phase,later|{if phase=="locators"&&!later{writer.execute_batch("BEGIN;UPDATE watch_locator SET head='concurrent';INSERT INTO watch_source VALUES('W0001','direct');DELETE FROM watch_source WHERE id='W0002';INSERT INTO pr_purge VALUES('owner/repo',3,1,0);UPDATE meta SET value=CAST(value AS INTEGER)+1 WHERE key='revision';COMMIT;").unwrap();}Ok(())})));
        same(s.watch_page(0).unwrap(), expected);
        ON_WINDOW.with(|h| h.borrow_mut().take());
        assert_eq!(s.conn.is_autocommit(), !caller);
        if caller {
            s.conn.execute_batch("COMMIT").unwrap();
        }
        same(s.watch_page(0).unwrap(), oracle(&s, 0).unwrap());
        assert_eq!(
            s.watch_page(0).unwrap().items[0].head.as_deref(),
            Some("concurrent")
        );
    }
}
#[test]
fn visibility_commit_during_protected_read_cannot_mix_old_rows_new_reasons_or_revision() {
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("db");
    let mut s = small(&path);
    protected(&mut s, 257);
    let expected = oracle(&s, 0).unwrap();
    let writer = Connection::open(&path).unwrap();
    ON_WINDOW.with(|h|*h.borrow_mut()=Some(Box::new(move |_,phase,later|{if phase=="locators"&&!later{writer.execute_batch("BEGIN;UPDATE publication SET phase='committed';UPDATE meta SET value=CAST(value AS INTEGER)+1 WHERE key='revision';COMMIT;").unwrap();}Ok(())})));
    same(s.watch_page(0).unwrap(), expected);
    ON_WINDOW.with(|h| h.borrow_mut().take());
    same(s.watch_page(0).unwrap(), oracle(&s, 0).unwrap());
    assert_eq!(
        s.watch_page(0).unwrap().items[0].head.as_deref(),
        Some("new-head")
    );
}
#[test]
fn late_read_errors_return_no_partial_page_keep_history_and_retry_cached() {
    for phase in ["locators", "reasons"] {
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("db");
        let s = small(&path);
        let history = state(&s.conn, HISTORY);
        let inputs = state(&s.conn, INPUTS);
        let before = s.conn.total_changes();
        ON_WINDOW.with(|h| {
            *h.borrow_mut() = Some(Box::new(move |_, p, later| {
                if p == phase && later {
                    return Err(Error::StorageUnavailable(
                        "late watch reader failure".into(),
                    ));
                }
                Ok(())
            }))
        });
        // A deep offset forces more than one candidate window.
        assert!(s.watch_page(128).is_err());
        ON_WINDOW.with(|h| h.borrow_mut().take());
        assert!(s.conn.is_autocommit());
        assert_eq!(s.conn.total_changes(), before);
        assert_eq!(state(&s.conn, HISTORY), history);
        assert_eq!(state(&s.conn, INPUTS), inputs);
        drop(s);
        let mut cached = Store::open_cached(&path).unwrap();
        same(
            cached.watch_page(128).unwrap(),
            oracle(&cached, 128).unwrap(),
        );
        assert!(
            cached
                .get::<serde_json::Value>("safe")
                .unwrap()
                .data
                .is_some()
        );
        drop(cached);
        let s = Store::open(&path).unwrap();
        same(s.watch_page(128).unwrap(), oracle(&s, 128).unwrap());
        assert_eq!(state(&s.conn, HISTORY), history);
    }
}
#[test]
fn interrupted_real_source_publication_keeps_old_pages_then_startup_exposes_new_once() {
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("db");
    let (mut s, args) = crate::publication::tests::seed(&path, "source");
    let old: Vec<_> = [0, 100, 200]
        .into_iter()
        .map(|offset| oracle(&s, offset).unwrap())
        .collect();
    let revision = s.revision().unwrap();
    let durable = state(&s.conn, &HISTORY[..HISTORY.len() - 1]);
    s.conn.execute_batch("CREATE TRIGGER fail_reader_publication BEFORE UPDATE OF applied ON publication_row WHEN NEW.applied=1 AND NEW.seq>200 BEGIN SELECT RAISE(ABORT,'late reader publication'); END;").unwrap();
    assert!(s.apply_publication("source", args).is_err());
    assert!(
        s.conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM main.publication_row WHERE applied=1)",
                [],
                |r| r.get::<_, bool>(0)
            )
            .unwrap()
    );
    for (offset, expected) in [0, 100, 200].into_iter().zip(&old) {
        same(s.watch_page(offset).unwrap(), expected.clone());
    }
    assert_eq!(s.revision().unwrap(), revision);
    drop(s);
    let cached = Store::open_cached(&path).unwrap();
    for (offset, expected) in [0, 100, 200].into_iter().zip(old) {
        same(cached.watch_page(offset).unwrap(), expected);
    }
    drop(cached);
    let c = Connection::open(&path).unwrap();
    c.execute_batch("DROP TRIGGER fail_reader_publication")
        .unwrap();
    drop(c);
    let s = Store::open(&path).unwrap();
    assert!(!crate::publication::pending(&s.conn).unwrap());
    assert_eq!(s.revision().unwrap(), revision + 1);
    assert_eq!(state(&s.conn, &HISTORY[..HISTORY.len() - 1]), durable);
    let mut offset = 0;
    let mut ids = vec![];
    loop {
        let page = s.watch_page(offset).unwrap();
        same(page.clone(), oracle(&s, offset).unwrap());
        ids.extend(page.items.into_iter().map(|r| r.id));
        if let Some(next) = page.next_offset {
            offset = next;
        } else {
            break;
        }
    }
    assert_eq!(ids.len(), 385);
    let mut unique = ids.clone();
    unique.sort();
    unique.dedup();
    assert_eq!(ids, unique);
}
#[test]
fn crash_child_entry() {
    let Some(path) = std::env::var_os("GITJUKE_TEST_WATCH_READER_DB") else {
        return;
    };
    let s = Store {
        conn: Connection::open(path).unwrap(),
        cache_writes: false,
    };
    crate::shared_storage::register(&s.conn).unwrap();
    crate::publication::protect(&s.conn).unwrap();
    s.watch_page(128).unwrap();
    panic!("missed watch reader barrier");
}
#[test]
fn sigkill_late_protected_reads_preserve_exact_images_history_and_cached_pagination() {
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
        for phase in ["locators", "reasons"] {
            let d = tempfile::tempdir().unwrap();
            let path = d.path().join("db");
            let marker = d.path().join("marker");
            let mut s = small(&path);
            protected(&mut s, 257);
            s.conn.pragma_update(None, "journal_mode", journal).unwrap();
            let expected = oracle(&s, 128).unwrap();
            let history = state(&s.conn, HISTORY);
            let inputs = state(&s.conn, INPUTS);
            drop(s);
            let mut child = Owned(
                Command::new(std::env::current_exe().unwrap())
                    .args([
                        "--exact",
                        "watch_reader::tests::crash_child_entry",
                        "--nocapture",
                    ])
                    .env("GITJUKE_TEST_WATCH_READER_DB", &path)
                    .env("GITJUKE_TEST_WATCH_READER_PHASE", phase)
                    .env("GITJUKE_TEST_WATCH_READER_MARKER", &marker)
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
            same(cached.watch_page(128).unwrap(), expected);
            same(cached.watch_page(0).unwrap(), oracle(&cached, 0).unwrap());
            assert!(
                cached
                    .get::<serde_json::Value>("safe")
                    .unwrap()
                    .data
                    .is_some()
            );
            assert_eq!(state(&cached.conn, HISTORY), history);
            assert_eq!(state(&cached.conn, INPUTS), inputs);
        }
    }
}
