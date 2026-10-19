use gitjuke_core::{model::StackState, runtime::Runtime, store::Store};
use std::{os::unix::fs::PermissionsExt, path::Path, time::Duration};
use tempfile::TempDir;
// Isolate independent app/gh fixtures while retaining each runtime's own
// concurrency and the existing state deadlines.
static WORKER_FIXTURES: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
fn setup() -> TempDir {
    let d = tempfile::tempdir().unwrap();
    let executable = d.path().join("gh");
    std::fs::write(&executable, include_str!("support/gh.py")).unwrap();
    std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
    let mut store = Store::open(&d.path().join("db")).unwrap();
    let mut settings = store.settings().unwrap();
    settings.gh_path = Some(executable.display().to_string());
    settings.config_dir = Some(d.path().join("config").display().to_string());
    store.save_settings(&settings).unwrap();
    d
}
async fn until(condition: impl FnMut() -> bool) {
    until_for(Duration::from_secs(25), condition).await;
}
async fn until_for(timeout: Duration, mut condition: impl FnMut() -> bool) {
    until_diagnosed(timeout, &mut condition, || {
        "runtime reached expected state".into()
    })
    .await;
}
async fn until_diagnosed(
    timeout: Duration,
    mut condition: impl FnMut() -> bool,
    diagnostic: impl FnOnce() -> String,
) {
    tokio::time::timeout(timeout, async {
        while !condition() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap_or_else(|e| panic!("{}: {e}", diagnostic()));
}
fn read_count(path: &Path) -> usize {
    std::fs::read_to_string(path.join("reads"))
        .unwrap_or_default()
        .lines()
        .count()
}
#[tokio::test]
async fn worker_to_sqlite_to_reader_and_identity_lock() {
    let _worker_fixture = WORKER_FIXTURES.lock().await;
    let d = setup();
    let r = Runtime::start(d.path().join("db"), |_| {}).unwrap();
    until(|| r.page(0).is_ok_and(|p| p.complete && p.items.len() == 1)).await;
    r.select("a/b".into(), 1).unwrap();
    until(|| {
        r.overview("a/b", 1)
            .is_ok_and(|s| s.data.is_some_and(|p| matches!(p.stack, StackState::None)))
    })
    .await;
    let detail = r.overview("a/b", 1).unwrap().data.unwrap();
    assert_eq!(detail.head, "abc");
    assert_eq!(detail.body.as_deref(), Some("<script>untrusted</script>"));
    std::fs::write(d.path().join("mode"), "mismatch").unwrap();
    r.refresh().unwrap();
    until(|| r.status().unwrap().locked).await;
    assert!(r.page(0).is_err());
    assert!(r.overview("a/b", 1).is_err());
    assert_eq!(r.status().unwrap().identity.unwrap().id, 42);
    r.shutdown().await;
}
#[tokio::test]
async fn persistent_pause_prevents_automatic_reads_but_allows_manual_sync() {
    let _worker_fixture = WORKER_FIXTURES.lock().await;
    let d = setup();
    let r = Runtime::start(d.path().join("db"), |_| {}).unwrap();
    until(|| r.page(0).is_ok_and(|p| p.complete)).await;
    r.set_pause(true).unwrap();
    until(|| !r.status().unwrap().busy).await;
    let before = read_count(d.path());
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(read_count(d.path()), before);
    r.shutdown().await;
    drop(r);
    let r = Runtime::start(d.path().join("db"), |_| {}).unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(r.status().unwrap().settings.paused);
    assert_eq!(read_count(d.path()), before);
    r.refresh().unwrap();
    until(|| !r.status().unwrap().locked && r.page(0).is_ok_and(|p| p.complete)).await;
    assert!(r.status().unwrap().settings.paused);
    assert!(read_count(d.path()) > before);
    r.shutdown().await;
}
fn scenario(d: &TempDir, value: serde_json::Value) {
    let tmp = d.path().join("scenario.tmp");
    std::fs::write(&tmp, serde_json::to_vec(&value).unwrap()).unwrap();
    std::fs::rename(tmp, d.path().join("scenario.json")).unwrap();
}

#[tokio::test]
async fn independent_reasons_selected_lifetime_and_request_removal() {
    let _worker_fixture = WORKER_FIXTURES.lock().await;
    let d = setup();
    let r = Runtime::start(d.path().join("db"), |_| {}).unwrap();
    until(|| {
        r.status().unwrap().discovery.len() == 4
            && r.status().unwrap().discovery.iter().all(|h| h.complete)
            && r.status().unwrap().watch_count == 3
    })
    .await;
    let watches = r.watches(0).unwrap();
    let first = watches.items.iter().find(|p| p.number == 1).unwrap();
    assert_eq!(first.reasons, vec!["authored", "direct", "team:T_1"]);
    r.select("a/b".into(), 4).unwrap();
    until(|| r.status().unwrap().watch_count == 4).await;
    r.reader_closed().unwrap();
    assert_eq!(r.status().unwrap().watch_count, 3);
    assert_eq!(r.status().unwrap().selected, Some(("a/b".into(), 4)));
    r.reader_opened().unwrap();
    assert_eq!(r.status().unwrap().watch_count, 4);
    scenario(&d, serde_json::json!({"direct_ids":[],"team_ids":[]}));
    r.refresh().unwrap();
    until(|| {
        r.watches(0).unwrap().items.iter().all(|w| {
            !w.reasons
                .iter()
                .any(|s| s == "direct" || s.starts_with("team:"))
        })
    })
    .await;
    let watches = r.watches(0).unwrap();
    assert_eq!(watches.items.len(), 2);
    assert!(
        watches
            .items
            .iter()
            .any(|p| p.number == 1 && p.reasons == vec!["authored"])
    );
    r.shutdown().await;
}
#[tokio::test]
async fn fifty_and_more_watches_rotate_without_a_membership_cap() {
    let _worker_fixture = WORKER_FIXTURES.lock().await;
    let d = setup();
    scenario(
        &d,
        serde_json::json!({"authored":151,"direct_ids":[],"team_ids":[],"no_teams":true}),
    );
    let r = Runtime::start(d.path().join("db"), |_| {}).unwrap();
    until(|| r.status().unwrap().watch_count == 151).await;
    until(|| {
        let a = r.watches(0).unwrap();
        let b = r.watches(100).unwrap();
        a.items.iter().chain(&b.items).all(|p| p.head.is_some())
    })
    .await;
    assert_eq!(r.watches(0).unwrap().items.len(), 100);
    assert_eq!(r.watches(100).unwrap().items.len(), 51);
    let reads = std::fs::read_to_string(d.path().join("reads")).unwrap();
    let batch_sizes = reads
        .lines()
        .filter_map(|l| {
            let v: serde_json::Value = serde_json::from_str(l).unwrap();
            v["variables"]["ids"].as_array().map(|a| a.len())
        })
        .collect::<Vec<_>>();
    assert!(batch_sizes.iter().all(|n| *n <= 10));
    assert!(batch_sizes.contains(&10));
    r.shutdown().await;
}
#[tokio::test]
async fn checks_paginate_mix_providers_and_preserve_attempt_and_merge_identity() {
    let _worker_fixture = WORKER_FIXTURES.lock().await;
    let d = setup();
    scenario(
        &d,
        serde_json::json!({"contexts":126,"merge":true,"attempt":2}),
    );
    let r = Runtime::start(d.path().join("db"), |_| {}).unwrap();
    // Complete 126 contexts on both commits while other watched PRs hydrate.
    // Functional traversal has a separate guard from native freshness budgets.
    until_diagnosed(
        Duration::from_secs(90),
        || {
            r.checks("a/b", 1, 0)
                .is_ok_and(|c| c.commits.len() == 2 && c.commits.iter().all(|m| m.complete))
        },
        || {
            serde_json::json!({"status":r.status().unwrap(),"checks":r.checks("a/b",1,0).unwrap()})
                .to_string()
        },
    )
    .await;
    let first = r.checks("a/b", 1, 0).unwrap();
    let next = r.checks("a/b", 1, first.next_offset.unwrap()).unwrap();
    assert_eq!(first.items.len(), 100);
    assert_eq!(next.items.len(), 100);
    assert!(first.items.iter().any(|c| c.kind == "StatusContext"));
    assert!(first.items.iter().any(|c| c.attempt == Some(2)));
    assert!(first.commits.iter().any(|c| c.resource == "merge-abc"));
    scenario(
        &d,
        serde_json::json!({"contexts":1,"head":"new-head","attempt":3}),
    );
    let sql = rusqlite::Connection::open(d.path().join("db")).unwrap();
    sql.execute("UPDATE watch_locator SET due=0", []).unwrap();
    r.reader_opened().unwrap();
    until(|| {
        r.checks("a/b", 1, 0).is_ok_and(|c| {
            c.head.as_deref() == Some("new-head")
                && c.items.len() == 1
                && c.items[0].attempt == Some(3)
        })
    })
    .await;
    assert_eq!(r.checks("a/b", 1, 0).unwrap().items[0].commit, "new-head");
    r.shutdown().await;
}
#[tokio::test]
async fn failed_team_discovery_preserves_prior_membership_without_locking_account() {
    let _worker_fixture = WORKER_FIXTURES.lock().await;
    let d = setup();
    let r = Runtime::start(d.path().join("db"), |_| {}).unwrap();
    until(|| r.status().unwrap().watch_count == 3 && r.status().unwrap().discovery.len() == 4)
        .await;
    scenario(&d, serde_json::json!({"deny_team":true}));
    r.refresh().unwrap();
    until(|| {
        r.status()
            .unwrap()
            .discovery
            .iter()
            .any(|h| h.resource.starts_with("team:") && h.error.is_some())
    })
    .await;
    assert!(!r.status().unwrap().locked);
    assert!(r.watches(0).unwrap().items.iter().any(|p| p.number == 3));
    r.shutdown().await;
}
#[tokio::test]
async fn search_partitions_more_than_a_thousand_requests_without_dropping_watches() {
    let _worker_fixture = WORKER_FIXTURES.lock().await;
    let d = setup();
    scenario(
        &d,
        serde_json::json!({"authored":0,"direct_count":1001,"no_teams":true}),
    );
    let r = Runtime::start(d.path().join("db"), |_| {}).unwrap();
    // Creation times span only 1,001 seconds, requiring deep search bisection.
    // This is a functional traversal guard; release latency is qualified separately.
    until_for(Duration::from_secs(90), || {
        r.status().unwrap().watch_count == 1001 && r.requested_page(0).is_ok_and(|p| p.complete)
    })
    .await;
    assert_eq!(r.requested_page(1000).unwrap().items.len(), 1);
    r.shutdown().await;
}
#[tokio::test]
async fn closing_reader_keeps_authored_and_requested_snapshot_work() {
    let _worker_fixture = WORKER_FIXTURES.lock().await;
    let d = setup();
    let r = Runtime::start(d.path().join("db"), |_| {}).unwrap();
    until(|| {
        r.watches(0)
            .is_ok_and(|p| p.items.len() == 3 && p.items.iter().all(|w| w.head.is_some()))
    })
    .await;
    r.reader_closed().unwrap();
    let before = read_count(d.path());
    let sql = rusqlite::Connection::open(d.path().join("db")).unwrap();
    sql.execute("UPDATE watch_locator SET due=0", []).unwrap();
    until(|| read_count(d.path()) > before + 1 && !r.status().unwrap().busy).await;
    assert_eq!(r.status().unwrap().watch_count, 3);
    r.shutdown().await;
}
#[tokio::test]
async fn denied_check_fields_purge_check_content_without_locking_or_false_empty_success() {
    let _worker_fixture = WORKER_FIXTURES.lock().await;
    let d = setup();
    let r = Runtime::start(d.path().join("db"), |_| {}).unwrap();
    until(|| r.checks("a/b", 1, 0).is_ok_and(|c| !c.items.is_empty())).await;
    scenario(&d, serde_json::json!({"deny_checks":true}));
    let sql = rusqlite::Connection::open(d.path().join("db")).unwrap();
    sql.execute("UPDATE watch_locator SET due=0", []).unwrap();
    until(|| {
        r.checks("a/b", 1, 0)
            .is_ok_and(|c| c.error.as_deref() == Some("Checks unavailable"))
    })
    .await;
    let c = r.checks("a/b", 1, 0).unwrap();
    assert!(c.items.is_empty());
    assert!(c.commits.iter().all(|c| !c.complete));
    assert!(!r.status().unwrap().locked);
    assert_eq!(r.status().unwrap().watch_count, 3);
    r.shutdown().await;
}
#[tokio::test]
async fn check_traversal_denials_purge_checks_without_losing_membership_or_activity() {
    let _worker_fixture = WORKER_FIXTURES.lock().await;
    for mode in ["deny_contexts", "deny_contexts_http"] {
        let d = setup();
        let r = Runtime::start(d.path().join("db"), |_| {}).unwrap();
        until(|| r.activity(0).is_ok_and(|a| a.baseline)).await;
        r.set_pause(true).unwrap();
        until(|| !r.status().unwrap().busy).await;
        let count = r.status().unwrap().watch_count;
        let original = r
            .activity(0)
            .unwrap()
            .items
            .into_iter()
            .map(|a| (a.id, a.seen))
            .collect::<Vec<_>>();
        assert!(!r.checks("a/b", 1, 0).unwrap().items.is_empty());
        let mut config = serde_json::json!({"contexts":26});
        config[mode] = true.into();
        scenario(&d, config);
        r.refresh().unwrap();
        until(|| {
            r.checks("a/b", 1, 0)
                .is_ok_and(|c| c.error.as_deref() == Some("Checks unavailable"))
        })
        .await;
        let denied = r.checks("a/b", 1, 0).unwrap();
        assert!(denied.items.is_empty());
        assert!(denied.commits.iter().all(|c| !c.complete));
        assert!(!r.status().unwrap().locked);
        assert!(!r.status().unwrap().storage_stopped);
        assert_eq!(r.status().unwrap().watch_count, count);
        assert_eq!(
            r.activity(0)
                .unwrap()
                .items
                .into_iter()
                .map(|a| (a.id, a.seen))
                .collect::<Vec<_>>(),
            original
        );
        assert!(
            r.activity(0)
                .unwrap()
                .items
                .iter()
                .filter(|a| a.kind == "check" && a.number == 1)
                .all(|a| a.text.is_none())
        );
        r.shutdown().await;
    }
}

#[tokio::test]
async fn missing_head_purges_full_pr_without_removing_watches_or_neighbor_cache() {
    let _worker_fixture = WORKER_FIXTURES.lock().await;
    let d = setup();
    let r = Runtime::start(d.path().join("db"), |_| {}).unwrap();
    until(|| r.activity(0).is_ok_and(|a| a.baseline)).await;
    r.select("a/b".into(), 1).unwrap();
    until(|| r.overview("a/b", 1).is_ok_and(|a| a.data.is_some())).await;
    r.set_pause(true).unwrap();
    until(|| !r.status().unwrap().busy).await;
    let before = r
        .activity(0)
        .unwrap()
        .items
        .into_iter()
        .map(|a| (a.id, a.seen))
        .collect::<Vec<_>>();
    let count = r.status().unwrap().watch_count;
    let sql = rusqlite::Connection::open(d.path().join("db")).unwrap();
    sql.execute("INSERT INTO content VALUES('api:repos/a/b/issues/10/comments?neighbor=1','{}',4070908800,4070908800)",[]).unwrap();
    scenario(&d, serde_json::json!({"missing_head":1}));
    r.refresh().unwrap();
    until(|| {
        r.checks("a/b", 1, 0)
            .is_ok_and(|a| a.error.as_deref() == Some("Resource unavailable"))
    })
    .await;
    assert!(r.overview("a/b", 1).unwrap().data.is_none());
    assert!(r.checks("a/b", 1, 0).unwrap().items.is_empty());
    assert!(!r.checks("a/b", 2, 0).unwrap().items.is_empty());
    assert_eq!(r.status().unwrap().watch_count, count);
    assert!(!r.status().unwrap().locked);
    assert_eq!(
        r.activity(0)
            .unwrap()
            .items
            .into_iter()
            .map(|a| (a.id, a.seen))
            .collect::<Vec<_>>(),
        before
    );
    assert_eq!(
        sql.query_row(
            "SELECT COUNT(*) FROM content WHERE key='api:repos/a/b/issues/10/comments?neighbor=1'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    r.shutdown().await;
}

#[tokio::test]
async fn overflowing_head_batches_reduce_without_losing_membership_or_repeating_same_batch() {
    let _worker_fixture = WORKER_FIXTURES.lock().await;
    let d = setup();
    scenario(
        &d,
        serde_json::json!({"authored":20,"direct_ids":[],"no_teams":true,"overflow_batches":true}),
    );
    let r = Runtime::start(d.path().join("db"), |_| {}).unwrap();
    // Include adaptive retries and competing Activity hydration for all 20 watches.
    // This is functional completion; native freshness budgets have separate gates.
    until_diagnosed(
        Duration::from_secs(90),
        || {
            r.watches(0)
                .is_ok_and(|p| p.items.len() == 20 && p.items.iter().all(|w| w.head.is_some()))
        },
        || {
            serde_json::json!({"status":r.status().unwrap(),"watches":r.watches(0).unwrap()})
                .to_string()
        },
    )
    .await;
    let reads = std::fs::read_to_string(d.path().join("reads")).unwrap();
    let sizes = reads
        .lines()
        .filter_map(|l| {
            let v: serde_json::Value = serde_json::from_str(l).unwrap();
            v["variables"]["ids"].as_array().map(|a| a.len())
        })
        .collect::<Vec<_>>();
    assert!(sizes.contains(&10));
    assert!(sizes.contains(&5));
    assert!(sizes.contains(&2));
    assert_eq!(sizes.iter().filter(|n| **n == 10).count(), 1);
    r.shutdown().await;
}

fn activity_comment(n: u64, mention: bool) -> serde_json::Value {
    serde_json::json!({"node_id":format!("IC_{n}"),"id":n,"created_at":gitjuke_core::sync::date(gitjuke_core::now()+2),"body":if mention {"@reader"} else {"hello"},"body_html":if mention {"<a class=\"user-mention\" href=\"https://github.com/reader\">@reader</a>"} else {"<p>hello</p>"},"html_url":format!("https://github.com/a/b/pull/1#issuecomment-{n}"),"user":{"id":43,"login":"other","type":"User"}})
}
#[tokio::test]
async fn notification_to_individual_activity_pagination_baseline_seen_pause_and_recovery() {
    let _worker_fixture = WORKER_FIXTURES.lock().await;
    let d = setup();
    let comments = (1..=51)
        .map(|n| activity_comment(n, n == 51))
        .collect::<Vec<_>>();
    scenario(
        &d,
        serde_json::json!({"comments":comments,"poll_interval":600,"notifications":[{"reason":"mention","repository":{"full_name":"a/b"},"subject":{"type":"PullRequest","url":"https://api.github.com/repos/a/b/pulls/1"}}]}),
    );
    let started = gitjuke_core::now();
    let r = Runtime::start(d.path().join("db"), |_| {}).unwrap();
    // Functional pagination includes several sources; keep its guard separate
    // from release latency, and the server poll window longer than this workflow.
    until_for(Duration::from_secs(60), || {
        r.activity(0).is_ok_and(|a| a.baseline)
    })
    .await;
    let a = r.activity(0).unwrap();
    assert!(a.items.iter().any(|a| a.kind == "mention"));
    assert!(a.items.iter().all(|a| a.seen));
    assert!(a.next_poll_at >= started + 600);
    let notifications = std::fs::read_to_string(d.path().join("reads"))
        .unwrap()
        .lines()
        .filter(|l| l.contains("notifications?"))
        .count();
    r.set_pause(true).unwrap();
    until(|| !r.status().unwrap().busy).await;
    let mut new = comments.clone();
    new.push(activity_comment(52, false));
    scenario(&d, serde_json::json!({"comments":new,"poll_interval":600}));
    r.refresh_activity("a/b", 1).unwrap();
    until_for(Duration::from_secs(60), || {
        r.activity(0)
            .is_ok_and(|a| a.items.iter().any(|r| r.id.ends_with("IC_52")))
    })
    .await;
    let item = r
        .activity(0)
        .unwrap()
        .items
        .into_iter()
        .find(|r| r.id.ends_with("IC_52"))
        .unwrap();
    assert!(!item.seen);
    assert!(r.status().unwrap().settings.paused);
    r.mark_activity_seen(std::slice::from_ref(&item.id))
        .unwrap();
    r.set_pause(false).unwrap();
    until(|| r.activity(0).is_ok_and(|a| a.pending == 0)).await;
    assert!(
        r.activity(0)
            .unwrap()
            .items
            .iter()
            .find(|a| a.id == item.id)
            .unwrap()
            .seen
    );
    let after = std::fs::read_to_string(d.path().join("reads"))
        .unwrap()
        .lines()
        .filter(|l| l.contains("notifications?"))
        .count();
    assert_eq!(
        notifications, after,
        "Resume must preserve the server poll deadline"
    );
    assert!(
        r.activity(0).unwrap().recovery.is_some(),
        "No completed recovery claim before the notification deadline"
    );
    r.shutdown().await;
}
#[tokio::test]
async fn partial_activity_denial_keeps_progress_and_purges_payloads() {
    let _worker_fixture = WORKER_FIXTURES.lock().await;
    let d = setup();
    scenario(
        &d,
        serde_json::json!({"comments":[activity_comment(1,false)]}),
    );
    let r = Runtime::start(d.path().join("db"), |_| {}).unwrap();
    until(|| r.activity(0).is_ok_and(|a| a.baseline)).await;
    let before = r.activity(0).unwrap();
    assert!(!before.items.is_empty());
    r.set_pause(true).unwrap();
    until(|| !r.status().unwrap().busy).await;
    let sql = rusqlite::Connection::open(d.path().join("db")).unwrap();
    let neighbor = "api:repos/a/b/issues/10/comments?per_page=50&page=1";
    sql.execute(
        "INSERT INTO content VALUES(?,'{}',?,?)",
        rusqlite::params![
            neighbor,
            gitjuke_core::now() as i64,
            gitjuke_core::now() as i64
        ],
    )
    .unwrap();
    scenario(
        &d,
        serde_json::json!({"comments":[activity_comment(2,false)],"deny_activity":"reviews"}),
    );
    r.refresh_activity("a/b", 1).unwrap();
    until(|| r.activity(0).is_ok_and(|a| a.error.is_some())).await;
    let page = r.activity(0).unwrap();
    assert!(
        !page.items.iter().any(|a| a.id.ends_with("IC_2")),
        "Incomplete traversals cannot publish new activity"
    );
    assert!(
        page.items
            .iter()
            .filter(|a| a.repo == "a/b" && a.number == 1)
            .all(|a| a.text.is_none())
    );
    assert!(!r.status().unwrap().locked);
    assert!(r.status().unwrap().settings.paused);
    assert_eq!(
        sql.query_row(
            "SELECT COUNT(*) FROM content WHERE key=?",
            [neighbor],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    r.shutdown().await;
}

#[tokio::test]
async fn conditional_hydration_reuses_exact_url_payload_without_replaying_activity() {
    let _worker_fixture = WORKER_FIXTURES.lock().await;
    let d = setup();
    scenario(
        &d,
        serde_json::json!({"validators":true,"comments":[activity_comment(1,false)]}),
    );
    let r = Runtime::start(d.path().join("db"), |_| {}).unwrap();
    until(|| r.activity(0).is_ok_and(|a| a.baseline)).await;
    r.set_pause(true).unwrap();
    until(|| !r.status().unwrap().busy).await;
    let count = r.activity(0).unwrap().items.len();
    let before = read_count(d.path());
    r.refresh_activity("a/b", 1).unwrap();
    until(|| read_count(d.path()) > before + 14 && !r.status().unwrap().busy).await;
    assert_eq!(count, r.activity(0).unwrap().items.len());
    assert!(r.activity(0).unwrap().error.is_none());
    let reads = std::fs::read_to_string(d.path().join("reads")).unwrap();
    assert!(reads.contains("If-None-Match:"));
    // An unchanged PR overview must not prevent a changed comments page
    // from being hydrated and published during the same traversal.
    scenario(
        &d,
        serde_json::json!({"validators":true,"comments":[activity_comment(1,false),activity_comment(2,false)]}),
    );
    r.refresh_activity("a/b", 1).unwrap();
    until(|| {
        r.activity(0)
            .is_ok_and(|a| a.items.iter().any(|item| item.id.ends_with("IC_2")))
    })
    .await;
    assert!(r.activity(0).unwrap().error.is_none());
    r.shutdown().await;
}
#[tokio::test]
async fn workflow_attempts_and_legacy_statuses_remain_individual_and_grouped() {
    let _worker_fixture = WORKER_FIXTURES.lock().await;
    let d = setup();
    let time = gitjuke_core::sync::date(gitjuke_core::now() + 2);
    scenario(
        &d,
        serde_json::json!({"contexts":4,"check_time":time,"merge":true,"direct_ids":[],"no_teams":true}),
    );
    let r = Runtime::start(d.path().join("db"), |_| {}).unwrap();
    until(|| {
        let a = r.activity(0);
        a.is_ok_and(|a| a.baseline && a.items.iter().filter(|r| r.kind == "check").count() == 8)
    })
    .await;
    assert!(r.activity(0).unwrap().items.iter().all(|r| r.seen));
    scenario(
        &d,
        serde_json::json!({"contexts":4,"check_time":time,"attempt":2,"merge":true,"direct_ids":[],"no_teams":true}),
    );
    r.refresh().unwrap();
    until(|| {
        r.checks("a/b", 1, 0)
            .is_ok_and(|c| c.items.iter().any(|r| r.attempt == Some(2)))
    })
    .await;
    r.refresh_activity("a/b", 1).unwrap();
    until(|| {
        r.activity(0).is_ok_and(|a| {
            a.items
                .iter()
                .filter(|r| {
                    r.id.split(":workflow:")
                        .nth(1)
                        .and_then(|s| s.split(':').nth(1))
                        == Some("2")
                })
                .count()
                == 4
        })
    })
    .await;
    let activity = r.activity(0).unwrap();
    assert!(
        activity
            .items
            .iter()
            .filter(|r| r.kind == "check")
            .all(|r| r.head.as_deref() == Some("abc"))
    );
    assert_eq!(
        activity
            .items
            .iter()
            .filter(|r| r.id.contains("status:"))
            .count(),
        4
    );
    r.shutdown().await;
}
