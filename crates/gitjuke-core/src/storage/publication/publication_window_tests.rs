use super::*;
use crate::publication_completion as query;
use serde_json::json;
use std::collections::BTreeMap;
fn n(c: &Connection, sql: &str) -> i64 {
    c.query_row(sql, [], |r| r.get(0)).unwrap()
}
fn cursor(c: &Connection) -> (i64, Option<i64>, Option<i64>) {
    c.query_row(
        "SELECT step,scan_after,scan_ceiling FROM main.publication",
        [],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )
    .unwrap()
}
fn bounded() -> usize {
    let ms = query::metrics();
    assert!(!ms.is_empty());
    for m in &ms {
        assert!(
            m.rows <= 128 && m.scans <= 128 && m.vm < 20_000 && m.sorts == 0 && m.autoindexes == 0,
            "{m:?}"
        );
    }
    eprintln!(
        "windows={}, max VM={}, max scans={}, sorts=0, automatic indexes=0",
        ms.len(),
        ms.iter().map(|m| m.vm).max().unwrap(),
        ms.iter().map(|m| m.scans).max().unwrap()
    );
    ms.len()
}
fn effective(c: &Connection) -> BTreeMap<(String, String), (Option<Value>, Option<Value>)> {
    let mut q = c
        .prepare(
            "SELECT tbl,k,new_json,reference,old_json FROM main.publication_row ORDER BY tbl,k",
        )
        .unwrap();
    q.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, Option<String>>(2)?,
            r.get::<_, Option<String>>(3)?,
            r.get::<_, Option<String>>(4)?,
        ))
    })
    .unwrap()
    .map(|r| {
        let (t, k, new, rf, old) = r.unwrap();
        let parse = |s: Option<String>| s.map(|s| serde_json::from_str::<Value>(&s).unwrap());
        ((t, k), (parse(new), parse(rf), parse(old)))
    })
    .filter(|(_, (new, rf, old))| rf.is_some() || new != old)
    .map(|(key, (new, rf, _))| (key, (new, rf)))
    .collect()
}
// Retain the previous unrestricted SQL only as an independent compact oracle.
// It is never called by production planning or completion.
fn legacy(c: &Connection, kind: &str, x: &Value) {
    for p in plans(kind, x).unwrap() {
        let rows = c
            .prepare(&format!("WITH candidate(k,n,ref) AS ({}) SELECT k,n,ref FROM candidate WHERE ?1 IS NOT NULL AND ?2 IS NOT NULL",p.sql))
            .unwrap()
            .query_map(params![p.tbl, x.to_string()], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, Option<String>>(1)?,
                    r.get::<_, Option<String>>(2)?,
                ))
            })
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        for (k, new, rf) in rows {
            let exists: bool = c
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM publication_row WHERE tbl=? AND k=?)",
                    params![p.tbl, k],
                    |r| r.get(0),
                )
                .unwrap();
            if exists {
                continue;
            }
            let t = table(p.tbl).unwrap();
            let old: Option<String> = c
                .query_row(
                    &format!(
                        "SELECT {} FROM {} b WHERE {}",
                        row(t, "b"),
                        t.name,
                        pred(t, "b")
                    ),
                    [&k],
                    |r| r.get(0),
                )
                .optional()
                .unwrap();
            if rf.is_none() && new == old {
                continue;
            }
            let keys: Value = serde_json::from_str(&k).unwrap();
            let old = if matches!(p.tbl, "activity_stage" | "source_stage" | "teams_stage")
                || p.tbl == "check_context" && keys[3] == 1
            {
                None
            } else {
                old
            };
            c.execute(
                "INSERT INTO publication_row(tbl,k,old_json,new_json,reference) VALUES(?,?,?,?,?)",
                params![p.tbl, k, old, new, rf],
            )
            .unwrap();
        }
    }
}
fn to_applying(s: &mut Store) {
    let rev = s.revision().unwrap();
    start_counts();
    while s
        .conn
        .query_row("SELECT phase='planning' FROM publication", [], |r| {
            r.get::<_, bool>(0)
        })
        .unwrap()
    {
        batch(&mut s.conn).unwrap();
        assert_eq!(s.revision().unwrap(), rev);
    }
    assert!(take_counts().iter().all(|n| *n <= 128));
}
#[test]
fn native_windows_match_unrestricted_preparation_oracle_for_every_kind() {
    for kind in [
        "source",
        "head",
        "checks",
        "teams",
        "source_page",
        "selection",
        "selected",
        "teams_page",
        "checks_first",
        "checks_partial",
        "activity",
        "activity_summary",
    ] {
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("db");
        let base = if matches!(kind, "source" | "head" | "checks" | "teams") {
            kind
        } else {
            "source"
        };
        let (mut s, mut args) = tests::seed(&path, base);
        let actual = match kind {
            "source_page" => {
                args = json!({"time":now(),"source":"authored","prs":[{"id":"new","repo":"a/b","number":99}]});
                "source_page"
            }
            "selection" => {
                args = json!({"time":now(),"repo":"a/b","number":1,"open":true});
                "selection"
            }
            "selected" => {
                args = json!({"time":now(),"pr":{"id":"P1","state":"open"},"open":true,"locator":["P1","a/b",1,1,"open",null,"new",null,0,0,null,null,0]});
                "selected"
            }
            "teams_page" => {
                args = json!({"time":now(),"first":true,"rows":[{"node_id":"T1","organization":{"login":"org"},"slug":"new"}]});
                "teams_page"
            }
            "checks_first" | "checks_partial" => {
                args = json!({"time":now(),"id":"P1","commit":"old","first":true,"complete":kind=="checks_first","total":1,"contexts":[{"id":"C0001"}]});
                "checks"
            }
            "activity" => {
                let time = now();
                let r = crate::activity_crash::record("new", time);
                s.activity_stage("a/b", 1, &[r], true).unwrap();
                args = json!({"time":time,"repo":"a/b","number":1,"baseline":1,"historical":false,"mode":"normal","boundary":0,"alerts":true,"intent":3});
                "activity"
            }
            "activity_summary" => {
                s.conn
                    .execute(
                        "UPDATE activity SET occurred=? WHERE id='A'",
                        [now() as i64],
                    )
                    .unwrap();
                args = json!({"time":now(),"intent":3});
                "activity_summary"
            }
            _ => kind,
        };
        // A rollback-only oracle keeps its temporary rows out of the real cursor run.
        let tx = s.conn.transaction().unwrap();
        legacy(&tx, actual, &args);
        let expected = effective(&tx);
        tx.rollback().unwrap();
        tests::start(&mut s, actual, &args);
        query::capture();
        to_applying(&mut s);
        bounded();
        let actual_rows = effective(&s.conn);
        assert_eq!(
            actual_rows.len(),
            expected.len(),
            "{kind}: changed row count"
        );
        for (key, row) in &expected {
            assert_eq!(actual_rows.get(key), Some(row), "{kind}: {key:?}");
        }
        crate::admission_query::tests::assert_exact(&s.conn);
        resume(&mut s.conn).unwrap();
        crate::admission_query::tests::assert_exact(&s.conn);
    }
}
fn prefix_fixture(path: &Path, count: usize) -> Store {
    let mut s = Store::open(path).unwrap();
    let tx = s.conn.transaction().unwrap();
    for i in 0..count {
        tx.execute(
            "INSERT INTO watch VALUES(?,0,0,0)",
            [format!("keep-{i:05}")],
        )
        .unwrap();
        tx.execute(
            "INSERT INTO source_stage VALUES('unrelated',?,'{}')",
            [format!("stage-{i:05}")],
        )
        .unwrap();
        tx.execute(
            "INSERT INTO progress VALUES(?,'durable')",
            [format!("history-{i:05}")],
        )
        .unwrap();
    }
    tx.execute_batch("INSERT INTO seen VALUES('history',0);INSERT INTO dedup VALUES('history');INSERT INTO activity_intent VALUES(1,'pr','attempted',1);INSERT INTO intent_member VALUES(1,'history');INSERT INTO alert_attempt VALUES('attempt',1);").unwrap();
    tx.commit().unwrap();
    tests::start(&mut s, "source", &json!({"time":now(),"source":"empty"}));
    s
}
#[test]
fn unchanged_and_prepared_prefixes_advance_bounded_cursors_without_repeated_scans() {
    let mut windows = vec![];
    for count in [257, 1025] {
        let d = tempfile::tempdir().unwrap();
        let mut s = prefix_fixture(&d.path().join("db"), count);
        query::capture();
        to_applying(&mut s);
        windows.push(bounded());
        assert_eq!(
            n(
                &s.conn,
                "SELECT COUNT(*) FROM publication_row WHERE tbl='watch'"
            ),
            count as i64
        );
        assert!(effective(&s.conn).contains_key(&("resource_state".into(), "[\"empty\"]".into())));
        resume(&mut s.conn).unwrap();
        assert_eq!(n(&s.conn, "SELECT COUNT(*) FROM progress"), count as i64);
        assert_eq!(n(&s.conn, "SELECT COUNT(*) FROM alert_attempt"), 1);
    }
    assert!(windows[1] > windows[0]); // cumulative work remains a bounded-window traversal
}
fn body_fixture(path: &Path) -> Store {
    let (mut s, args) = tests::seed(path, "source");
    tests::start(&mut s, "source", &args);
    while {
        let p = cursor(&s.conn);
        p.0 != 3 || p.1.is_none()
    } {
        batch(&mut s.conn).unwrap();
    }
    s
}
type Image = (String, String, Option<String>, Option<String>, i64);
fn images(c: &Connection) -> Vec<Image> {
    c.prepare("SELECT tbl,k,old_json,new_json,old_bytes FROM main.publication_row ORDER BY seq")
        .unwrap()
        .query_map([], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
        })
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap()
}
#[test]
fn cursor_late_fault_and_retry_keep_old_visibility_exact_accounting_and_history() {
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("db");
    let mut s = body_fixture(&path);
    let before = cursor(&s.conn);
    let journal = images(&s.conn);
    let usage = crate::admission_query::bytes(&s.conn).unwrap();
    let rev = s.revision().unwrap();
    s.conn.execute_batch("CREATE TRIGGER fail_cursor BEFORE UPDATE OF scan_after,scan_ceiling ON publication BEGIN SELECT RAISE(ABORT,'late planning cursor'); END;").unwrap();
    assert!(resume(&mut s.conn).is_err());
    assert_eq!(cursor(&s.conn), before);
    assert_eq!(images(&s.conn), journal);
    assert_eq!(s.revision().unwrap(), rev);
    let actual = crate::admission_query::bytes(&s.conn).unwrap();
    assert_eq!(
        (actual.total, actual.protected),
        (usage.total, usage.protected)
    );
    let mut cached = Store::open_cached(&path).unwrap();
    assert_eq!(
        cached
            .get::<crate::model::PullRequest>("list:P141")
            .unwrap()
            .data
            .unwrap()
            .title,
        "PR 141"
    );
    drop(cached);
    crate::admission_query::tests::assert_exact(&s.conn);
    s.conn.execute_batch("DROP TRIGGER fail_cursor").unwrap();
    drop(s);
    let mut s = Store::open(&path).unwrap();
    assert!(!pending(&s.conn).unwrap());
    assert_eq!(s.revision().unwrap(), rev + 1);
    assert_eq!(n(&s.conn, "SELECT COUNT(*) FROM alert_attempt"), 1);
    assert_eq!(
        s.get::<crate::model::PullRequest>("list:P141")
            .unwrap()
            .data
            .unwrap()
            .title,
        "fresh"
    );
}
#[test]
fn completion_windows_and_maintained_counts_match_full_oracles_behind_protected_prefixes() {
    for count in [257, 1025] {
        let d = tempfile::tempdir().unwrap();
        let mut s = Store::open(&d.path().join("db")).unwrap();
        let time = now();
        s.conn.execute_batch("INSERT INTO activity_cycle VALUES(1,'recovery',123,1);INSERT INTO resource_state VALUES('authored',1,1,NULL),('direct',1,1,NULL),('teams',1,1,NULL)").unwrap();
        let tx = s.conn.transaction().unwrap();
        for i in 0..count {
            let key = format!("k-{i:05}");
            tx.execute("INSERT INTO progress VALUES(?,'x')", [&key])
                .unwrap();
            tx.execute(
                "INSERT INTO activity_target VALUES('a/b',?,0,1)",
                [i as i64],
            )
            .unwrap();
            tx.execute(
                "INSERT INTO pending(key,job,manual,foreground,at) VALUES(?,'{}',0,0,0)",
                [&key],
            )
            .unwrap();
            tx.execute("INSERT INTO watch_locator(id,repo,number,head,last_success) VALUES(?,'a/b',?,'sha',1)",params![key,i as i64]).unwrap();
            tx.execute("INSERT INTO watch_source VALUES(?,'authored')", [&key])
                .unwrap();
            tx.execute(
                "INSERT INTO check_state VALUES(?,'sha',1,1,NULL,?)",
                params![key, time as i64],
            )
            .unwrap();
            tx.execute(
                "INSERT INTO check_context VALUES(?,'sha','ctx','',0)",
                [&key],
            )
            .unwrap();
            tx.execute("INSERT INTO check_context VALUES('P','sha',?,'',1)", [&key])
                .unwrap();
            tx.execute("INSERT INTO source_stage VALUES('source',?,'{}')", [&key])
                .unwrap();
        }
        tx.commit().unwrap();
        query::capture();
        assert!(query::ready(&s.conn, time).unwrap());
        assert_eq!(
            query::catchup_keys(&s.conn, 55).unwrap(),
            Vec::<String>::new()
        );
        assert_eq!(
            query::staged_total(
                &s.conn,
                "P",
                "sha",
                false,
                &["k-00000".into(), "new".into()]
            )
            .unwrap(),
            count as i64 + 1
        );
        assert_eq!(
            query::source_total(&s.conn, "source").unwrap(),
            count as u64
        );
        assert_eq!(
            query::published_count(&s.conn, "k-00000", "sha").unwrap(),
            1
        );
        bounded();
        s.conn
            .execute(
                "UPDATE activity_target SET done=0 WHERE number=?",
                [count as i64 - 1],
            )
            .unwrap();
        query::capture();
        assert!(!query::ready(&s.conn, time).unwrap());
        bounded();
        assert_eq!(
            n(
                &s.conn,
                "SELECT COUNT(*) FROM activity_cycle WHERE mode='recovery'"
            ),
            1
        );
        crate::admission_query::tests::assert_exact(&s.conn);
    }
}
#[test]
fn crash_child() {
    let Some(path) = std::env::var_os("GITJUKE_TEST_PLAN_DB") else {
        return;
    };
    let mut c = Connection::open(path).unwrap();
    crate::shared_storage::register(&c).unwrap();
    resume(&mut c).unwrap();
    panic!("missed planning barrier");
}
#[test]
fn sigkill_late_cursor_and_journal_commit_together_then_resume_without_replay() {
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
        for after in [false, true] {
            let d = tempfile::tempdir().unwrap();
            let path = d.path().join("db");
            let marker = d.path().join("marker");
            let mut s = body_fixture(&path);
            let old = cursor(&s.conn);
            let old_images = images(&s.conn);
            let old_usage = crate::admission_query::bytes(&s.conn).unwrap();
            let rev = s.revision().unwrap();
            let args: String = s
                .conn
                .query_row("SELECT args FROM publication", [], |r| r.get(0))
                .unwrap();
            let tx = s.conn.transaction().unwrap();
            planning::prepare(
                &tx,
                "source",
                &serde_json::from_str(&args).unwrap(),
                old.0,
                old.1,
                old.2,
            )
            .unwrap();
            let expected_cursor = cursor(&tx);
            let expected_images = images(&tx);
            let expected_usage = crate::admission_query::bytes(&tx).unwrap();
            tx.rollback().unwrap();
            assert_eq!(expected_images.len(), old_images.len() + 12);
            assert!(expected_usage.protected > old_usage.protected);
            s.conn.pragma_update(None, "journal_mode", mode).unwrap();
            drop(s);
            let mut child = Owned(
                Command::new(std::env::current_exe().unwrap())
                    .args([
                        "--exact",
                        "publication::window_tests::crash_child",
                        "--nocapture",
                    ])
                    .env("GITJUKE_TEST_PLAN_DB", &path)
                    .env(
                        "GITJUKE_TEST_PUBLICATION_PHASE",
                        if after {
                            "cursor_after"
                        } else {
                            "cursor_before"
                        },
                    )
                    .env("GITJUKE_TEST_PUBLICATION_MARKER", &marker)
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
            assert_eq!(cursor(&c), if after { expected_cursor } else { old });
            assert_eq!(images(&c), if after { expected_images } else { old_images });
            let usage = crate::admission_query::bytes(&c).unwrap();
            let expected = if after { expected_usage } else { old_usage };
            assert_eq!(
                (usage.total, usage.protected),
                (expected.total, expected.protected)
            );
            assert_eq!(
                n(
                    &c,
                    "SELECT CAST(value AS INTEGER) FROM meta WHERE key='revision'"
                ),
                rev as i64
            );
            crate::admission_query::tests::assert_exact(&c);
            drop(c);
            let mut cached = Store::open_cached(&path).unwrap();
            assert_eq!(
                cached
                    .get::<crate::model::PullRequest>("list:P141")
                    .unwrap()
                    .data
                    .unwrap()
                    .title,
                "PR 141"
            );
            drop(cached);
            let mut s = Store::open(&path).unwrap();
            assert!(!pending(&s.conn).unwrap());
            assert_eq!(s.revision().unwrap(), rev + 1);
            assert_eq!(n(&s.conn, "SELECT COUNT(*) FROM alert_attempt"), 1);
            assert_eq!(
                s.get::<crate::model::PullRequest>("list:P141")
                    .unwrap()
                    .data
                    .unwrap()
                    .title,
                "fresh"
            );
        }
    }
}

fn ready_fixture(path: &Path) -> Store {
    let mut s = Store::open(path).unwrap();
    s.bind(&crate::model::Identity {
        id: 42,
        login: "reader".into(),
    })
    .unwrap();
    s.conn.execute_batch("INSERT INTO activity_cycle VALUES(1,'recovery',123,1);INSERT INTO resource_state VALUES('authored',1,1,NULL),('direct',1,1,NULL),('teams',1,1,NULL);INSERT INTO progress VALUES('activity:baseline','1'),('catchup:missed','missed');INSERT INTO seen VALUES('missed',0);INSERT INTO dedup VALUES('missed');INSERT INTO activity_intent VALUES(1,'pr','attempted',1);INSERT INTO intent_member VALUES(1,'missed');INSERT INTO alert_attempt VALUES('attempt',1);INSERT INTO due VALUES('activity:notifications',4070908800);INSERT INTO api_validator VALUES('notifications','etag',NULL,4070908800)").unwrap();
    s.conn
        .execute(
            "INSERT INTO activity VALUES('missed','a/b',1,'comment',?,1,'eligible',?)",
            [now() as i64; 2],
        )
        .unwrap();
    let tx = s.conn.transaction().unwrap();
    for i in 0..257 {
        tx.execute("INSERT INTO activity_target VALUES('a/b',?,0,1)", [i])
            .unwrap();
        tx.execute(
            "INSERT INTO resource_state VALUES(?,1,1,NULL)",
            [format!("unrelated-{i:04}")],
        )
        .unwrap();
    }
    tx.commit().unwrap();
    s
}
#[test]
fn readiness_windows_hold_one_snapshot_and_do_not_publish_partial_completion() {
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("db");
    let s = ready_fixture(&path);
    let writer = Connection::open(&path).unwrap();
    query::ON_WINDOW.with(|f|*f.borrow_mut()=Some(("activity_target".into(),Box::new(move||{writer.execute_batch("BEGIN IMMEDIATE;UPDATE activity_target SET done=0 WHERE number=256;UPDATE resource_state SET error='late fault' WHERE key='teams';COMMIT").unwrap();}))));
    assert!(query::ready(&s.conn, now()).unwrap());
    assert!(!query::ready(&s.conn, now()).unwrap());
    assert_eq!(
        n(
            &s.conn,
            "SELECT COUNT(*) FROM activity_cycle WHERE mode='recovery'"
        ),
        1
    );
    assert_eq!(n(&s.conn, "SELECT COUNT(*) FROM activity_intent"), 1);
    assert_eq!(
        n(
            &s.conn,
            "SELECT COUNT(*) FROM progress WHERE key='catchup:missed'"
        ),
        1
    );
}
#[test]
fn completion_crash_child() {
    let Some(path) = std::env::var_os("GITJUKE_TEST_COMPLETION_DB") else {
        return;
    };
    let mut s = Store {
        conn: Connection::open(path).unwrap(),
        cache_writes: true,
    };
    crate::shared_storage::register(&s.conn).unwrap();
    s.activity_finish_recovery(false).unwrap();
    panic!("missed read barrier");
}
#[test]
fn sigkill_late_readiness_window_never_commits_a_summary_or_completion_claim() {
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
        let s = ready_fixture(&path);
        let rev = s.revision().unwrap();
        s.conn.pragma_update(None, "journal_mode", journal).unwrap();
        drop(s);
        let mut child = Owned(
            Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "publication::window_tests::completion_crash_child",
                    "--nocapture",
                ])
                .env("GITJUKE_TEST_COMPLETION_DB", &path)
                .env("GITJUKE_TEST_COMPLETION_TABLE", "resource_state")
                .env("GITJUKE_TEST_COMPLETION_MARKER", &marker)
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
        assert_eq!(
            n(
                &c,
                "SELECT COUNT(*) FROM activity_cycle WHERE mode='recovery' AND boundary=123"
            ),
            1
        );
        assert_eq!(
            n(
                &c,
                "SELECT COUNT(*) FROM progress WHERE key='catchup:missed'"
            ),
            1
        );
        assert_eq!(n(&c, "SELECT COUNT(*) FROM activity_intent"), 1);
        assert_eq!(
            n(
                &c,
                "SELECT CAST(value AS INTEGER) FROM meta WHERE key='revision'"
            ),
            rev as i64
        );
        drop(c);
        let mut s = Store::open(&path).unwrap();
        s.activity_finish_recovery(false).unwrap();
        s.activity_finish_recovery(false).unwrap();
        assert_eq!(
            n(
                &s.conn,
                "SELECT COUNT(*) FROM activity_cycle WHERE mode='normal'"
            ),
            1
        );
        assert_eq!(
            n(
                &s.conn,
                "SELECT COUNT(*) FROM activity_intent WHERE kind='summary' AND state='unavailable'"
            ),
            1
        );
        assert_eq!(
            n(
                &s.conn,
                "SELECT COUNT(*) FROM alert_attempt WHERE id='attempt'"
            ),
            1
        );
        assert_eq!(
            n(
                &s.conn,
                "SELECT at FROM due WHERE key='activity:notifications'"
            ),
            4070908800
        );
        assert_eq!(s.revision().unwrap(), rev + 1);
    }
}
#[test]
fn small_preflight_uses_scope_indexes_before_rejecting_large_valid_prefixes() {
    for count in [257, 1025] {
        let d = tempfile::tempdir().unwrap();
        let mut s = Store::open(&d.path().join("db")).unwrap();
        s.conn
            .execute_batch(
                "INSERT INTO teams VALUES('T','o/t');INSERT INTO teams_stage VALUES('T','o/t')",
            )
            .unwrap();
        for i in 0..count {
            s.conn
                .execute(
                    "INSERT INTO watch_source VALUES(?,'team:T')",
                    [format!("P{i:05}")],
                )
                .unwrap();
            s.conn
                .execute(
                    "INSERT INTO check_context VALUES('P','sha',?,'',0)",
                    [format!("ctx-{i:05}")],
                )
                .unwrap();
        }
        let changes = s.conn.total_changes();
        query::capture();
        assert!(!small::publish(&mut s.conn, "teams", &json!({"time":now()})).unwrap());
        assert_eq!(
            small::count(
                &s.conn,
                "check_context",
                "pr_id='P' AND commit_sha='sha' AND staged=1",
                "{}"
            )
            .unwrap(),
            0
        );
        bounded();
        assert_eq!(s.conn.total_changes(), changes);
    }
}

#[test]
fn signed_native_positions_and_invalid_cursors_preserve_original_preparation() {
    let d = tempfile::tempdir().unwrap();
    let mut s = prefix_fixture(&d.path().join("db"), 0);
    s.conn
        .execute(
            "INSERT INTO watch(rowid,id,authored,requested,selected) VALUES(?,'first',0,0,0)",
            [i64::MIN],
        )
        .unwrap();
    s.conn
        .execute(
            "INSERT INTO watch(rowid,id,authored,requested,selected) VALUES(?,'last',0,0,0)",
            [i64::MAX],
        )
        .unwrap();
    to_applying(&mut s);
    assert_eq!(
        n(
            &s.conn,
            "SELECT COUNT(*) FROM publication_row WHERE tbl='watch'"
        ),
        2
    );
    resume(&mut s.conn).unwrap();
    tests::start(&mut s, "source", &json!({"time":now(),"source":"empty"}));
    s.conn
        .execute("UPDATE publication SET scan_after=5,scan_ceiling=4", [])
        .unwrap();
    let before = images(&s.conn);
    assert!(batch(&mut s.conn).is_err());
    assert_eq!(images(&s.conn), before);
    assert_eq!(cursor(&s.conn), (0, Some(5), Some(4)));
    s.conn
        .execute(
            "UPDATE publication SET scan_after=NULL,scan_ceiling=NULL,phase='unknown'",
            [],
        )
        .unwrap();
    assert!(batch(&mut s.conn).is_err());
    assert_eq!(images(&s.conn), before);
    assert_eq!(n(&s.conn, "SELECT COUNT(*) FROM publication"), 1);
    crate::admission_query::tests::assert_exact(&s.conn);
}
