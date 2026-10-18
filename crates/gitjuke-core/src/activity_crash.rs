//! Actual process death through the production staging/publication/recovery paths.
//! This entire module and its barriers are absent from production builds.
use crate::{activity::ActivityRecord, model::Identity, now, store::Store};
use rusqlite::{Connection, params};
use std::{
    path::Path,
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

pub(crate) fn point(phase: &str) {
    if std::env::var("GITJUKE_TEST_ACTIVITY_PHASE").as_deref() == Ok(phase) {
        std::fs::write(
            std::env::var_os("GITJUKE_TEST_ACTIVITY_MARKER").unwrap(),
            phase,
        )
        .unwrap();
        loop {
            std::thread::park();
        }
    }
}
fn count(c: &Connection, sql: &str) -> i64 {
    c.query_row(sql, [], |r| r.get(0)).unwrap()
}
fn text(c: &Connection, sql: &str) -> String {
    c.query_row(sql, [], |r| r.get(0)).unwrap()
}
pub(crate) fn identity() -> Identity {
    Identity {
        id: 42,
        login: "reader".into(),
    }
}
pub(crate) fn record(id: &str, occurred: u64) -> ActivityRecord {
    ActivityRecord {
        id: id.into(),
        repo: "a/b".into(),
        number: 1,
        kind: "comment".into(),
        occurred,
        actor: Some("other".into()),
        url: "https://github.com/a/b/pull/1#issuecomment-1".into(),
        text: Some("fixture payload".into()),
        eligible: true,
        reason: "eligible".into(),
        seen: false,
        head: None,
    }
}
fn records(time: u64, mode: &str) -> Vec<ActivityRecord> {
    vec![
        record("new-a", time - 20),
        record("new-b", time - 20),
        record(
            "new-z",
            if mode == "recovery" {
                time + 60
            } else {
                time - 20
            },
        ),
    ]
}
pub(crate) fn seed(path: &Path, mode: &str, time: u64) -> Store {
    let mut s = Store::open(path).unwrap();
    s.bind(&identity()).unwrap();
    let mut settings = s.settings().unwrap();
    settings.paused = mode == "manual";
    s.save_settings(&settings).unwrap();
    s.conn
        .execute_batch(
            "INSERT INTO activity VALUES('old','a/b',1,'comment',1,1,'eligible',1);
        INSERT INTO seen VALUES('old',0); INSERT INTO dedup VALUES('old');
        INSERT INTO activity_intent VALUES(1,'pr','attempted',1),(2,'summary','unavailable',1);
        INSERT INTO intent_member VALUES(1,'old'),(2,'old');
        INSERT INTO alert_attempt VALUES('attempt',1);
        INSERT INTO watch_locator(id,repo,number) VALUES('P','a/b',1);
        INSERT INTO watch_source VALUES('P','direct');
        INSERT INTO due VALUES('activity:notifications',4070908800);
        INSERT INTO api_validator VALUES('notifications','etag',NULL,4070908800);
        INSERT INTO activity_target VALUES('a/b',1,17,0);
        INSERT INTO progress VALUES('activity:pr:a/b:1','17');",
        )
        .unwrap();
    // Keep the preservation sentinel inside retention, independently of the source records.
    s.conn
        .execute(
            "UPDATE activity SET occurred=? WHERE id='old'",
            [time as i64],
        )
        .unwrap();
    s.conn
        .execute(
            "INSERT INTO activity_cycle VALUES(1,?,?,0)",
            params![
                if mode == "normal" {
                    "normal"
                } else {
                    "recovery"
                },
                (time - 10) as i64
            ],
        )
        .unwrap();
    s.conn
        .execute(
            "INSERT INTO content VALUES('existing','{}',?,?)",
            params![time as i64, time as i64],
        )
        .unwrap();
    if mode != "baseline" {
        s.conn
            .execute(
                "INSERT INTO progress VALUES('activity:baseline',?)",
                [(time - 1000).to_string()],
            )
            .unwrap();
    }
    s
}
pub(crate) fn preserved(s: &Store, paused: bool) {
    assert_eq!(s.identity().unwrap(), Some(identity()));
    assert_eq!(s.settings().unwrap().paused, paused);
    assert_eq!(
        count(&s.conn, "SELECT seen FROM seen WHERE source_id='old'"),
        0
    );
    assert_eq!(
        count(&s.conn, "SELECT COUNT(*) FROM dedup WHERE source_id='old'"),
        1
    );
    assert_eq!(
        text(&s.conn, "SELECT state FROM activity_intent WHERE id=1"),
        "attempted"
    );
    assert_eq!(
        text(&s.conn, "SELECT state FROM activity_intent WHERE id=2"),
        "unavailable"
    );
    assert_eq!(
        count(
            &s.conn,
            "SELECT COUNT(*) FROM intent_member WHERE activity='old'"
        ),
        2
    );
    assert_eq!(count(&s.conn, "SELECT COUNT(*) FROM alert_attempt"), 1);
    assert_eq!(
        count(
            &s.conn,
            "SELECT COUNT(*) FROM watch_source WHERE source='direct'"
        ),
        1
    );
    assert_eq!(
        text(&s.conn, "SELECT json FROM content WHERE key='existing'"),
        "{}"
    );
    assert_eq!(s.due("activity:notifications").unwrap(), 4070908800);
    assert_eq!(
        count(&s.conn, "SELECT next_at FROM api_validator"),
        4070908800
    );
    assert_eq!(
        count(
            &s.conn,
            "SELECT COUNT(*) FROM activity_intent WHERE state IN ('pending','recovery')"
        ),
        0
    );
}
pub(crate) fn mark_ready(s: &mut Store) {
    s.conn
        .execute("UPDATE activity_cycle SET notifications_done=1", [])
        .unwrap();
    for source in ["authored", "direct", "teams"] {
        s.conn.execute("INSERT INTO resource_state VALUES(?,1,1,NULL) ON CONFLICT(key) DO UPDATE SET complete=1,error=NULL",[source]).unwrap();
    }
}
fn ready(s: &mut Store) {
    mark_ready(s);
    s.activity_finish_recovery(false).unwrap();
}
struct OwnedChild(Child);
impl Drop for OwnedChild {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn kill_at(path: &Path, phase: &str, mode: &str, time: u64) {
    let marker = path.with_extension("barrier");
    let mut child = OwnedChild(
        Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "activity_crash::crash_child_entry",
                "--nocapture",
            ])
            .env("GITJUKE_TEST_ACTIVITY_DB", path)
            .env("GITJUKE_TEST_ACTIVITY_PHASE", phase)
            .env("GITJUKE_TEST_ACTIVITY_MODE", mode)
            .env("GITJUKE_TEST_ACTIVITY_TIME", time.to_string())
            .env("GITJUKE_TEST_ACTIVITY_MARKER", &marker)
            .stdout(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let start = Instant::now();
    while !marker.exists() {
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "barrier {mode}/{phase} not reached"
        );
        assert!(
            child.0.try_wait().unwrap().is_none(),
            "child exited before {mode}/{phase}"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    child.0.kill().unwrap();
    use std::os::unix::process::ExitStatusExt;
    assert_eq!(child.0.wait().unwrap().signal(), Some(libc::SIGKILL));
}
#[test]
fn crash_child_entry() {
    let Some(path) = std::env::var_os("GITJUKE_TEST_ACTIVITY_DB") else {
        return;
    };
    let mode = std::env::var("GITJUKE_TEST_ACTIVITY_MODE").unwrap();
    let time = std::env::var("GITJUKE_TEST_ACTIVITY_TIME")
        .unwrap()
        .parse::<u64>()
        .unwrap();
    let mut s = Store::open(Path::new(&path)).unwrap();
    if mode == "summary" || mode == "first_summary" {
        // Store startup removes abandoned staging, not committed catch-up membership.
        s.activity_finish_recovery(false).unwrap();
    } else {
        s.activity_stage("a/b", 1, &records(time, &mode), true)
            .unwrap();
        s.activity_publish("a/b", 1, mode == "manual", &identity())
            .unwrap();
    }
    panic!("child did not stop at the configured activity barrier");
}
#[test]
fn sigkill_publication_preserves_atomic_ledgers_and_reconciles_without_replay() {
    for mode in ["baseline", "normal", "recovery", "manual"] {
        for phase in [
            "stage_before_commit",
            "stage_after_commit",
            "publish_after_dedup",
            "publish_after_activity",
            "publish_after_intent_created",
            "publish_after_intent",
            "publish_after_payloads",
            "publish_after_progress",
            "publish_before_commit",
            "publish_after_commit",
        ] {
            if phase == "publish_after_intent_created" && matches!(mode, "baseline" | "manual") {
                continue;
            }
            let d = tempfile::tempdir().unwrap();
            let path = d.path().join("db");
            let time = now();
            let s = seed(&path, mode, time);
            let revision = s.revision().unwrap();
            drop(s);
            kill_at(&path, phase, mode, time);
            let c = Connection::open(&path).unwrap();
            assert_eq!(text(&c, "PRAGMA quick_check(1)"), "ok", "{mode}/{phase}");
            let committed = phase == "publish_after_commit";
            let new_count = if committed { 3 } else { 0 };
            for sql in [
                "SELECT COUNT(*) FROM activity WHERE id LIKE 'new-%'",
                "SELECT COUNT(*) FROM seen WHERE source_id LIKE 'new-%'",
                "SELECT COUNT(*) FROM dedup WHERE source_id LIKE 'new-%'",
                "SELECT COUNT(*) FROM content WHERE key LIKE 'activity-payload:new-%'",
            ] {
                assert_eq!(count(&c, sql), new_count, "{mode}/{phase}: {sql}");
            }
            assert_eq!(
                count(&c, "SELECT COUNT(*) FROM activity_stage"),
                if committed || phase == "stage_before_commit" {
                    0
                } else {
                    3
                }
            );
            assert_eq!(
                count(&c, "SELECT done FROM activity_target"),
                i64::from(committed)
            );
            assert_eq!(
                count(
                    &c,
                    "SELECT CAST(value AS INTEGER) FROM meta WHERE key='revision'"
                ),
                revision as i64 + i64::from(committed)
            );
            assert_eq!(
                count(
                    &c,
                    "SELECT CAST(value AS INTEGER) FROM progress WHERE key='activity:pr:a/b:1'"
                ) != 17,
                committed
            );
            let intent = committed && matches!(mode, "normal" | "recovery");
            assert_eq!(
                count(&c, "SELECT COUNT(*) FROM activity_intent WHERE id>2"),
                i64::from(intent)
            );
            assert_eq!(
                count(&c, "SELECT COUNT(*) FROM intent_member WHERE intent>2"),
                if intent {
                    if mode == "normal" { 3 } else { 1 }
                } else {
                    0
                }
            );
            assert_eq!(
                count(
                    &c,
                    "SELECT COUNT(*) FROM progress WHERE key LIKE 'catchup:%'"
                ),
                if committed && mode == "recovery" {
                    2
                } else {
                    0
                }
            );
            drop(c);
            let mut s = Store::open(&path).unwrap();
            preserved(&s, mode == "manual");
            assert_eq!(count(&s.conn, "SELECT COUNT(*) FROM activity_stage"), 0);
            // Restart gates publication on fresh full hydration. Reconcile the same
            // source IDs twice, as changed thread reasons/bodies can repeat them.
            s.activity_recovery(time).unwrap();
            for _ in 0..2 {
                s.activity_stage("a/b", 1, &records(time, mode), true)
                    .unwrap();
                s.activity_publish("a/b", 1, mode == "manual", &identity())
                    .unwrap();
            }
            ready(&mut s);
            ready(&mut s);
            preserved(&s, mode == "manual");
            assert_eq!(
                count(
                    &s.conn,
                    "SELECT COUNT(*) FROM activity WHERE id LIKE 'new-%'"
                ),
                3
            );
            assert_eq!(
                count(
                    &s.conn,
                    "SELECT COUNT(*) FROM seen WHERE source_id LIKE 'new-%' AND seen=1"
                ),
                if mode == "baseline" { 3 } else { 0 }
            );
            let expected_pr = if mode == "recovery" || (mode == "normal" && committed) {
                1
            } else {
                0
            };
            let expected_summary = if mode == "recovery" || (mode == "normal" && !committed) {
                1
            } else {
                0
            };
            assert_eq!(
                count(
                    &s.conn,
                    "SELECT COUNT(*) FROM activity_intent WHERE id>2 AND kind='pr'"
                ),
                expected_pr,
                "{mode}/{phase}"
            );
            assert_eq!(
                count(
                    &s.conn,
                    "SELECT COUNT(*) FROM activity_intent WHERE id>2 AND kind='summary'"
                ),
                expected_summary,
                "{mode}/{phase}"
            );
            if mode != "manual" {
                assert!(s.activity_page(0).unwrap().baseline);
            }
        }
    }
}

#[test]
fn sigkill_summary_commits_membership_progress_and_baseline_together() {
    for mode in ["summary", "first_summary"] {
        for phase in [
            "summary_after_intent",
            "summary_after_membership",
            "summary_after_progress",
            "summary_before_commit",
            "summary_after_commit",
        ] {
            if mode == "first_summary" && phase == "summary_after_intent" {
                continue;
            }
            let d = tempfile::tempdir().unwrap();
            let path = d.path().join("db");
            let time = now();
            let mut s = seed(
                &path,
                if mode == "first_summary" {
                    "baseline"
                } else {
                    "recovery"
                },
                time,
            );
            // All source records precede the recovery boundary. One becomes seen,
            // another expires before completion. Neither may join the summary.
            s.activity_stage("a/b", 1, &records(time, "normal"), true)
                .unwrap();
            s.activity_publish("a/b", 1, false, &identity()).unwrap();
            if mode == "summary" {
                s.activity_seen(&["new-b".into()]).unwrap();
                s.conn
                    .execute("UPDATE activity SET occurred=1 WHERE id='new-z'", [])
                    .unwrap();
                // Expiration is now scheduled separately from atomic publication.
                // Keep the same expired-entry/membership preconditions explicitly.
                s.maintain_retention_at(time).unwrap();
                let manual = record("manual", time - 20);
                s.activity_stage("a/b", 1, &[manual], true).unwrap();
                s.activity_publish("a/b", 1, true, &identity()).unwrap();
                let mut suppressed = record("suppressed", time - 20);
                suppressed.eligible = false;
                s.activity_stage("a/b", 1, &[suppressed], true).unwrap();
                s.activity_publish("a/b", 1, false, &identity()).unwrap();
                // Publication's retention already removes the expired record's
                // catch-up key. Restore it to exercise completion's own filter.
                s.conn
                    .execute(
                        "INSERT OR IGNORE INTO progress VALUES('catchup:new-z','new-z')",
                        [],
                    )
                    .unwrap();
                let mut other = record("other-pr", time - 20);
                other.repo = "c/d".into();
                other.number = 2;
                s.conn
                    .execute("INSERT INTO activity_target VALUES('c/d',2,0,0)", [])
                    .unwrap();
                s.activity_stage("c/d", 2, &[other], true).unwrap();
                s.activity_publish("c/d", 2, false, &identity()).unwrap();
                // Reinstate an expired minimal entry so completion itself must
                // filter by occurrence time, independently of retention cleanup.
                s.conn
                    .execute(
                        "INSERT INTO activity VALUES('new-z','a/b',1,'comment',1,1,'eligible',1)",
                        [],
                    )
                    .unwrap();
            }
            mark_ready(&mut s);
            let revision = s.revision().unwrap();
            let catchup = count(
                &s.conn,
                "SELECT COUNT(*) FROM progress WHERE key LIKE 'catchup:%'",
            );
            assert_eq!(catchup, if mode == "summary" { 4 } else { 0 });
            drop(s);
            kill_at(&path, phase, mode, time);
            let c = Connection::open(&path).unwrap();
            assert_eq!(text(&c, "PRAGMA quick_check(1)"), "ok");
            let committed = phase == "summary_after_commit";
            assert_eq!(
                count(
                    &c,
                    "SELECT COUNT(*) FROM activity_intent WHERE id>2 AND kind='summary'"
                ),
                i64::from(committed && mode == "summary")
            );
            assert_eq!(
                count(&c, "SELECT COUNT(*) FROM intent_member WHERE intent>2"),
                if committed && mode == "summary" { 2 } else { 0 }
            );
            assert_eq!(
                count(
                    &c,
                    "SELECT COUNT(*) FROM progress WHERE key LIKE 'catchup:%'"
                ),
                if committed { 0 } else { catchup }
            );
            assert_eq!(
                count(
                    &c,
                    "SELECT COUNT(*) FROM progress WHERE key='activity:baseline'"
                ),
                i64::from(committed || mode == "summary")
            );
            assert_eq!(
                text(&c, "SELECT mode FROM activity_cycle"),
                if committed { "normal" } else { "recovery" }
            );
            assert_eq!(
                count(
                    &c,
                    "SELECT CAST(value AS INTEGER) FROM meta WHERE key='revision'"
                ),
                revision as i64 + i64::from(committed)
            );
            drop(c);
            let mut s = Store::open(&path).unwrap();
            preserved(&s, false);
            s.activity_recovery(time + 100).unwrap();
            assert_eq!(
                count(&s.conn, "SELECT boundary FROM activity_cycle"),
                if committed {
                    (time + 100) as i64
                } else {
                    (time - 10) as i64
                }
            );
            // Reconciliation is required again after restart. Empty authoritative
            // batches suffice here; existing durable catch-up members survive.
            for (repo, number) in if mode == "summary" {
                vec![("a/b", 1), ("c/d", 2)]
            } else {
                vec![("a/b", 1)]
            } {
                s.activity_stage(repo, number, &[], true).unwrap();
                s.activity_publish(repo, number, false, &identity())
                    .unwrap();
            }
            ready(&mut s);
            ready(&mut s);
            preserved(&s, false);
            assert!(s.activity_page(0).unwrap().baseline);
            assert_eq!(text(&s.conn, "SELECT mode FROM activity_cycle"), "normal");
            assert_eq!(
                count(
                    &s.conn,
                    "SELECT COUNT(*) FROM progress WHERE key LIKE 'catchup:%'"
                ),
                0
            );
            assert_eq!(
                count(
                    &s.conn,
                    "SELECT COUNT(*) FROM activity_intent WHERE id>2 AND kind='summary'"
                ),
                i64::from(mode == "summary")
            );
            if mode == "summary" {
                assert_eq!(
                    count(&s.conn, "SELECT COUNT(*) FROM intent_member WHERE intent>2"),
                    2
                );
                for id in ["new-a", "other-pr"] {
                    assert_eq!(
                        s.conn
                            .query_row(
                                "SELECT COUNT(*) FROM intent_member WHERE intent>2 AND activity=?",
                                [id],
                                |r| r.get::<_, i64>(0)
                            )
                            .unwrap(),
                        1
                    );
                    assert_eq!(
                        s.conn
                            .query_row("SELECT seen FROM seen WHERE source_id=?", [id], |r| r
                                .get::<_, i64>(0))
                            .unwrap(),
                        0
                    );
                }
            }
        }
    }
}
