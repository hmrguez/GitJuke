use super::*;
use crate::{
    activity_crash::{identity, record},
    now,
};
fn setup() -> (tempfile::TempDir, Store) {
    let d = tempfile::tempdir().unwrap();
    let mut s = Store::open(&d.path().join("db")).unwrap();
    s.bind(&identity()).unwrap();
    let mut settings = s.settings().unwrap();
    settings.alerts.enabled = true;
    settings.alerts.permission = Permission::Granted;
    settings.alerts.onboarded = true;
    s.save_settings(&settings).unwrap();
    s.conn.execute_batch("INSERT INTO activity_cycle VALUES(1,'normal',0,1); INSERT INTO activity_target VALUES('a/b',1,0,0); INSERT INTO progress VALUES('activity:baseline','1'); INSERT INTO resource_state VALUES('authored',1,1,NULL),('direct',1,1,NULL),('teams',1,1,NULL);").unwrap();
    (d, s)
}
fn publish(s: &mut Store, items: &[ActivityRecord]) {
    s.activity_stage("a/b", 1, items, true).unwrap();
    s.activity_publish("a/b", 1, false, &identity()).unwrap();
}
fn count(s: &Store, sql: &str) -> i64 {
    s.conn.query_row(sql, [], |r| r.get(0)).unwrap()
}
#[test]
fn opt_in_and_permission_boundaries_never_replay_backlog() {
    let (_d, mut s) = setup();
    let mut a = s.settings().unwrap();
    a.alerts.enabled = false;
    s.save_settings(&a).unwrap();
    publish(&mut s, &[record("disabled", now())]);
    assert_eq!(
        count(
            &s,
            "SELECT count(*) FROM activity_intent WHERE state='unavailable'"
        ),
        1
    );
    s.alert_choices(AlertChoices {
        enabled: true,
        previews: false,
        categories: [true; 5],
    })
    .unwrap();
    assert!(s.reserve_alert().unwrap().is_none());
    s.alert_permission(Permission::Denied).unwrap();
    publish(&mut s, &[record("denied", now() + 1)]);
    assert!(s.reserve_alert().unwrap().is_none());
    s.alert_permission(Permission::Granted).unwrap();
    assert!(s.reserve_alert().unwrap().is_none());
    publish(&mut s, &[record("future", now() + 1)]);
    assert_eq!(s.reserve_alert().unwrap().unwrap().alert.count, 1);
    assert_eq!(count(&s, "SELECT count(*) FROM seen WHERE seen=0"), 3);
}
#[test]
fn completed_batches_group_without_focus_suppression_and_previews_are_generic() {
    for size in [2, 17, 257] {
        let (_d, mut s) = setup();
        let rows = (0..size)
            .map(|n| record(&format!("item-{n:04}"), now()))
            .collect::<Vec<_>>();
        publish(&mut s, &rows);
        let r = s.reserve_alert().unwrap().unwrap();
        assert_eq!(r.alert.count, size);
        assert_eq!(count(&s, "SELECT count(*) FROM activity_intent"), 1);
        for secret in ["a/b", "other", "fixture payload", "#1"] {
            assert!(!r.alert.body.contains(secret));
        }
        let before = s.conn.total_changes();
        assert!(s.attempt_alert(&r).unwrap());
        assert!(s.conn.total_changes() - before <= 128);
        assert!(!s.attempt_alert(&r).unwrap());
        assert!(s.reserve_alert().unwrap().is_none());
        assert_eq!(
            count(&s, "SELECT count(*) FROM seen WHERE seen=0"),
            size as i64
        );
        assert_eq!(
            s.alert_destination(r.alert.intent).unwrap().unwrap().id,
            "item-0000"
        );
    }
}
#[test]
fn reservation_revalidates_seen_pause_permission_policy_generation_and_membership() {
    for race in [
        "seen",
        "pause",
        "permission",
        "category",
        "generation",
        "member",
    ] {
        let (_d, mut s) = setup();
        publish(&mut s, &[record("a", now()), record("b", now())]);
        let r = s.reserve_alert().unwrap().unwrap();
        match race {
            "seen" => {
                s.activity_seen(&["a".into()]).unwrap();
            }
            "pause" => {
                let mut a = s.settings().unwrap();
                a.paused = true;
                s.save_settings(&a).unwrap();
            }
            "permission" => s.alert_permission(Permission::Denied).unwrap(),
            "category" => s
                .alert_choices(AlertChoices {
                    enabled: true,
                    previews: false,
                    categories: [false, true, true, true, true],
                })
                .unwrap(),
            "generation" => {
                s.conn
                    .execute("UPDATE meta SET value=value+1 WHERE key='generation'", [])
                    .unwrap();
            }
            _ => {
                s.conn
                    .execute("DELETE FROM intent_member WHERE activity='a'", [])
                    .unwrap();
            }
        }
        assert!(!s.attempt_alert(&r).unwrap(), "{race}");
        assert_eq!(count(&s, "SELECT count(*) FROM alert_attempt"), 0);
    }
}
#[test]
fn failed_attempt_commit_cannot_dispatch_and_ambiguous_attempt_stays_terminal() {
    let (d, mut s) = setup();
    publish(&mut s, &[record("a", now())]);
    let r = s.reserve_alert().unwrap().unwrap();
    s.conn.execute_batch("CREATE TRIGGER fail_attempt BEFORE INSERT ON alert_attempt BEGIN SELECT RAISE(ABORT,'fault'); END;").unwrap();
    assert!(s.attempt_alert(&r).is_err());
    assert_eq!(
        count(
            &s,
            "SELECT count(*) FROM activity_intent WHERE state='pending'"
        ),
        1
    );
    s.conn.execute_batch("DROP TRIGGER fail_attempt;").unwrap();
    assert!(s.attempt_alert(&r).unwrap());
    drop(s);
    let mut s = Store::open(&d.path().join("db")).unwrap();
    s.activity_recovery(now()).unwrap();
    assert!(s.reserve_alert().unwrap().is_none());
    assert_eq!(count(&s, "SELECT count(*) FROM alert_attempt"), 1);
}
#[test]
fn reconstruction_is_bounded_idempotent_and_excludes_terminal_history() {
    let (d, mut s) = setup();
    let rows = (0..257)
        .map(|n| record(&format!("r-{n:04}"), now()))
        .collect::<Vec<_>>();
    publish(&mut s, &rows);
    s.conn.execute_batch("INSERT INTO activity_intent(kind,state,created) VALUES('pr','attempted',1),('pr','unavailable',1);").unwrap();
    s.activity_recovery(now()).unwrap();
    assert_eq!(
        count(
            &s,
            "SELECT count(*) FROM activity_intent WHERE state='recovery'"
        ),
        1
    );
    let before = s.conn.total_changes();
    assert!(!s.reconstruct_alerts().unwrap());
    assert!(s.conn.total_changes() - before <= 128);
    drop(s);
    let mut s = Store::open(&d.path().join("db")).unwrap();
    while !s.reconstruct_alerts().unwrap() {}
    assert_eq!(
        count(
            &s,
            "SELECT count(*) FROM progress WHERE key LIKE 'catchup:%'"
        ),
        257
    );
    assert_eq!(
        count(
            &s,
            "SELECT count(*) FROM activity_intent WHERE state IN ('attempted','unavailable')"
        ),
        2
    );
    s.activity_seen(&["r-0000".into()]).unwrap();
    s.conn
        .execute_batch(
            "UPDATE activity_target SET done=1; UPDATE activity_cycle SET notifications_done=1;",
        )
        .unwrap();
    s.activity_finish_recovery(false).unwrap();
    let r = s.reserve_alert().unwrap().unwrap();
    assert_eq!(r.alert.count, 256);
    assert_eq!(r.alert.title, "Missed activity");
    assert!(s.attempt_alert(&r).unwrap());
    assert!(s.alert_destination(r.alert.intent).unwrap().is_none());
    s.activity_finish_recovery(false).unwrap();
    assert_eq!(
        count(
            &s,
            "SELECT count(*) FROM activity_intent WHERE kind='summary'"
        ),
        1
    );
}
#[test]
fn notification_preview_includes_actor_context_and_message() {
    let (_d, mut s) = setup();
    publish(&mut s, &[record("a", now())]);
    s.alert_choices(AlertChoices {
        enabled: true,
        previews: true,
        categories: [true; 5],
    })
    .unwrap();
    s.conn
        .execute(
            "INSERT INTO content VALUES('pr:a/b:1',?,?2,?2)",
            params![r#"{"title":"Fix notification previews"}"#, now() as i64],
        )
        .unwrap();
    let a = s.reserve_alert().unwrap().unwrap().alert;
    assert_eq!(a.title, "New comment from other · a/b #1");
    assert_eq!(a.body, "Fix notification previews\nfixture payload");
}
#[test]
fn attempt_crash_child() {
    let Some(path) = std::env::var_os("GITJUKE_TEST_ALERT_DB") else {
        return;
    };
    let mut s = Store::open(std::path::Path::new(&path)).unwrap();
    let r = s.reserve_alert().unwrap().unwrap();
    s.attempt_alert(&r).unwrap();
    panic!("must be killed");
}
#[test]
fn actual_sigkill_before_and_after_attempt_commit() {
    use std::os::unix::process::ExitStatusExt;
    use std::{
        process::{Command, Stdio},
        time::{Duration, Instant},
    };
    for (phase, attempted) in [("attempt_before_commit", 0), ("attempt_after_commit", 1)] {
        let (d, mut s) = setup();
        publish(&mut s, &[record("a", now())]);
        drop(s);
        let marker = d.path().join("marker");
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "alerts::tests::attempt_crash_child",
                "--nocapture",
            ])
            .env("GITJUKE_TEST_ALERT_DB", d.path().join("db"))
            .env("GITJUKE_TEST_ACTIVITY_PHASE", phase)
            .env("GITJUKE_TEST_ACTIVITY_MARKER", &marker)
            .stdout(Stdio::null())
            .spawn()
            .unwrap();
        let start = Instant::now();
        while !marker.exists() {
            if start.elapsed() > Duration::from_secs(15) {
                let _ = child.kill();
                let _ = child.wait();
                panic!("barrier timeout")
            };
            assert!(child.try_wait().unwrap().is_none());
            std::thread::sleep(Duration::from_millis(10));
        }
        child.kill().unwrap();
        assert_eq!(child.wait().unwrap().signal(), Some(libc::SIGKILL));
        let mut s = Store::open(&d.path().join("db")).unwrap();
        assert_eq!(count(&s, "SELECT count(*) FROM alert_attempt"), attempted);
        s.activity_recovery(now()).unwrap();
        while !s.reconstruct_alerts().unwrap() {}
        assert_eq!(
            count(
                &s,
                "SELECT count(*) FROM progress WHERE key LIKE 'catchup:%'"
            ),
            1 - attempted
        );
    }
}
fn check(
    s: &mut Store,
    id: &str,
    run: Option<u64>,
    attempt: Option<u64>,
    suite: &str,
    status: &str,
) {
    let c = crate::sync::CheckContext {
        id: id.into(),
        kind: "CheckRun".into(),
        name: id.into(),
        status: "COMPLETED".into(),
        conclusion: Some("SUCCESS".into()),
        url: None,
        suite_id: Some(suite.into()),
        suite_status: Some(status.into()),
        suite_conclusion: Some("SUCCESS".into()),
        suite_updated_at: Some(crate::sync::date(now())),
        source_created_at: None,
        run_id: run,
        attempt,
        commit: "head".into(),
    };
    s.conn
        .execute(
            "INSERT OR REPLACE INTO check_context VALUES('P','head',?,?,0)",
            params![id, serde_json::to_string(&c).unwrap()],
        )
        .unwrap();
}
fn head(s: &mut Store) {
    s.conn
        .execute(
            "INSERT INTO watch_locator(id,repo,number,head) VALUES('P','a/b',1,'head')",
            [],
        )
        .unwrap();
    s.conn
        .execute(
            "INSERT INTO check_state VALUES('P','head',?,1,NULL,?)",
            params![now() as i64, now() as i64],
        )
        .unwrap();
}
#[test]
fn authoritative_workflow_attempts_coalesce_jobs_keep_runs_distinct_and_external_unknown() {
    let (_d, mut s) = setup();
    head(&mut s);
    check(&mut s, "job-a", Some(7), Some(1), "suite-a", "COMPLETED");
    check(&mut s, "job-b", Some(7), Some(1), "suite-a", "COMPLETED");
    check(&mut s, "job-c", Some(8), Some(1), "suite-b", "COMPLETED");
    check(&mut s, "external-a", None, None, "external", "COMPLETED");
    check(&mut s, "external-b", None, None, "external", "COMPLETED");
    check(&mut s, "missing", Some(9), None, "missing", "COMPLETED");
    check(&mut s, "zero", Some(10), Some(0), "zero", "COMPLETED");
    check(
        &mut s,
        "partial",
        Some(11),
        Some(1),
        "partial",
        "IN_PROGRESS",
    );
    let records = s.activity_check_records("a/b", 1, true, "head").unwrap();
    assert_eq!(records.len(), 3);
    assert_eq!(records.iter().filter(|a| a.eligible).count(), 2);
    assert_eq!(
        records
            .iter()
            .filter(|a| a.reason.contains("unavailable"))
            .count(),
        1
    );
    publish(&mut s, &records);
    let r = s.reserve_alert().unwrap().unwrap();
    assert_eq!(r.alert.count, 2);
    assert!(s.attempt_alert(&r).unwrap());
    check(&mut s, "job-a", Some(7), Some(2), "suite-a", "COMPLETED");
    check(&mut s, "job-b", Some(7), Some(2), "suite-a", "COMPLETED");
    let next = s.activity_check_records("a/b", 1, true, "head").unwrap();
    publish(&mut s, &next);
    assert_eq!(s.reserve_alert().unwrap().unwrap().alert.count, 1);
}
#[test]
fn head_and_retry_races_reject_reserved_terminal_results() {
    for race in ["head", "retry", "denial", "clear"] {
        let (_d, mut s) = setup();
        head(&mut s);
        check(&mut s, "job", Some(7), Some(1), "suite", "COMPLETED");
        let records = s.activity_check_records("a/b", 1, true, "head").unwrap();
        publish(&mut s, &records);
        let r = s.reserve_alert().unwrap().unwrap();
        match race {
            "head" => {
                s.conn
                    .execute("UPDATE watch_locator SET head='new'", [])
                    .unwrap();
            }
            "retry" => check(&mut s, "job", Some(7), Some(2), "suite", "IN_PROGRESS"),
            "denial" => {
                s.conn
                    .execute("INSERT INTO check_purge VALUES('P')", [])
                    .unwrap();
            }
            _ => {
                s.clear().unwrap();
            }
        }
        assert!(!s.attempt_alert(&r).unwrap(), "{race}");
        assert_eq!(count(&s, "SELECT count(*) FROM alert_attempt"), 0);
    }
}
#[test]
fn notification_destination_hides_expired_cleared_denied_and_unattempted_sources() {
    for state in ["unattempted", "expired", "clear", "denied", "purged"] {
        let (_d, mut s) = setup();
        publish(&mut s, &[record("a", now())]);
        let r = s.reserve_alert().unwrap().unwrap();
        if state != "unattempted" {
            assert!(s.attempt_alert(&r).unwrap());
        }
        match state {
            "expired" => {
                s.conn
                    .execute("UPDATE activity SET occurred=1", [])
                    .unwrap();
            }
            "clear" => s.clear().unwrap(),
            "purged" => s.activity_purge("a/b", 1).unwrap(),
            "denied" => {
                s.conn
                    .execute(
                        "INSERT INTO resource_state VALUES('conversation-denied:a/b:1',1,1,NULL)",
                        [],
                    )
                    .unwrap();
            }
            _ => {}
        }
        let destination = s.alert_destination(r.alert.intent).unwrap();
        if state == "clear" {
            let record = destination.unwrap();
            assert_eq!(record.id, "a");
            assert!(record.text.is_none());
            assert!(record.actor.is_none());
        } else {
            assert!(destination.is_none(), "{state}");
        }
        assert_eq!(count(&s, "SELECT count(*) FROM seen WHERE seen=0"), 1);
    }
}

#[test]
fn summary_and_unavailable_notification_routes_remain_distinct_and_do_not_mark_seen() {
    let (_d, mut s) = setup();
    publish(&mut s, &[record("a", now())]);
    let r = s.reserve_alert().unwrap().unwrap();
    assert!(s.attempt_alert(&r).unwrap());
    let route = s.notification_route(r.alert.intent).unwrap();
    assert!(!route.unavailable);
    assert_eq!(route.record.unwrap().id, "a");
    s.conn
        .execute("UPDATE activity SET occurred=1", [])
        .unwrap();
    assert!(s.notification_route(r.alert.intent).unwrap().unavailable);
    s.conn
        .execute(
            "UPDATE activity_intent SET kind='summary' WHERE id=?",
            [r.alert.intent],
        )
        .unwrap();
    let summary = s.notification_route(r.alert.intent).unwrap();
    assert!(!summary.unavailable);
    assert!(summary.record.is_none());
    assert_eq!(count(&s, "SELECT count(*) FROM seen WHERE seen=0"), 1);
}

#[test]
fn completed_recovery_groups_across_prs_while_post_boundary_activity_alerts_normally() {
    let (_d, mut s) = setup();
    let boundary = now();
    s.activity_recovery(boundary).unwrap();
    s.conn
        .execute("INSERT INTO activity_target VALUES('c/d',2,0,0)", [])
        .unwrap();
    for (repo, number, id) in [("a/b", 1, "missed-a"), ("c/d", 2, "missed-b")] {
        let mut item = record(id, boundary - 1);
        item.repo = repo.into();
        item.number = number;
        s.activity_stage(repo, number, &[item], true).unwrap();
        s.activity_publish(repo, number, false, &identity())
            .unwrap();
    }
    publish(&mut s, &[record("post-boundary", boundary + 1)]);
    s.activity_finish_recovery(false).unwrap();
    assert_eq!(
        count(
            &s,
            "SELECT count(*) FROM activity_intent WHERE kind='summary'"
        ),
        0
    );
    let normal = s.reserve_alert().unwrap().unwrap();
    assert_eq!(normal.alert.count, 1);
    assert_eq!(normal.alert.title, "New comment");
    assert!(s.attempt_alert(&normal).unwrap());
    s.conn
        .execute("UPDATE activity_cycle SET notifications_done=1", [])
        .unwrap();
    s.activity_finish_recovery(false).unwrap();
    let summary = s.reserve_alert().unwrap().unwrap();
    assert_eq!(summary.alert.count, 2);
    assert_eq!(summary.alert.title, "Missed activity");
    assert!(s.attempt_alert(&summary).unwrap());
    assert!(
        !s.notification_route(summary.alert.intent)
            .unwrap()
            .unavailable
    );
    assert_eq!(count(&s, "SELECT count(*) FROM seen WHERE seen=0"), 3);
}

#[test]
fn notification_preview_normalizes_and_truncates_unicode_with_ellipsis() {
    assert_eq!(preview_text("  hello\n\tworld\0  ", 30), "hello world");
    assert_eq!(
        preview_text("🦀".repeat(200).as_str(), 180),
        format!("{}…", "🦀".repeat(179))
    );
    assert_eq!(preview_text("exact", 5), "exact");
}

#[test]
fn notification_preview_groups_updates_without_hiding_the_excerpt() {
    let (_d, mut s) = setup();
    let mut settings = s.settings().unwrap();
    settings.alerts.previews = true;
    s.save_settings(&settings).unwrap();
    let mut first = record("a", now());
    first.text = Some("🦀".repeat(200));
    publish(&mut s, &[first, record("b", now())]);
    let alert = s.reserve_alert().unwrap().unwrap().alert;
    assert_eq!(alert.count, 2);
    assert!(alert.body.starts_with(&"🦀".repeat(179)));
    assert!(alert.body.ends_with("…\nAnd 1 more update"));
    assert!(alert.body.chars().count() <= 256);
}

#[test]
fn notification_preview_handles_empty_messages_and_disabled_previews() {
    for (kind, fallback) in [
        ("comment", "No message preview available."),
        ("request", "Your review was requested."),
        ("review", "No message preview available."),
        ("mention", "No message preview available."),
    ] {
        for previews in [false, true] {
            let (_d, mut s) = setup();
            let mut settings = s.settings().unwrap();
            settings.alerts.previews = previews;
            s.save_settings(&settings).unwrap();
            let mut item = record("a", now());
            item.kind = kind.into();
            item.text = Some(" \n\t".into());
            publish(&mut s, &[item]);
            let alert = s.reserve_alert().unwrap().unwrap().alert;
            if previews {
                assert_eq!(alert.body, fallback);
            } else {
                assert!(!alert.title.contains("other"));
                assert!(!alert.title.contains("a/b"));
                assert!(!alert.body.contains("other"));
                assert!(!alert.body.contains("a/b"));
            }
        }
    }
}
