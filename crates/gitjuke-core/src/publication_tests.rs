use super::*;
use crate::model::*;
use serde_json::json;

fn n(c: &Connection, sql: &str) -> i64 {
    c.query_row(sql, [], |r| r.get(0)).unwrap()
}
fn pr(i: u64) -> PullRequest {
    serde_json::from_value(json!({"id":format!("P{i}"),"repoId":1,"repo":"a/b","number":i,"title":format!("PR {i}"),"url":format!("https://github.com/a/b/pull/{i}"),"state":"open","draft":false,"author":"reader","updatedAt":"2026-10-17T00:00:00Z"})).unwrap()
}
pub(crate) fn seed(path: &Path, kind: &str) -> (Store, Value) {
    let mut s = Store::open(path).unwrap();
    s.bind(&Identity {
        id: 42,
        login: "reader".into(),
    })
    .unwrap();
    let mut settings = s.settings().unwrap();
    settings.paused = true;
    s.save_settings(&settings).unwrap();
    s.conn.execute_batch("INSERT INTO activity VALUES('A','a/b',1,'comment',1,1,'eligible',1);INSERT INTO seen VALUES('A',0);INSERT INTO dedup VALUES('A');INSERT INTO progress VALUES('activity:baseline','1'),('catchup:A','A');INSERT INTO activity_cycle VALUES(1,'recovery',123,0);INSERT INTO activity_intent VALUES(1,'pr','attempted',1),(2,'summary','unavailable',1);INSERT INTO intent_member VALUES(1,'A'),(2,'A');INSERT INTO alert_attempt VALUES('attempt',1);INSERT INTO api_validator VALUES('notifications','etag',NULL,4070908800);INSERT INTO due VALUES('activity:notifications',4070908800);").unwrap();
    let time = now();
    let tx = s.conn.transaction().unwrap();
    for i in 0..257u64 {
        let p = pr(i + 1);
        tx.execute(
            "INSERT INTO watch_locator(id,repo,number,head,last_success) VALUES(?,'a/b',?,'old',?)",
            params![p.id, p.number as i64, time as i64],
        )
        .unwrap();
        tx.execute("INSERT INTO watch_source VALUES(?,'authored')", [&p.id])
            .unwrap();
        tx.execute("INSERT INTO watch VALUES(?,1,0,0)", [&p.id])
            .unwrap();
        tx.execute(
            "INSERT INTO content VALUES(?,?,?,?)",
            params![
                format!("list:{}", p.id),
                serde_json::to_string(&p).unwrap(),
                time as i64,
                time as i64
            ],
        )
        .unwrap();
        let mut fresh = pr(i + 129);
        fresh.title = "fresh".into();
        tx.execute(
            "INSERT INTO source_stage VALUES('authored',?,?)",
            params![fresh.id, serde_json::to_string(&fresh).unwrap()],
        )
        .unwrap();
        tx.execute(
            "INSERT INTO check_state VALUES('P1',?,?,1,NULL,?)",
            params![format!("sha-{i}"), time as i64, time as i64],
        )
        .unwrap();
        tx.execute("INSERT INTO check_context VALUES('P1','old',?,?,0)",params![format!("C{i:04}"),format!(r#"{{"id":"C{i:04}","kind":"StatusContext","name":"old","status":"PENDING","commit":"old"}}"# )]).unwrap();
        tx.execute("INSERT INTO check_context VALUES('P1','old',?,?,1)",params![format!("N{i:04}"),format!(r#"{{"id":"N{i:04}","kind":"StatusContext","name":"new","status":"SUCCESS","commit":"old"}}"# )]).unwrap();
        tx.execute(
            "INSERT INTO teams VALUES(?,?)",
            params![format!("T{i}"), format!("o/team{i}")],
        )
        .unwrap();
        tx.execute(
            "INSERT INTO teams_stage VALUES(?,?)",
            params![format!("T{}", i + 129), format!("o/team{}", i + 129)],
        )
        .unwrap();
        tx.execute(
            "INSERT INTO watch_source VALUES(?,?)",
            params![p.id, format!("team:T{i}")],
        )
        .unwrap();
    }
    tx.execute(
        "INSERT INTO check_state VALUES('P1','old',?,1,NULL,?)",
        params![time as i64, time as i64],
    )
    .unwrap();
    tx.execute("INSERT INTO resource_state VALUES('authored',?,1,NULL),('direct',?,1,NULL),('teams',?,1,NULL)",params![time as i64,time as i64,time as i64]).unwrap();
    tx.commit().unwrap();
    let args = match kind {
        "source" => json!({"time":time,"source":"authored"}),
        "teams" => json!({"time":time}),
        "checks" => {
            json!({"time":time,"id":"P1","commit":"old","first":false,"complete":true,"total":257,"contexts":[]})
        }
        "head" => {
            json!({"time":time,"pr":pr(1),"changed":true,"locator":["P1","a/b",1,1,"open",null,"new",null,1,time+10,time,null,0]})
        }
        _ => unreachable!(),
    };
    (s, args)
}
fn durable(c: &Connection) -> Vec<Vec<rusqlite::types::Value>> {
    let mut rows = vec![];
    for t in [
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
    ] {
        let mut q = c.prepare(&format!("SELECT * FROM {t} ORDER BY 1")).unwrap();
        let width = q.column_count();
        rows.extend(
            q.query_map([], |r| {
                (0..width)
                    .map(|i| r.get(i))
                    .collect::<rusqlite::Result<Vec<_>>>()
            })
            .unwrap()
            .map(|r| r.unwrap()),
        );
    }
    rows
}
fn visible(c: &Connection) -> Vec<Vec<rusqlite::types::Value>> {
    let mut rows = vec![];
    for t in [
        "watch_source",
        "watch_locator",
        "watch",
        "content",
        "check_context",
        "check_state",
        "teams",
        "resource_state",
    ] {
        // Staging is intentionally not a published surface.
        let filter = if t == "check_context" {
            " WHERE staged=0"
        } else {
            ""
        };
        let order = if t == "check_context" {
            "1,2,3,5"
        } else {
            "1,2"
        };
        let mut q = c
            .prepare(&format!("SELECT * FROM {t}{filter} ORDER BY {order}"))
            .unwrap();
        let width = q.column_count();
        rows.extend(
            q.query_map([], |r| {
                (0..width)
                    .map(|i| r.get(i))
                    .collect::<rusqlite::Result<Vec<_>>>()
            })
            .unwrap()
            .map(|r| r.unwrap()),
        );
    }
    rows
}
pub(crate) fn start(s: &mut Store, kind: &str, args: &Value) {
    s.conn
        .execute(
            "INSERT INTO publication(id,kind,args,phase) VALUES(1,?,?,'planning')",
            params![kind, args.to_string()],
        )
        .unwrap();
}
#[test]
fn bounded_batches_keep_old_snapshot_until_atomic_visibility_commit() {
    for kind in ["source", "head", "checks", "teams"] {
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("db");
        let (mut s, args) = seed(&path, kind);
        let old = visible(&s.conn);
        let history = durable(&s.conn);
        let revision = s.revision().unwrap();
        start(&mut s, kind, &args);
        let mut transactions = 0;
        let mut committed = false;
        COUNTS.with(|v| *v.borrow_mut() = Some(vec![]));
        while pending(&s.conn).unwrap() {
            batch(&mut s.conn).unwrap();
            transactions += 1;
            let cached = Store::open_cached(&path).unwrap();
            assert_eq!(durable(&cached.conn), history);
            if s.revision().unwrap() == revision {
                assert!(
                    visible(&cached.conn) == old,
                    "old snapshot changed for {kind} in batch {transactions}"
                );
            } else {
                committed = true;
                assert_eq!(s.revision().unwrap(), revision + 1);
            }
            if transactions % 25 == 0 {
                assert_eq!(
                    s.conn
                        .query_row("PRAGMA quick_check(1)", [], |r| r.get::<_, String>(0))
                        .unwrap(),
                    "ok"
                );
            }
        }
        let counts = COUNTS.with(|v| v.borrow_mut().take().unwrap());
        assert!(!counts.is_empty());
        assert!(counts.iter().all(|n| *n <= 128), "{kind}: {counts:?}");
        assert!(transactions > 20);
        assert!(committed);
        assert_ne!(visible(&s.conn), old);
        assert_eq!(durable(&s.conn), history);
        match kind {
            "source" => {
                assert_eq!(
                    n(
                        &s.conn,
                        "SELECT COUNT(*) FROM watch_source WHERE source='authored'"
                    ),
                    257
                );
                assert_eq!(
                    n(
                        &s.conn,
                        "SELECT COUNT(*) FROM watch_source WHERE source='authored' AND id='P1'"
                    ),
                    0
                );
            }
            "head" => {
                assert_eq!(
                    n(&s.conn, "SELECT epoch FROM watch_locator WHERE id='P1'"),
                    1
                );
                assert_eq!(
                    n(
                        &s.conn,
                        "SELECT COUNT(*) FROM check_state WHERE pr_id='P1' AND complete=1"
                    ),
                    0
                );
            }
            "checks" => {
                assert_eq!(
                    n(
                        &s.conn,
                        "SELECT COUNT(*) FROM check_context WHERE pr_id='P1' AND staged=0"
                    ),
                    257
                );
                assert_eq!(
                    n(
                        &s.conn,
                        "SELECT complete FROM check_state WHERE pr_id='P1' AND commit_sha='old'"
                    ),
                    1
                );
            }
            _ => assert_eq!(n(&s.conn, "SELECT COUNT(*) FROM teams"), 257),
        }
    }
}
#[test]
fn late_apply_and_visibility_faults_remain_old_and_resume_without_epoch_or_membership_loss() {
    for kind in ["source", "head", "checks", "teams"] {
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("db");
        let (mut s, args) = seed(&path, kind);
        let old = visible(&s.conn);
        let history = durable(&s.conn);
        start(&mut s, kind, &args);
        // Fail after one applied prefix, including its companion target mutation.
        while n(
            &s.conn,
            "SELECT COUNT(*) FROM publication_row WHERE applied=1",
        ) < 12
        {
            batch(&mut s.conn).unwrap();
        }
        s.conn.execute_batch("CREATE TRIGGER fail_applied BEFORE UPDATE ON publication_row WHEN NEW.applied=1 BEGIN SELECT RAISE(ABORT,'fixture apply'); END;").unwrap();
        assert!(resume(&mut s.conn).is_err());
        assert_eq!(visible(&s.conn), old);
        assert_eq!(durable(&s.conn), history);
        drop(s);
        assert!(Store::open(&path).is_err());
        let cached = Store::open_cached(&path).unwrap();
        assert_eq!(visible(&cached.conn), old);
        drop(cached);
        let c = Connection::open(&path).unwrap();
        c.execute_batch("DROP TRIGGER fail_applied; CREATE TRIGGER fail_commit BEFORE UPDATE ON publication WHEN NEW.phase='committed' BEGIN SELECT RAISE(ABORT,'fixture visibility'); END;").unwrap();
        drop(c);
        assert!(Store::open(&path).is_err());
        let cached = Store::open_cached(&path).unwrap();
        assert_eq!(visible(&cached.conn), old);
        assert_eq!(durable(&cached.conn), history);
        drop(cached);
        let c = Connection::open(&path).unwrap();
        c.execute_batch("DROP TRIGGER fail_commit").unwrap();
        drop(c);
        let s = Store::open(&path).unwrap();
        assert!(!pending(&s.conn).unwrap());
        assert_ne!(visible(&s.conn), old);
        assert_eq!(durable(&s.conn), history);
    }
}
#[test]
fn crash_child_entry() {
    let Some(path) = std::env::var_os("GITJUKE_TEST_PUBLICATION_DB") else {
        return;
    };
    let mut c = Connection::open(path).unwrap();
    resume(&mut c).unwrap();
    panic!("child did not reach barrier")
}
#[test]
fn sigkill_publications_preserve_old_or_complete_new_snapshots() {
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
    for kind in ["source", "head", "checks", "teams"] {
        for phase in [
            "plan_before",
            "plan_after",
            "apply_before",
            "apply_after",
            "commit_before",
            "commit_after",
            "cleanup_before",
            "cleanup_after",
            "finish_before",
            "finish_after",
        ] {
            let d = tempfile::tempdir().unwrap();
            let path = d.path().join("db");
            let marker = d.path().join("marker");
            let (mut s, args) = seed(&path, kind);
            let old = visible(&s.conn);
            let history = durable(&s.conn);
            let rev = s.revision().unwrap();
            start(&mut s, kind, &args);
            s.checkpoint().unwrap();
            drop(s);
            let mut child = OwnedChild(
                Command::new(std::env::current_exe().unwrap())
                    .args([
                        "--exact",
                        "publication::tests::crash_child_entry",
                        "--nocapture",
                    ])
                    .env("GITJUKE_TEST_PUBLICATION_DB", &path)
                    .env("GITJUKE_TEST_PUBLICATION_PHASE", phase)
                    .env("GITJUKE_TEST_PUBLICATION_MARKER", &marker)
                    .stdout(Stdio::null())
                    .spawn()
                    .unwrap(),
            );
            let start = Instant::now();
            while !marker.exists() {
                assert!(start.elapsed() < Duration::from_secs(30), "{kind}/{phase}");
                assert!(child.0.try_wait().unwrap().is_none());
                std::thread::sleep(Duration::from_millis(5));
            }
            child.0.kill().unwrap();
            use std::os::unix::process::ExitStatusExt;
            assert_eq!(child.0.wait().unwrap().signal(), Some(libc::SIGKILL));
            let c = Connection::open(&path).unwrap();
            assert_eq!(
                c.query_row("PRAGMA quick_check(1)", [], |r| r.get::<_, String>(0))
                    .unwrap(),
                "ok"
            );
            let committed = matches!(
                phase,
                "commit_after"
                    | "cleanup_before"
                    | "cleanup_after"
                    | "finish_before"
                    | "finish_after"
            );
            assert_eq!(
                n(
                    &c,
                    "SELECT COUNT(*) FROM publication WHERE phase='committed'"
                ) == 1,
                committed && phase != "finish_after"
            );
            drop(c);
            let cached = Store::open_cached(&path).unwrap();
            assert_eq!(durable(&cached.conn), history);
            assert_eq!(cached.revision().unwrap(), rev + u64::from(committed));
            if !committed {
                assert_eq!(visible(&cached.conn), old, "{kind}/{phase}");
            } else {
                assert_ne!(visible(&cached.conn), old);
            }
            let snapshot = visible(&cached.conn);
            drop(cached);
            let s = Store::open(&path).unwrap();
            assert_eq!(durable(&s.conn), history);
            assert!(!pending(&s.conn).unwrap());
            assert_ne!(visible(&s.conn), old);
            if committed {
                assert_eq!(visible(&s.conn), snapshot);
            }
            assert_eq!(s.revision().unwrap(), rev + 1);
        }
    }
}

#[test]
fn actual_ceiling_source_and_check_publications_transfer_staging_without_extra_budget() {
    super::small::FORCE.with(|v| v.set(true));
    let _serial = crate::content_capacity::CAPACITY.blocking_lock();
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("db");
    let mut s = Store::open(&path).unwrap();
    crate::content_budget::admission_tests::seed_full(&mut s);
    s.conn
        .execute("DELETE FROM content WHERE key='bulk-000'", [])
        .unwrap();
    let mut p = pr(7);
    let base = serde_json::to_string(&p).unwrap().len();
    p.title.push_str(&"x".repeat(1024 * 1024 - base));
    let body = serde_json::to_string(&p).unwrap();
    assert_eq!(body.len(), 1024 * 1024);
    s.conn
        .execute(
            "INSERT INTO source_stage VALUES('authored',?,?)",
            params![p.id, body],
        )
        .unwrap();
    assert_eq!(
        crate::content_budget::disposable_bytes(&s.conn).unwrap(),
        crate::store::CONTENT_LIMIT
    );
    s.publish_source("authored").unwrap();
    assert_eq!(
        crate::content_budget::disposable_bytes(&s.conn).unwrap(),
        crate::store::CONTENT_LIMIT
    );
    assert_eq!(n(&s.conn, "SELECT COUNT(*) FROM source_stage"), 0);
    s.conn
        .execute("DELETE FROM content WHERE key='bulk-001'", [])
        .unwrap();
    let mut ctx = json!({"id":"one","kind":"StatusContext","name":"CI","status":"SUCCESS","commit":"sha","padding":""});
    let base = ctx.to_string().len();
    ctx["padding"] = json!("x".repeat(1024 * 1024 - base));
    let body = ctx.to_string();
    assert_eq!(body.len(), 1024 * 1024);
    s.conn
        .execute("UPDATE watch_locator SET head='sha' WHERE id='P7'", [])
        .unwrap();
    s.conn
        .execute(
            "INSERT INTO check_context VALUES('P7','sha','one',?,1)",
            [body],
        )
        .unwrap();
    assert_eq!(
        crate::content_budget::disposable_bytes(&s.conn).unwrap(),
        crate::store::CONTENT_LIMIT
    );
    s.apply_publication("checks",json!({"time":now(),"id":"P7","commit":"sha","first":false,"complete":true,"total":1,"contexts":[]})).unwrap();
    assert_eq!(
        crate::content_budget::disposable_bytes(&s.conn).unwrap(),
        crate::store::CONTENT_LIMIT
    );
    assert_eq!(
        n(
            &s.conn,
            "SELECT complete FROM check_state WHERE pr_id='P7' AND commit_sha='sha'"
        ),
        1
    );
    drop(s);
    let s = Store::open(&path).unwrap();
    assert_eq!(
        crate::content_budget::disposable_bytes(&s.conn).unwrap(),
        crate::store::CONTENT_LIMIT
    );
    assert_eq!(
        n(
            &s.conn,
            "SELECT COUNT(*) FROM check_context WHERE pr_id='P7' AND staged=0"
        ),
        1
    );
}
#[test]
fn pending_publication_blocks_recovery_even_with_old_complete_health() {
    let d = tempfile::tempdir().unwrap();
    let mut s = Store::open(&d.path().join("db")).unwrap();
    s.activity_recovery(123).unwrap();
    s.conn.execute_batch("UPDATE activity_cycle SET notifications_done=1;INSERT INTO resource_state VALUES('authored',1,1,NULL),('direct',1,1,NULL),('teams',1,1,NULL);").unwrap();
    start(&mut s, "source", &json!({"time":now(),"source":"authored"}));
    let rev = s.revision().unwrap();
    s.activity_finish_recovery(false).unwrap();
    assert_eq!(s.revision().unwrap(), rev);
    resume(&mut s.conn).unwrap();
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
fn exact_retry_finishes_pending_publication_without_publishing_empty_or_advancing_epoch_twice() {
    use crate::sync::{CheckTarget, ContextPage, HeadRead, HeadSnapshot, WatchTarget};
    for kind in ["source", "teams", "checks", "head"] {
        let d = tempfile::tempdir().unwrap();
        let (mut s, _) = seed(&d.path().join("db"), kind);
        let revision = s.revision().unwrap();
        let before = durable(&s.conn);
        let target = WatchTarget {
            id: "P1".into(),
            repo: "a/b".into(),
            number: 1,
            generation: 0,
        };
        let checks = CheckTarget {
            watch: target.clone(),
            head: "old".into(),
            commit: "old".into(),
            cursor: "".into(),
            total: 257,
            page_size: 25,
        };
        let page = ContextPage {
            contexts: vec![],
            next: None,
            total: 257,
            commit: "old".into(),
        };
        let read = HeadRead {
            snapshot: HeadSnapshot {
                pr: pr(1),
                head: "new".into(),
                merge: None,
                review: None,
                generation: 0,
                fetched_at: now(),
            },
            pages: vec![],
        };
        let invoke = |s: &mut Store| -> Result<()> {
            match kind {
                "source" => s.publish_source("authored"),
                "teams" => s.publish_teams(),
                "checks" => s.stage_contexts(&checks, &page, false).map(|v| assert!(v)),
                _ => s
                    .accept_head(&read, &target)
                    .map(|v| assert_eq!(v.unwrap().generation, 1)),
            }
        };
        s.conn.execute_batch("CREATE TRIGGER retry_fault BEFORE UPDATE ON publication WHEN NEW.phase='committed' BEGIN SELECT RAISE(ABORT,'fixture retry'); END;").unwrap();
        small::FORCE.with(|v| v.set(true));
        let first = invoke(&mut s);
        small::FORCE.with(|v| v.set(false));
        assert!(first.is_err());
        assert!(pending(&s.conn).unwrap());
        s.conn.execute_batch("DROP TRIGGER retry_fault").unwrap();
        invoke(&mut s).unwrap();
        assert_eq!(s.revision().unwrap(), revision + 1);
        assert_eq!(durable(&s.conn), before);
        match kind {
            "source" => assert_eq!(
                n(
                    &s.conn,
                    "SELECT COUNT(*) FROM watch_source WHERE source='authored'"
                ),
                257
            ),
            "teams" => assert_eq!(n(&s.conn, "SELECT COUNT(*) FROM teams"), 257),
            "checks" => assert_eq!(
                n(
                    &s.conn,
                    "SELECT COUNT(*) FROM check_context WHERE pr_id='P1' AND staged=0"
                ),
                257
            ),
            _ => assert_eq!(
                n(&s.conn, "SELECT epoch FROM watch_locator WHERE id='P1'"),
                1
            ),
        }
    }
}
fn canonical(c: &Connection) -> Vec<Vec<rusqlite::types::Value>> {
    let mut rows = vec![];
    for t in TABLES {
        let order = t
            .keys
            .iter()
            .map(|i| t.columns[*i])
            .collect::<Vec<_>>()
            .join(",");
        let mut q = c
            .prepare(&format!("SELECT * FROM {} ORDER BY {order}", t.name))
            .unwrap();
        let width = q.column_count();
        rows.extend(
            q.query_map([], |r| {
                (0..width)
                    .map(|i| r.get(i))
                    .collect::<rusqlite::Result<Vec<_>>>()
            })
            .unwrap()
            .map(|r| r.unwrap()),
        );
    }
    rows
}
#[test]
fn small_single_transaction_and_journal_paths_publish_identical_snapshots() {
    for kind in [
        "source_page",
        "source",
        "selection",
        "selected",
        "head",
        "checks",
        "teams_page",
        "teams",
    ] {
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("db");
        let copy = d.path().join("copy");
        let (s, mut args) = seed(
            &path,
            if matches!(
                kind,
                "source_page" | "selection" | "selected" | "teams_page"
            ) {
                "source"
            } else {
                kind
            },
        );
        s.conn.execute_batch("DELETE FROM watch_locator WHERE number>3;DELETE FROM watch_source WHERE id NOT IN(SELECT id FROM watch_locator);DELETE FROM watch WHERE id NOT IN(SELECT id FROM watch_locator);UPDATE watch SET requested=1;
            DELETE FROM content WHERE key NOT IN('list:P1','list:P2','list:P3');DELETE FROM source_stage WHERE id NOT IN('P129','P130','P131');
            DELETE FROM check_context WHERE id NOT IN('C0000','C0001','C0002','N0000','N0001','N0002');DELETE FROM check_state WHERE commit_sha NOT IN('old','sha-0','sha-1','sha-2');
            DELETE FROM teams WHERE id NOT IN('T0','T1','T2');DELETE FROM teams_stage WHERE id NOT IN('T129','T130','T131');").unwrap();
        let time = args["time"].clone();
        match kind {
            "source_page" => args = json!({"time":time,"source":"authored","prs":[pr(9),pr(10)]}),
            "selection" => args = json!({"time":time,"repo":"a/b","number":2,"open":true}),
            "selected" => {
                let loc: String = s
                    .conn
                    .query_row(
                        &format!(
                            "SELECT {} FROM watch_locator b WHERE id='P1'",
                            row(table("watch_locator").unwrap(), "b")
                        ),
                        [],
                        |r| r.get(0),
                    )
                    .unwrap();
                args = json!({"time":time,"pr":pr(1),"locator":serde_json::from_str::<Value>(&loc).unwrap(),"open":true});
            }
            "checks" => args["total"] = json!(3),
            "teams_page" => {
                args = json!({"time":time,"first":true,"rows":[{"node_id":"T999","organization":{"login":"o"},"slug":"new"}]})
            }
            _ => {}
        }
        s.checkpoint().unwrap();
        drop(s);
        std::fs::copy(&path, &copy).unwrap();
        let mut fast = Store {
            conn: Connection::open(&path).unwrap(),
            cache_writes: true,
        };
        let mut journal = Store {
            conn: Connection::open(&copy).unwrap(),
            cache_writes: true,
        };
        COUNTS.with(|v| *v.borrow_mut() = Some(vec![]));
        fast.apply_publication(kind, args.clone()).unwrap();
        let counts = COUNTS.with(|v| v.borrow_mut().take().unwrap());
        assert_eq!(counts.len(), 1, "{kind}");
        assert!(counts[0] <= 128);
        small::FORCE.with(|v| v.set(true));
        let result = journal.apply_publication(kind, args);
        small::FORCE.with(|v| v.set(false));
        result.unwrap();
        assert!(canonical(&fast.conn) == canonical(&journal.conn), "{kind}");
        assert_eq!(fast.revision().unwrap(), journal.revision().unwrap());
    }
}

#[test]
fn empty_replacements_cannot_hide_large_deletions_behind_capped_preflight_counts() {
    for kind in ["checks_complete", "checks_first", "teams_page"] {
        let d = tempfile::tempdir().unwrap();
        let (mut s, _) = seed(&d.path().join("db"), "checks");
        let before = durable(&s.conn);
        let (operation, args) = match kind {
            "teams_page" => ("teams_page", json!({"time":now(),"rows":[],"first":true})),
            _ => (
                "checks",
                json!({"time":now(),"id":"P1","commit":"old","first":true,"complete":kind=="checks_complete","total":0,"contexts":[]}),
            ),
        };
        COUNTS.with(|v| *v.borrow_mut() = Some(vec![]));
        s.apply_publication(operation, args).unwrap();
        let counts = COUNTS.with(|v| v.borrow_mut().take().unwrap());
        assert!(counts.len() > 1, "{kind} must use bounded journal batches");
        assert!(counts.iter().all(|n| *n <= 128), "{kind}: {counts:?}");
        assert_eq!(durable(&s.conn), before);
        match kind {
            "teams_page" => assert_eq!(n(&s.conn, "SELECT COUNT(*) FROM teams_stage"), 0),
            "checks_complete" => {
                assert_eq!(
                    n(
                        &s.conn,
                        "SELECT COUNT(*) FROM check_context WHERE pr_id='P1' AND commit_sha='old'"
                    ),
                    0
                );
                assert_eq!(
                    n(
                        &s.conn,
                        "SELECT complete FROM check_state WHERE pr_id='P1' AND commit_sha='old'"
                    ),
                    1
                );
            }
            _ => {
                assert_eq!(
                    n(
                        &s.conn,
                        "SELECT COUNT(*) FROM check_context WHERE pr_id='P1' AND commit_sha='old' AND staged=1"
                    ),
                    0
                );
                assert_eq!(
                    n(
                        &s.conn,
                        "SELECT COUNT(*) FROM check_context WHERE pr_id='P1' AND commit_sha='old' AND staged=0"
                    ),
                    257
                );
            }
        }
    }
}
