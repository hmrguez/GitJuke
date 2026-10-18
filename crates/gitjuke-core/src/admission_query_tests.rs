use super::*;
use crate::{content_budget, now, store::Store};
use rusqlite::Connection;
use std::path::Path;
fn n(c: &Connection, sql: &str) -> i64 {
    c.query_row(sql, [], |r| r.get(0)).unwrap()
}
fn reference_bytes(c: &Connection) -> (i64, i64) {
    let staged = n(
        c,
        "SELECT (SELECT COALESCE(SUM(octet_length(json)),0) FROM stage)+(SELECT COALESCE(SUM(octet_length(json)),0) FROM source_stage)+(SELECT COALESCE(SUM(octet_length(json)),0) FROM activity_stage)+(SELECT COALESCE(SUM(octet_length(json)),0) FROM check_context WHERE staged=1)",
    );
    let old = n(
        c,
        "SELECT COALESCE(SUM(CASE WHEN (tbl='content' AND old_json IS NOT NULL) OR (tbl='check_context' AND json_extract(old_json,'$[4]')=0) THEN old_bytes ELSE 0 END),0) FROM publication_row",
    );
    (
        staged
            + old
            + n(
                c,
                "SELECT (SELECT COALESCE(SUM(octet_length(json)),0) FROM content)+(SELECT COALESCE(SUM(octet_length(json)),0) FROM check_context WHERE staged=0)",
            ),
        staged + old,
    )
}
fn reference_lru(c: &Connection, limit: usize, cutoff: Option<u64>) -> Vec<Victim> {
    c.prepare("SELECT * FROM (SELECT accessed,0 AS kind,key AS a,'' AS b,'' AS c,octet_length(json) AS bytes FROM content UNION ALL SELECT COALESCE(s.accessed,0),1,c.pr_id,c.commit_sha,c.id,octet_length(c.json) FROM check_context c LEFT JOIN check_state s USING(pr_id,commit_sha) WHERE c.staged=0) WHERE (?1 IS NULL OR accessed<?1) ORDER BY accessed,kind,a,b,c LIMIT ?2").unwrap().query_map(params![cutoff.map(|t|t as i64),limit as i64],victim).unwrap().collect::<rusqlite::Result<_>>().unwrap()
}
fn bounded() -> (usize, i32, i32) {
    let ms = metrics();
    assert!(!ms.is_empty());
    for m in &ms {
        assert!(
            m.rows <= WINDOW
                && m.sorts == 0
                && m.autoindexes == 0
                && m.scans <= 254
                && m.vm < 20_000,
            "{m:?}"
        );
    }
    eprintln!(
        "windows={}, max rows={}, VM={}, scan={}, SORT=0, AUTOINDEX=0",
        ms.len(),
        ms.iter().map(|m| m.rows).max().unwrap(),
        ms.iter().map(|m| m.vm).max().unwrap(),
        ms.iter().map(|m| m.scans).max().unwrap()
    );
    (
        ms.len(),
        ms.iter().map(|m| m.vm).max().unwrap(),
        ms.iter().map(|m| m.scans).max().unwrap(),
    )
}
#[test]
fn compact_large_sets_have_bounded_statements_and_exact_totals_and_lru() {
    let mut work = vec![];
    for count in [257, 1025] {
        let d = tempfile::tempdir().unwrap();
        let mut s = Store::open(&d.path().join("db")).unwrap();
        let tx = s.conn.transaction().unwrap();
        for i in 0..count {
            let k = format!("k-{i:04}");
            // Reverse timestamps force the best check victims into the last
            // window. Many ties, multi-context snapshots, orphans and staging.
            let at = ((count - i) / 8) as i64;
            tx.execute(
                "INSERT INTO content VALUES(?, ?,1,?)",
                params![k, if i % 7 == 0 { "" } else { "é" }, at],
            )
            .unwrap();
            if i % 3 != 0 {
                tx.execute(
                    "INSERT INTO check_state VALUES(?,'sha',1,1,NULL,?)",
                    params![k, at],
                )
                .unwrap();
            }
            for id in ["a", "z"] {
                tx.execute(
                    "INSERT INTO check_context VALUES(?,'sha',?,'é',0)",
                    params![k, id],
                )
                .unwrap();
            }
            tx.execute(
                "INSERT INTO check_context VALUES(?,'sha','a','staged',1)",
                [&k],
            )
            .unwrap();
            tx.execute("INSERT INTO stage VALUES(?,'é')", [&k]).unwrap();
            tx.execute("INSERT INTO source_stage VALUES('source',?,'é')", [&k])
                .unwrap();
            tx.execute("INSERT INTO activity_stage VALUES('a/b',1,?,'é')", [&k])
                .unwrap();
            // Interleave unrelated journal rows so eligibility cannot produce
            // an unbounded skipped prefix inside one byte-total statement.
            tx.execute(
                "INSERT INTO publication_row(tbl,k,old_json,old_bytes) VALUES(?,?,?,?)",
                params![
                    if i % 2 == 0 { "content" } else { "check_state" },
                    k,
                    "[1]",
                    i as i64
                ],
            )
            .unwrap();
        }
        tx.execute(
            "INSERT INTO content(rowid,key,json,fetched,accessed) VALUES(?,'minimum','é',1,-1)",
            [i64::MIN],
        )
        .unwrap();
        tx.execute(
            "INSERT INTO content(rowid,key,json,fetched,accessed) VALUES(?,'maximum','é',1,1)",
            [i64::MAX],
        )
        .unwrap();
        tx.commit().unwrap();
        capture();
        let total = bytes(&s.conn).unwrap();
        assert_eq!((total.total, total.protected), reference_bytes(&s.conn));
        work.push(bounded());
        for cutoff in [None, Some(1), Some(50)] {
            capture();
            assert_eq!(
                victims(&s.conn, 128, cutoff).unwrap(),
                reference_lru(&s.conn, 128, cutoff)
            );
            bounded();
        }
        assert!(victims(&s.conn, 0, None).unwrap().is_empty());
    }
    // More data means more bounded windows, not constant total operation work.
    assert!(work[1].0 > work[0].0);
    assert_eq!((work[1].1, work[1].2), (work[0].1, work[0].2));
}
#[test]
fn request_inputs_match_sqlite_utf8_charge_and_rollback() {
    let d = tempfile::tempdir().unwrap();
    let mut s = Store::open(&d.path().join("db")).unwrap();
    for (kind, field) in [
        ("source_page", "prs"),
        ("checks", "contexts"),
        ("head", "pr"),
        ("selected", "pr"),
    ] {
        let objects = (0..257)
            .map(|i| serde_json::json!({"id":i,"text":"é\\\"雪"}))
            .collect::<Vec<_>>();
        let value = if field == "pr" {
            objects[0].clone()
        } else {
            serde_json::json!(objects)
        };
        let args = serde_json::json!({field:value,"ignored":[1,2,3]});
        let tx = s.conn.transaction().unwrap();
        tx.execute(
            "INSERT INTO publication(id,kind,args,phase) VALUES(1,?,?,'planning')",
            params![kind, args.to_string()],
        )
        .unwrap();
        let expected = if field == "pr" {
            n(
                &tx,
                "SELECT octet_length(json_extract(args,'$.pr')) FROM publication",
            )
        } else {
            n(
                &tx,
                &format!(
                    "SELECT SUM(octet_length(value)) FROM publication,json_each(args,'$.{field}')"
                ),
            )
        };
        assert_eq!(bytes(&tx).unwrap().total, expected);
        assert_eq!(bytes(&tx).unwrap().protected, expected);
        tx.rollback().unwrap();
        assert_eq!(bytes(&s.conn).unwrap().total, 0);
    }
    // Retained published check images count; staged images do not.
    s.conn.execute_batch("INSERT INTO publication_row(tbl,k,old_json,old_bytes) VALUES('check_context','a','[0,0,0,0,0]',7),('check_context','b','[0,0,0,0,1]',13),('content','c','[]',11);").unwrap();
    assert_eq!(bytes(&s.conn).unwrap().total, 18);
    assert_eq!(bytes(&s.conn).unwrap().protected, 18);
}
fn seed(path: &Path) -> Store {
    let mut s = Store::open(path).unwrap();
    s.bind(&crate::model::Identity {
        id: 42,
        login: "reader".into(),
    })
    .unwrap();
    let mut settings = s.settings().unwrap();
    settings.paused = true;
    s.save_settings(&settings).unwrap();
    let tx = s.conn.transaction().unwrap();
    for i in 0..385 {
        tx.execute(
            "INSERT INTO content VALUES(?, ?,?,?)",
            params![
                format!("v-{i:04}"),
                "\"é\"",
                now() as i64,
                now() as i64 - 100
            ],
        )
        .unwrap();
    }
    tx.execute(
        "INSERT INTO content VALUES('safe','true',?,?)",
        [now() as i64; 2],
    )
    .unwrap();
    tx.execute_batch("INSERT INTO activity VALUES('old','a/b',1,'comment',1,1,'eligible',1);INSERT INTO seen VALUES('old',0);INSERT INTO dedup VALUES('old');INSERT INTO progress VALUES('activity:baseline','1'),('catchup:old','old');INSERT INTO activity_cycle VALUES(1,'recovery',123,0);INSERT INTO activity_intent VALUES(1,'pr','attempted',1),(2,'summary','unavailable',1);INSERT INTO intent_member VALUES(1,'old'),(2,'old');INSERT INTO alert_attempt VALUES('attempt',1);INSERT INTO api_validator VALUES('notifications','etag',NULL,4070908800);INSERT INTO due VALUES('activity:notifications',4070908800);").unwrap();
    tx.commit().unwrap();
    s
}
fn history(c: &Connection) -> Vec<String> {
    [
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
    ]
    .into_iter()
    .map(|t| {
        let mut q = c
            .prepare(&format!("SELECT * FROM {t} ORDER BY rowid"))
            .unwrap();
        let width = q.column_count();
        format!(
            "{t}:{:?}",
            q.query_map([], |r| (0..width)
                .map(|i| r.get::<_, rusqlite::types::Value>(i))
                .collect::<rusqlite::Result<Vec<_>>>())
                .unwrap()
                .collect::<rusqlite::Result<Vec<_>>>()
                .unwrap()
        )
    })
    .collect()
}
fn publish(s: &mut Store) -> crate::Result<()> {
    content_budget::publish_inner(
        &mut s.conn,
        now(),
        512,
        |tx| {
            tx.execute(
                "INSERT INTO content VALUES('candidate','true',?,?)",
                [now() as i64; 2],
            )?;
            tx.execute("INSERT INTO progress VALUES('candidate','committed')", [])?;
            crate::store::bump(tx)?;
            Ok(())
        },
        || {},
        |_| true,
        true,
    )
}
fn prefix(c: &Connection, count: i64) {
    assert_eq!(
        n(c, "SELECT COUNT(*) FROM content WHERE key LIKE 'v-%'"),
        385 - count
    );
    assert_eq!(
        n(
            c,
            &format!("SELECT COUNT(*) FROM content WHERE key>= 'v-{count:04}' AND key LIKE 'v-%'")
        ),
        385 - count
    );
}
#[test]
fn late_batch_fault_preserves_exact_prefix_and_retries_atomically() {
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("db");
    let mut s = seed(&path);
    let original = history(&s.conn);
    let revision = s.revision().unwrap();
    s.conn.execute_batch("CREATE TRIGGER fail_late BEFORE DELETE ON content WHEN old.key='v-0250' BEGIN SELECT RAISE(ABORT,'late victim'); END;").unwrap();
    assert!(publish(&mut s).is_err());
    prefix(&s.conn, 127);
    assert_eq!(s.revision().unwrap(), revision + 1);
    assert_eq!(history(&s.conn), original);
    assert_eq!(
        n(
            &s.conn,
            "SELECT COUNT(*) FROM content WHERE key='candidate'"
        ),
        0
    );
    drop(s);
    let mut cached = Store::open_cached(&path).unwrap();
    assert_eq!(cached.get::<bool>("safe").unwrap().data, Some(true));
    assert_eq!(history(&cached.conn), original);
    drop(cached);
    let mut s = Store::open(&path).unwrap();
    s.conn.execute_batch("DROP TRIGGER fail_late").unwrap();
    crate::publication::start_counts();
    publish(&mut s).unwrap();
    assert!(crate::publication::take_counts().iter().all(|v| *v <= 128));
    prefix(&s.conn, 259);
    assert_eq!(bytes(&s.conn).unwrap().total, 512);
    assert_eq!(
        n(
            &s.conn,
            "SELECT COUNT(*) FROM progress WHERE key='candidate'"
        ),
        1
    );
    s.conn
        .execute("DELETE FROM progress WHERE key='candidate'", [])
        .unwrap();
    assert_eq!(history(&s.conn), original);
}
#[test]
fn crash_child_entry() {
    let Some(path) = std::env::var_os("GITJUKE_TEST_ADMISSION_WINDOW_DB") else {
        return;
    };
    let mut s = Store {
        conn: Connection::open(path).unwrap(),
        cache_writes: true,
    };
    publish(&mut s).unwrap();
    panic!("missed barrier");
}
#[test]
fn sigkill_read_and_reclamation_keep_only_committed_prefixes() {
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
    for mode in ["DELETE", "WAL"] {
        for phase in ["query", "reclaim_before_commit", "reclaim_after_commit"] {
            let d = tempfile::tempdir().unwrap();
            let path = d.path().join("db");
            let marker = d.path().join("marker");
            let s = seed(&path);
            let original = history(&s.conn);
            let revision = s.revision().unwrap();
            s.conn.pragma_update(None, "journal_mode", mode).unwrap();
            drop(s);
            let mut command = Command::new(std::env::current_exe().unwrap());
            command
                .args([
                    "--exact",
                    "admission_query::tests::crash_child_entry",
                    "--nocapture",
                ])
                .env("GITJUKE_TEST_ADMISSION_WINDOW_DB", &path)
                .stdout(Stdio::null());
            if phase == "query" {
                command.env("GITJUKE_TEST_ADMISSION_QUERY_MARKER", &marker);
            } else {
                command
                    .env("GITJUKE_TEST_ADMISSION_PHASE", phase)
                    .env("GITJUKE_TEST_ADMISSION_MARKER", &marker);
            }
            let mut child = Owned(command.spawn().unwrap());
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
            let committed = phase == "reclaim_after_commit";
            prefix(&c, if committed { 127 } else { 0 });
            assert_eq!(history(&c), original);
            assert_eq!(
                n(
                    &c,
                    "SELECT CAST(value AS INTEGER) FROM meta WHERE key='revision'"
                ),
                revision as i64 + i64::from(committed)
            );
            drop(c);
            let mut cached = Store::open_cached(&path).unwrap();
            assert_eq!(cached.get::<bool>("safe").unwrap().data, Some(true));
            drop(cached);
            let mut s = Store::open(&path).unwrap();
            publish(&mut s).unwrap();
            prefix(&s.conn, 259);
            assert_eq!(bytes(&s.conn).unwrap().total, 512);
            s.conn
                .execute("DELETE FROM progress WHERE key='candidate'", [])
                .unwrap();
            assert_eq!(history(&s.conn), original);
        }
    }
}

#[test]
fn autocommit_totals_hold_one_snapshot_across_concurrent_window_changes() {
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("db");
    let mut s = Store::open(&path).unwrap();
    let tx = s.conn.transaction().unwrap();
    for i in 0..257 {
        tx.execute("INSERT INTO content VALUES(?,'é',1,1)", [format!("c-{i}")])
            .unwrap();
    }
    tx.execute("INSERT INTO stage VALUES('stage','é')", [])
        .unwrap();
    tx.commit().unwrap();
    let writer = Connection::open(&path).unwrap();
    ON_WINDOW.with(|f| *f.borrow_mut()=Some(Box::new(move || {
        writer.execute_batch("BEGIN IMMEDIATE;DELETE FROM content;DELETE FROM stage;INSERT INTO content VALUES('new','雪',1,1);COMMIT;").unwrap();
    })));
    let old = bytes(&s.conn).unwrap();
    assert_eq!((old.total, old.protected), (516, 2));
    let new = bytes(&s.conn).unwrap();
    assert_eq!((new.total, new.protected), (3, 0));
    assert!(s.conn.is_autocommit());
}

#[test]
fn retained_images_stay_charged_during_eviction_and_refuse_impossible_admission() {
    let d = tempfile::tempdir().unwrap();
    let mut s = Store::open(&d.path().join("db")).unwrap();
    s.conn.execute_batch("INSERT INTO content VALUES('old','true',4070908800,4070908800);INSERT INTO publication_row(tbl,k,old_json,old_bytes) VALUES('content','old','[]',18);").unwrap();
    let revision = s.revision().unwrap();
    let write = |tx: &rusqlite::Transaction<'_>| -> crate::Result<()> {
        tx.execute("INSERT INTO progress VALUES('candidate','1')", [])?;
        crate::store::bump(tx)?;
        Ok(())
    };
    assert!(
        content_budget::publish_inner(&mut s.conn, now(), 17, write, || {}, |_| true, true)
            .is_err()
    );
    assert_eq!(n(&s.conn, "SELECT COUNT(*) FROM content"), 1);
    assert_eq!(
        n(
            &s.conn,
            "SELECT COUNT(*) FROM progress WHERE key='candidate'"
        ),
        0
    );
    assert_eq!(s.revision().unwrap(), revision);
    content_budget::publish_inner(&mut s.conn, now(), 18, write, || {}, |_| true, true).unwrap();
    assert_eq!(n(&s.conn, "SELECT COUNT(*) FROM content"), 0);
    assert_eq!(bytes(&s.conn).unwrap().total, 18);
    assert_eq!(bytes(&s.conn).unwrap().protected, 18);
    assert_eq!(n(&s.conn, "SELECT old_bytes FROM publication_row"), 18);
    assert_eq!(s.revision().unwrap(), revision + 1);
}

#[test]
fn invalid_stored_request_stops_admission_without_writes_or_a_leaked_snapshot() {
    let d = tempfile::tempdir().unwrap();
    let mut s = Store::open(&d.path().join("db")).unwrap();
    s.conn
        .execute(
            "INSERT INTO publication(id,kind,args,phase) VALUES(1,'checks','{','planning')",
            [],
        )
        .unwrap();
    assert!(matches!(
        bytes(&s.conn),
        Err(crate::Error::StorageUnavailable(_))
    ));
    assert!(s.conn.is_autocommit());
    let revision = s.revision().unwrap();
    assert!(matches!(
        content_budget::publish_inner(
            &mut s.conn,
            now(),
            512,
            |tx| {
                tx.execute("INSERT INTO content VALUES('candidate','true',1,1)", [])?;
                crate::store::bump(tx)?;
                Ok(())
            },
            || {},
            |_| true,
            true
        ),
        Err(crate::Error::StorageUnavailable(_))
    ));
    assert_eq!(s.revision().unwrap(), revision);
    assert_eq!(n(&s.conn, "SELECT COUNT(*) FROM content"), 0);
    assert_eq!(
        s.conn
            .query_row("SELECT args FROM publication", [], |r| r
                .get::<_, String>(0))
            .unwrap(),
        "{"
    );
    assert!(s.conn.is_autocommit());
}
