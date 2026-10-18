use crate::{
    activity_crash::{identity, mark_ready, preserved, record, seed},
    now, publication,
    store::Store,
};
use rusqlite::Connection;
use std::{
    path::Path,
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};
fn n(c: &Connection, sql: &str) -> i64 {
    c.query_row(sql, [], |r| r.get(0)).unwrap()
}
fn text(c: &Connection, sql: &str) -> String {
    c.query_row(sql, [], |r| r.get(0)).unwrap()
}
pub(crate) fn fixture(path: &Path, kind: &str, mode: &str) -> Store {
    let time = now();
    let mut s = seed(path, mode, time);
    if kind == "summary" {
        // Build candidates through the production publisher, spanning two PRs.
        for number in [1, 2] {
            s.conn
                .execute(
                    "INSERT OR IGNORE INTO activity_target VALUES('a/b',?,0,0)",
                    [number],
                )
                .unwrap();
            let records = (0..257)
                .map(|i| record(&format!("N{number}-{i:04}"), time - 20))
                .collect::<Vec<_>>();
            s.activity_stage("a/b", number as u64, &records, true)
                .unwrap();
            s.activity_publish("a/b", number as u64, false, &identity())
                .unwrap();
        }
        s.activity_seen(&["N1-0000".into()]).unwrap();
        s.conn
            .execute("UPDATE activity SET occurred=1 WHERE id='N1-0001'", [])
            .unwrap();
        mark_ready(&mut s);
    } else {
        let records = (0..257)
            .map(|i| {
                record(
                    &format!("N1-{i:04}"),
                    if i == 256 { time + 60 } else { time - 20 },
                )
            })
            .collect::<Vec<_>>();
        s.activity_stage("a/b", 1, &records, true).unwrap();
    }
    s
}
fn run(s: &mut Store, kind: &str) {
    if kind == "summary" {
        s.activity_finish_recovery(false).unwrap()
    } else {
        s.activity_publish("a/b", 1, false, &identity()).unwrap()
    }
}
fn assert_new(s: &mut Store, kind: &str, mode: &str) {
    preserved(s, mode == "manual");
    if kind == "summary" {
        assert_eq!(text(&s.conn, "SELECT mode FROM activity_cycle"), "normal");
        assert_eq!(
            n(
                &s.conn,
                "SELECT COUNT(*) FROM progress WHERE key LIKE 'catchup:%'"
            ),
            0
        );
        assert_eq!(
            n(
                &s.conn,
                "SELECT COUNT(*) FROM intent_member m JOIN activity_intent i ON i.id=m.intent WHERE i.kind='summary' AND i.id>2"
            ),
            512
        );
        assert_eq!(
            n(
                &s.conn,
                "SELECT COUNT(*) FROM activity_intent WHERE kind='summary' AND id>2"
            ),
            1
        );
    } else {
        assert_eq!(
            n(
                &s.conn,
                "SELECT COUNT(*) FROM activity WHERE id LIKE 'N1-%'"
            ),
            257
        );
        assert_eq!(
            n(
                &s.conn,
                "SELECT COUNT(*) FROM dedup WHERE source_id LIKE 'N1-%'"
            ),
            257
        );
        assert_eq!(
            n(
                &s.conn,
                "SELECT COUNT(*) FROM seen WHERE source_id LIKE 'N1-%' AND seen=1"
            ),
            if mode == "baseline" { 257 } else { 0 }
        );
        assert_eq!(
            n(&s.conn, "SELECT done FROM activity_target WHERE number=1"),
            1
        );
        assert_eq!(n(&s.conn, "SELECT COUNT(*) FROM activity_stage"), 0);
        assert_eq!(
            n(
                &s.conn,
                "SELECT COUNT(*) FROM progress WHERE key LIKE 'catchup:%'"
            ),
            if mode == "recovery" { 256 } else { 0 }
        );
        assert_eq!(
            n(&s.conn, "SELECT COUNT(*) FROM intent_member WHERE intent>2"),
            match mode {
                "normal" => 257,
                "recovery" => 1,
                _ => 0,
            }
        );
        let page = s.activity_page(0).unwrap();
        assert!(
            page.items
                .iter()
                .filter(|r| r.id.starts_with("N1-"))
                .all(|r| r.text.as_deref() == Some("fixture payload"))
        );
    }
}
fn snapshot(c: &Connection) -> Vec<(String, String)> {
    let mut out = vec![];
    for t in [
        "activity",
        "seen",
        "dedup",
        "progress",
        "activity_target",
        "activity_cycle",
        "activity_intent",
        "intent_member",
    ] {
        let cols: Vec<String> = {
            let mut q = c.prepare(&format!("PRAGMA main.table_info({t})")).unwrap();
            q.query_map([], |r| r.get(1))
                .unwrap()
                .collect::<rusqlite::Result<_>>()
                .unwrap()
        };
        let mut q = c
            .prepare(&format!(
                "SELECT json_array({}) FROM {t} ORDER BY {}",
                cols.join(","),
                cols.join(",")
            ))
            .unwrap();
        out.extend(
            q.query_map([], |r| Ok((t.to_string(), r.get::<_, String>(0)?)))
                .unwrap()
                .map(|r| r.unwrap()),
        );
    }
    out
}

#[test]
fn large_publication_modes_and_summary_are_bounded_and_idempotent() {
    for mode in ["baseline", "normal", "recovery", "manual"] {
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("db");
        let mut s = fixture(&path, "activity", mode);
        publication::start_counts();
        run(&mut s, "activity");
        let counts = publication::take_counts();
        assert!(counts.len() > 10);
        assert!(counts.iter().all(|n| *n <= 128), "{counts:?}");
        assert_new(&mut s, "activity", mode);
        // Complete source IDs may be hydrated again, without reclassifying intents.
        let records = (0..257)
            .map(|i| record(&format!("N1-{i:04}"), now() - 20))
            .collect::<Vec<_>>();
        let intents = n(&s.conn, "SELECT COUNT(*) FROM activity_intent");
        s.activity_stage("a/b", 1, &records, true).unwrap();
        run(&mut s, "activity");
        assert_eq!(n(&s.conn, "SELECT COUNT(*) FROM activity_intent"), intents);
    }
    let d = tempfile::tempdir().unwrap();
    let mut s = fixture(&d.path().join("db"), "summary", "recovery");
    publication::start_counts();
    run(&mut s, "summary");
    let counts = publication::take_counts();
    assert!(counts.len() > 10);
    assert!(counts.iter().all(|n| *n <= 128));
    assert_new(&mut s, "summary", "recovery");
    let rev = s.revision().unwrap();
    run(&mut s, "summary");
    assert_eq!(s.revision().unwrap(), rev);
}
#[test]
fn late_apply_and_visibility_failure_protect_readers_and_exact_retry() {
    for kind in ["activity", "summary"] {
        for visibility in [false, true] {
            let d = tempfile::tempdir().unwrap();
            let path = d.path().join("db");
            let mut s = fixture(&path, kind, "recovery");
            let old_snapshot = snapshot(&s.conn);
            let old = s.activity_page(0).unwrap();
            let rev = s.revision().unwrap();
            s.conn.execute_batch(if visibility {"CREATE TRIGGER fail BEFORE UPDATE OF phase ON publication WHEN NEW.phase='committed' BEGIN SELECT RAISE(ABORT,'late visibility'); END;"} else {"CREATE TRIGGER fail BEFORE UPDATE OF applied ON publication_row WHEN NEW.applied=1 AND NEW.seq>(SELECT MIN(seq)+200 FROM publication_row) BEGIN SELECT RAISE(ABORT,'late application'); END;"}).unwrap();
            let result = if kind == "summary" {
                s.activity_finish_recovery(false)
            } else {
                s.activity_publish("a/b", 1, false, &identity())
            };
            assert!(result.is_err());
            assert!(
                n(
                    &s.conn,
                    "SELECT COUNT(*) FROM main.publication_row WHERE applied=1"
                ) > 0
            );
            assert_eq!(s.revision().unwrap(), rev);
            assert_eq!(snapshot(&s.conn), old_snapshot);
            assert_eq!(
                serde_json::to_value(s.activity_page(0).unwrap()).unwrap(),
                serde_json::to_value(&old).unwrap()
            );
            drop(s);
            let mut cached = Store::open_cached(&path).unwrap();
            assert_eq!(
                serde_json::to_value(cached.activity_page(0).unwrap()).unwrap(),
                serde_json::to_value(&old).unwrap()
            );
            drop(cached);
            let c = Connection::open(&path).unwrap();
            c.execute_batch("DROP TRIGGER fail").unwrap();
            drop(c);
            let mut s = Store {
                conn: Connection::open(&path).unwrap(),
                cache_writes: true,
            };
            // Same request must finish the persisted input, even after payload consumption.
            run(&mut s, kind);
            assert_new(&mut s, kind, "recovery");
            assert_eq!(s.revision().unwrap(), rev + 1);
            drop(s);
            let mut s = Store::open(&path).unwrap();
            assert_new(&mut s, kind, "recovery");
            assert_eq!(s.revision().unwrap(), rev + 1);
        }
    }
}
struct Owned(Child);
impl Drop for Owned {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
#[test]
fn child() {
    let Some(path) = std::env::var_os("GITJUKE_TEST_BOUNDED_ACTIVITY_DB") else {
        return;
    };
    let kind = std::env::var("GITJUKE_TEST_BOUNDED_ACTIVITY_KIND").unwrap();
    // Open cached input through a normal connection without startup's abandoned-stage cleanup.
    let mut s = Store {
        conn: Connection::open(Path::new(&path)).unwrap(),
        cache_writes: true,
    };
    s.conn.pragma_update(None, "synchronous", "FULL").unwrap();
    if kind == "reset" {
        s.activity_stage("a/b", 1, &[], true).unwrap();
    } else {
        run(&mut s, &kind);
    }
}
#[test]
fn actual_sigkill_large_activity_and_summary() {
    for kind in ["activity", "summary"] {
        for phase in [
            "plan_before",
            "plan_after",
            "prepared_before",
            "prepared_after",
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
            let marker = d.path().join("barrier");
            let mut s = fixture(&path, kind, "recovery");
            let rev = s.revision().unwrap();
            let old_snapshot = snapshot(&s.conn);
            let old = serde_json::to_value(s.activity_page(0).unwrap()).unwrap();
            drop(s);
            let mut child = Owned(
                Command::new(std::env::current_exe().unwrap())
                    .args(["--exact", "activity_bounded::child", "--nocapture"])
                    .env("GITJUKE_TEST_BOUNDED_ACTIVITY_DB", &path)
                    .env("GITJUKE_TEST_BOUNDED_ACTIVITY_KIND", kind)
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
            let raw = Connection::open(&path).unwrap();
            assert_eq!(text(&raw, "PRAGMA integrity_check"), "ok");
            let committed = matches!(
                phase,
                "commit_after"
                    | "cleanup_before"
                    | "cleanup_after"
                    | "finish_before"
                    | "finish_after"
            );
            let mut cached = Store::open_cached(&path).unwrap();
            if !committed {
                assert_eq!(snapshot(&cached.conn), old_snapshot);
                assert_eq!(
                    serde_json::to_value(cached.activity_page(0).unwrap()).unwrap(),
                    old,
                    "{kind}/{phase}"
                );
            } else {
                assert_new(&mut cached, kind, "recovery");
            }
            assert_eq!(cached.revision().unwrap(), rev + u64::from(committed));
            drop(cached);
            let journal = publication::pending(&raw).unwrap();
            drop(raw);
            let mut s = Store::open(&path).unwrap();
            if !journal && !committed {
                assert_eq!(
                    serde_json::to_value(s.activity_page(0).unwrap()).unwrap(),
                    old
                );
                if kind == "activity" {
                    let time = n(&s.conn, "SELECT boundary FROM activity_cycle") as u64 + 10;
                    let records = (0..257)
                        .map(|i| {
                            record(
                                &format!("N1-{i:04}"),
                                if i == 256 { time + 60 } else { time - 20 },
                            )
                        })
                        .collect::<Vec<_>>();
                    s.activity_stage("a/b", 1, &records, true).unwrap();
                }
                run(&mut s, kind);
            }
            assert_new(&mut s, kind, "recovery");
            assert_eq!(s.revision().unwrap(), rev + 1);
            assert!(!publication::pending(&s.conn).unwrap());
        }
    }
}

#[test]
fn staging_restart_and_notification_progress_share_global_bounds() {
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("db");
    let mut s = fixture(&path, "activity", "recovery");
    let old = serde_json::to_value(s.activity_page(0).unwrap()).unwrap();
    s.conn.execute_batch("CREATE TRIGGER fail_stage BEFORE DELETE ON activity_stage WHEN OLD.id='N1-0256' BEGIN SELECT RAISE(ABORT,'late stage cleanup'); END;").unwrap();
    publication::start_counts();
    assert!(s.activity_stage("a/b", 1, &[], true).is_err());
    let counts = publication::take_counts();
    assert!(counts.iter().all(|n| *n <= 128));
    assert_eq!(n(&s.conn, "SELECT COUNT(*) FROM activity_stage"), 1);
    assert_eq!(
        serde_json::to_value(s.activity_page(0).unwrap()).unwrap(),
        old
    );
    s.conn.execute_batch("DROP TRIGGER fail_stage").unwrap();
    let records = (0..257)
        .map(|i| record(&format!("replacement-{i}"), now()))
        .collect::<Vec<_>>();
    publication::start_counts();
    s.activity_stage("a/b", 1, &records, true).unwrap();
    let counts = publication::take_counts();
    assert!(counts.len() >= 4);
    assert!(counts.iter().all(|n| *n <= 128));
    assert_eq!(n(&s.conn, "SELECT COUNT(*) FROM activity_stage"), 257);
    assert!(s.activity_stage("a/b", 1, &records[0..1], false).is_err());
    assert_eq!(n(&s.conn, "SELECT done FROM activity_target"), 0);
    let targets = (2..52).map(|n| ("a/b".into(), n)).collect::<Vec<_>>();
    s.conn.execute_batch("CREATE TRIGGER fail_notifications BEFORE INSERT ON progress WHEN NEW.key='activity:notifications' BEGIN SELECT RAISE(ABORT,'notification progress'); END;").unwrap();
    assert!(s.activity_notifications(&targets, Some(123)).is_err());
    assert_eq!(n(&s.conn, "SELECT COUNT(*) FROM activity_target"), 1);
    s.conn
        .execute_batch("DROP TRIGGER fail_notifications")
        .unwrap();
    publication::start_counts();
    s.activity_notifications(&targets, Some(123)).unwrap();
    assert_eq!(publication::take_counts(), vec![53]);
    assert_eq!(s.activity_progress("activity:notifications").unwrap(), 123);
    let mut oversized = targets;
    oversized.push(("a/b".into(), 52));
    assert!(s.activity_notifications(&oversized, Some(999)).is_err());
    assert_eq!(s.activity_progress("activity:notifications").unwrap(), 123);
}

#[test]
fn large_baseline_completion_has_no_summary_and_one_visibility_commit() {
    let d = tempfile::tempdir().unwrap();
    let mut s = fixture(&d.path().join("db"), "activity", "baseline");
    run(&mut s, "activity");
    mark_ready(&mut s);
    s.conn
        .execute_batch(
            "INSERT INTO progress SELECT 'catchup:'||id,id FROM activity WHERE id LIKE 'N1-%'",
        )
        .unwrap();
    publication::start_counts();
    let rev = s.revision().unwrap();
    s.conn.execute_batch("CREATE TRIGGER fail_baseline BEFORE UPDATE OF phase ON publication WHEN NEW.phase='committed' BEGIN SELECT RAISE(ABORT,'baseline visibility'); END;").unwrap();
    assert!(s.activity_finish_recovery(false).is_err());
    assert!(!s.activity_page(0).unwrap().baseline);
    assert_eq!(text(&s.conn, "SELECT mode FROM activity_cycle"), "recovery");
    assert_eq!(
        n(
            &s.conn,
            "SELECT COUNT(*) FROM progress WHERE key LIKE 'catchup:%'"
        ),
        257
    );
    s.conn.execute_batch("DROP TRIGGER fail_baseline").unwrap();
    run(&mut s, "summary");
    assert!(publication::take_counts().iter().all(|n| *n <= 128));
    assert_eq!(s.revision().unwrap(), rev + 1);
    assert!(s.activity_page(0).unwrap().baseline);
    assert_eq!(text(&s.conn, "SELECT mode FROM activity_cycle"), "normal");
    assert_eq!(
        n(&s.conn, "SELECT COUNT(*) FROM activity_intent WHERE id>2"),
        0
    );
}

#[test]
fn actual_ceiling_activity_moves_staging_and_charges_retained_old_payloads() {
    let _serial = crate::content_capacity::CAPACITY.blocking_lock();
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("db");
    let mut s = fixture(&path, "activity", "normal");
    crate::content_budget::admission_tests::seed_full(&mut s);
    assert_eq!(
        crate::content_budget::disposable_bytes(&s.conn).unwrap(),
        crate::store::CONTENT_LIMIT
    );
    publication::start_counts();
    run(&mut s, "activity");
    assert!(publication::take_counts().iter().all(|n| *n <= 128));
    assert_new(&mut s, "activity", "normal");
    let intents = n(&s.conn, "SELECT COUNT(*) FROM activity_intent");
    let records = (0..257)
        .map(|i| {
            let mut r = record(&format!("N1-{i:04}"), now() - 20);
            r.text = Some("changed body".repeat(200));
            r
        })
        .collect::<Vec<_>>();
    // Keep old Activity bodies newer than disposable bulk filler. Staging may
    // legitimately reclaim older filler and increment its own cache revision.
    s.conn
        .execute(
            "UPDATE content SET accessed=(SELECT MIN(accessed)-1 FROM content WHERE key GLOB 'activity-payload:N1-*') WHERE key LIKE 'bulk-%'",
            [],
        )
        .unwrap();
    s.activity_stage("a/b", 1, &records, true).unwrap();
    assert_eq!(
        n(
            &s.conn,
            "SELECT COUNT(*) FROM content WHERE key LIKE 'activity-payload:N1-%'"
        ),
        257
    );
    let old = s.activity_page(0).unwrap();
    s.conn.execute_batch("CREATE TRIGGER fail_visible BEFORE UPDATE OF phase ON publication WHEN NEW.phase='committed' BEGIN SELECT RAISE(ABORT,'payload visibility'); END;").unwrap();
    assert!(s.activity_publish("a/b", 1, false, &identity()).is_err());
    assert!(
        n(
            &s.conn,
            "SELECT SUM(old_bytes) FROM main.publication_row WHERE tbl='content'"
        ) > 0
    );
    assert!(
        crate::content_budget::disposable_bytes(&s.conn).unwrap() <= crate::store::CONTENT_LIMIT
    );
    assert_eq!(
        serde_json::to_value(s.activity_page(0).unwrap()).unwrap(),
        serde_json::to_value(old).unwrap()
    );
    drop(s);
    let c = Connection::open(&path).unwrap();
    c.execute_batch("DROP TRIGGER fail_visible").unwrap();
    drop(c);
    let mut s = Store::open(&path).unwrap();
    assert!(
        s.activity_page(0)
            .unwrap()
            .items
            .iter()
            .filter(|r| r.id.starts_with("N1-"))
            .all(|r| r.text.as_deref() == Some("changed body".repeat(200).as_str()))
    );
    assert_eq!(n(&s.conn, "SELECT COUNT(*) FROM activity_intent"), intents);
    assert!(
        crate::content_budget::disposable_bytes(&s.conn).unwrap() <= crate::store::CONTENT_LIMIT
    );
}

#[test]
fn journal_classifies_expired_historical_suppressed_and_post_boundary_without_replay() {
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("db");
    let time = now();
    let mut s = seed(&path, "recovery", time);
    let mut records = (0..257)
        .map(|i| {
            let mut r = record(
                &format!("mixed-{i:04}"),
                match i % 5 {
                    0 => time - crate::retention::ACTIVITY_AGE - 1,
                    1 => time - 2000,
                    4 => time + 60,
                    _ => time - 20,
                },
            );
            if i % 5 == 2 {
                r.eligible = false;
                r.reason = "own action".into();
            }
            r
        })
        .collect::<Vec<_>>();
    s.activity_stage("a/b", 1, &records, true).unwrap();
    s.activity_publish("a/b", 1, false, &identity()).unwrap();
    assert_eq!(
        n(
            &s.conn,
            "SELECT COUNT(*) FROM activity WHERE id LIKE 'mixed-%'"
        ),
        205
    );
    assert_eq!(
        n(
            &s.conn,
            "SELECT COUNT(*) FROM dedup WHERE source_id LIKE 'mixed-%'"
        ),
        257
    );
    assert_eq!(
        n(
            &s.conn,
            "SELECT COUNT(*) FROM seen WHERE source_id LIKE 'mixed-%' AND seen=1"
        ),
        104
    );
    assert_eq!(
        n(
            &s.conn,
            "SELECT COUNT(*) FROM progress WHERE key LIKE 'catchup:%'"
        ),
        51
    );
    assert_eq!(
        n(&s.conn, "SELECT COUNT(*) FROM intent_member WHERE intent>2"),
        51
    );
    let history = snapshot(&s.conn);
    for r in &mut records {
        r.occurred = time;
        r.eligible = true;
    }
    s.activity_stage("a/b", 1, &records, true).unwrap();
    s.activity_publish("a/b", 1, false, &identity()).unwrap();
    // Only successful per-PR progress/target timestamps may advance on rehydration.
    let filtered = |rows: Vec<(String, String)>| {
        rows.into_iter()
            .filter(|(t, j)| {
                t != "activity_target" && !(t == "progress" && j.contains("activity:pr:"))
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(filtered(snapshot(&s.conn)), filtered(history));
    assert_eq!(
        n(
            &s.conn,
            "SELECT COUNT(*) FROM content WHERE key LIKE 'activity-payload:mixed-%'"
        ),
        205
    );
}

#[test]
fn actual_sigkill_staging_reset_requires_fresh_complete_hydration() {
    for phase in ["stage_reset_before_commit", "stage_reset_after_commit"] {
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("db");
        let marker = d.path().join("barrier");
        let mut s = fixture(&path, "activity", "recovery");
        let old = serde_json::to_value(s.activity_page(0).unwrap()).unwrap();
        drop(s);
        let mut child = Owned(
            Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "activity_bounded::child", "--nocapture"])
                .env("GITJUKE_TEST_BOUNDED_ACTIVITY_DB", &path)
                .env("GITJUKE_TEST_BOUNDED_ACTIVITY_KIND", "reset")
                .env("GITJUKE_TEST_ACTIVITY_PHASE", phase)
                .env("GITJUKE_TEST_ACTIVITY_MARKER", &marker)
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
        use std::os::unix::process::ExitStatusExt;
        assert_eq!(child.0.wait().unwrap().signal(), Some(libc::SIGKILL));
        let raw = Connection::open(&path).unwrap();
        assert_eq!(text(&raw, "PRAGMA integrity_check"), "ok");
        assert_eq!(
            n(&raw, "SELECT COUNT(*) FROM activity_stage"),
            if phase.ends_with("before_commit") {
                257
            } else {
                129
            }
        );
        assert_eq!(
            n(&raw, "SELECT COUNT(*) FROM activity WHERE id LIKE 'N1-%'"),
            0
        );
        drop(raw);
        let mut s = Store::open(&path).unwrap();
        assert_eq!(
            serde_json::to_value(s.activity_page(0).unwrap()).unwrap(),
            old
        );
        assert_eq!(n(&s.conn, "SELECT COUNT(*) FROM activity_stage"), 0);
        preserved(&s, false);
        let time = n(&s.conn, "SELECT boundary FROM activity_cycle") as u64 + 10;
        let records = (0..257)
            .map(|i| {
                record(
                    &format!("N1-{i:04}"),
                    if i == 256 { time + 60 } else { time - 20 },
                )
            })
            .collect::<Vec<_>>();
        s.activity_stage("a/b", 1, &records, true).unwrap();
        run(&mut s, "activity");
        assert_new(&mut s, "activity", "recovery");
    }
}
#[test]
fn source_api_payload_publication_reserves_its_three_mutations_from_admission() {
    let d = tempfile::tempdir().unwrap();
    let mut s = seed(&d.path().join("db"), "normal", now());
    for i in 0..257 {
        s.conn
            .execute(
                "INSERT INTO content VALUES(?,'{}',1,1)",
                [format!("expired-{i:04}")],
            )
            .unwrap();
    }
    publication::start_counts();
    let generation = s.generation().unwrap();
    s.put(
        "api:source",
        &serde_json::json!({"data":"fixture"}),
        generation,
    )
    .unwrap();
    assert_eq!(publication::take_counts(), vec![128]);
    assert_eq!(
        n(
            &s.conn,
            "SELECT COUNT(*) FROM content WHERE key LIKE 'expired-%'"
        ),
        132
    );
    preserved(&s, false);
}

#[test]
fn schema_nine_pending_watch_publication_upgrades_and_finishes_before_cleanup() {
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("db");
    let (mut s, args) = publication::tests::seed(&path, "head");
    publication::tests::start(&mut s, "head", &args);
    s.conn
        .execute_batch("DROP TABLE retention_cursor; DROP INDEX IF EXISTS activity_expiry; DROP INDEX IF EXISTS check_expiry; DROP INDEX IF EXISTS check_published_keys; DROP INDEX IF EXISTS publication_unapplied; DROP TABLE control_sweep; DROP INDEX content_lru")
        .unwrap();
    s.conn.pragma_update(None, "user_version", 9).unwrap();
    let rev = s.revision().unwrap();
    drop(s);
    let s = Store::open(&path).unwrap();
    assert_eq!(
        n(&s.conn, "PRAGMA user_version"),
        crate::store::SCHEMA_VERSION
    );
    assert!(!publication::pending(&s.conn).unwrap());
    assert_eq!(
        n(&s.conn, "SELECT epoch FROM watch_locator WHERE id='P1'"),
        1
    );
    assert_eq!(s.revision().unwrap(), rev + 1);
    assert_eq!(
        text(&s.conn, "SELECT state FROM activity_intent WHERE id=1"),
        "attempted"
    );
    assert_eq!(
        n(
            &s.conn,
            "SELECT COUNT(*) FROM intent_member WHERE activity='A'"
        ),
        2
    );
}
