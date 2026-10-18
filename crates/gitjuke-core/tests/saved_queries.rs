use gitjuke_core::{runtime::Runtime, store::Store};
use serde_json::{Value, json};
use std::{os::unix::fs::PermissionsExt, time::Duration};
use tempfile::TempDir;
fn scenario(d: &TempDir, v: Value) {
    let p = d.path().join("scenario.tmp");
    std::fs::write(&p, serde_json::to_vec(&v).unwrap()).unwrap();
    std::fs::rename(p, d.path().join("scenario.json")).unwrap();
}
fn config() -> Value {
    json!({"authored":0,"no_teams":true,"direct_ids":[],"query_ids":(1..=150).collect::<Vec<_>>()})
}
async fn until(mut f: impl FnMut() -> bool) {
    tokio::time::timeout(Duration::from_secs(40), async {
        while !f() {
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .expect("query fixture state");
}
async fn start() -> (TempDir, Runtime) {
    let d = tempfile::tempdir().unwrap();
    let gh = d.path().join("gh");
    std::fs::write(&gh, include_str!("support/gh.py")).unwrap();
    std::fs::set_permissions(&gh, std::fs::Permissions::from_mode(0o700)).unwrap();
    scenario(&d, config());
    let mut s = Store::open(&d.path().join("db")).unwrap();
    let mut settings = s.settings().unwrap();
    settings.paused = true;
    settings.gh_path = Some(gh.display().to_string());
    settings.config_dir = Some(d.path().join("config").display().to_string());
    s.save_settings(&settings).unwrap();
    drop(s);
    let r = Runtime::start(d.path().join("db"), |_| {}).unwrap();
    r.refresh().unwrap();
    until(|| {
        r.status()
            .is_ok_and(|s| !s.locked && !s.busy && s.queued == 0)
    })
    .await;
    (d, r)
}
async fn read(r: &Runtime, q: &gitjuke_core::saved_queries::SavedQuery, page: u32) {
    r.request_query(q.id, q.version, page).unwrap();
    until(|| {
        r.query_page(q.id, q.version, page).is_ok_and(|p| {
            p.fetched_at.is_some() && p.error.as_ref().is_none_or(|e| !e.starts_with("Queued"))
        })
    })
    .await;
}
#[test]
fn crud_restart_compare_and_swap_and_clear_preserve_configuration() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("db");
    let mut s = Store::open(&p).unwrap();
    assert!(s.save_query(None, None, " ", "repo:a/b").is_err());
    let q = s.save_query(None, None, " Review ", " repo:a/b ").unwrap();
    assert_eq!(q.name, "Review");
    let q2 = s
        .save_query(Some(q.id), Some(q.version), "Renamed", "is:closed")
        .unwrap();
    assert!(
        s.save_query(Some(q.id), Some(q.version), "Stale", "is:open")
            .is_err()
    );
    assert!(s.delete_query(q.id, q.version).is_err());
    let mut settings = s.settings().unwrap();
    settings.paused = true;
    settings.gh_path = Some("/explicit/gh".into());
    s.save_settings(&settings).unwrap();
    s.clear().unwrap();
    drop(s);
    let mut s = Store::open(&p).unwrap();
    assert_eq!(s.saved_query(q.id).unwrap(), q2);
    assert!(s.settings().unwrap().paused);
    assert_eq!(
        s.settings().unwrap().gh_path.as_deref(),
        Some("/explicit/gh")
    );
    s.delete_query(q2.id, q2.version).unwrap();
    let q3 = s.save_query(None, None, "Next", "is:open").unwrap();
    assert_ne!(q3.id, q.id);
    drop(s);
    assert_eq!(
        Store::open(&p)
            .unwrap()
            .saved_queries(0)
            .unwrap()
            .items
            .len(),
        1
    );
}
#[test]
fn navigation_pages_and_upgrade_from_14() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("db");
    drop(Store::open(&p).unwrap());
    let c = rusqlite::Connection::open(&p).unwrap();
    c.execute_batch(
        "DROP INDEX activity_intent_dispatch; DROP TABLE saved_query; PRAGMA user_version=14;",
    )
    .unwrap();
    drop(c);
    let mut s = Store::open(&p).unwrap();
    for n in 0..101 {
        s.save_query(None, None, &format!("Query {n}"), "is:open")
            .unwrap();
    }
    assert_eq!(s.saved_queries(0).unwrap().items.len(), 100);
    assert_eq!(s.saved_queries(0).unwrap().next_offset, Some(100));
    assert_eq!(s.saved_queries(100).unwrap().items.len(), 1);
}
#[tokio::test]
async fn pagination_counts_restart_empty_and_no_watch_enrollment() {
    let (d, r) = start().await;
    let q = r
        .save_query(None, None, "Local".into(), "repo:a/b".into())
        .unwrap();
    r.activate_query(Some(q.id), Some(q.version)).unwrap();
    read(&r, &q, 1).await;
    let p = r.query_page(q.id, q.version, 1).unwrap();
    assert_eq!(p.items.len(), 100);
    assert_eq!(p.cached_count, 100);
    assert_eq!(p.total, Some(150));
    assert!(!p.complete);
    assert_eq!(p.next_page, Some(2));
    read(&r, &q, 2).await;
    let p = r.query_page(q.id, q.version, 2).unwrap();
    assert_eq!(p.items.len(), 50);
    assert_eq!(p.cached_count, 150);
    assert!(p.complete);
    assert_eq!(r.status().unwrap().watch_count, 0);
    r.shutdown().await;
    let r = Runtime::start(d.path().join("db"), |_| {}).unwrap();
    assert!(r.query_page(q.id, q.version, 1).is_err());
    r.refresh().unwrap();
    until(|| r.status().is_ok_and(|s| !s.locked)).await;
    assert_eq!(r.query_page(q.id, q.version, 2).unwrap().items.len(), 50);
    let mut v = config();
    v["query_ids"] = json!([]);
    scenario(&d, v);
    read(&r, &q, 1).await;
    until(|| r.query_page(q.id, q.version, 1).unwrap().total == Some(0)).await;
    let p = r.query_page(q.id, q.version, 1).unwrap();
    assert!(p.complete);
    assert_eq!(p.cached_count, 0);
    assert!(r.query_page(q.id, q.version, 2).unwrap().items.is_empty());
    r.shutdown().await;
}
#[tokio::test]
async fn partial_denied_and_search_ceiling_are_honest() {
    let (d, r) = start().await;
    let q = r
        .save_query(None, None, "Local".into(), "repo:a/b".into())
        .unwrap();
    let mut v = config();
    v["query_partial"] = json!(true);
    v["query_total"] = json!(1001);
    scenario(&d, v);
    read(&r, &q, 1).await;
    let p = r.query_page(q.id, q.version, 1).unwrap();
    assert!(!p.complete);
    assert_eq!(p.items.len(), 100);
    assert!(p.error.unwrap().contains("partial"));
    let neighbor = r
        .save_query(None, None, "Neighbor".into(), "repo:a/b".into())
        .unwrap();
    read(&r, &neighbor, 1).await;
    let sql = rusqlite::Connection::open(d.path().join("db")).unwrap();
    let before: i64 = sql
        .query_row(
            "SELECT CAST(value AS INTEGER) FROM meta WHERE key='generation'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let mut v = config();
    v["query_denied"] = json!(true);
    scenario(&d, v);
    r.request_query(q.id, q.version, 1).unwrap();
    until(|| {
        r.query_page(q.id, q.version, 1)
            .is_ok_and(|p| p.error.is_some_and(|e| e.contains("unavailable")))
    })
    .await;
    let p = r.query_page(q.id, q.version, 1).unwrap();
    assert!(p.items.is_empty());
    assert!(!p.complete);
    assert_eq!(p.total, None);
    assert_eq!(
        r.query_page(neighbor.id, neighbor.version, 1)
            .unwrap()
            .items
            .len(),
        100,
        "query denial must preserve unrelated query cache"
    );
    assert_eq!(
        sql.query_row(
            "SELECT CAST(value AS INTEGER) FROM meta WHERE key='generation'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        before,
        "query denial is scoped"
    );
    r.shutdown().await;
}
#[tokio::test]
async fn edits_and_deletion_during_fetch_reject_late_results() {
    let (d, r) = start().await;
    let q = r
        .save_query(None, None, "Before".into(), "repo:a/b".into())
        .unwrap();
    let mut v = config();
    v["slow_query"] = json!(0.8);
    scenario(&d, v);
    r.request_query(q.id, q.version, 1).unwrap();
    until(|| d.path().join("query-started").exists()).await;
    let edited = r
        .save_query(
            Some(q.id),
            Some(q.version),
            "After".into(),
            "is:closed".into(),
        )
        .unwrap();
    until(|| r.status().is_ok_and(|s| !s.busy && s.queued == 0)).await;
    assert!(r.query_page(q.id, q.version, 1).is_err());
    let sql = rusqlite::Connection::open(d.path().join("db")).unwrap();
    assert_eq!(
        sql.query_row(
            "SELECT COUNT(*) FROM content WHERE key LIKE 'query:%'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0,
        "obsolete results must never commit, even invisibly"
    );
    assert!(
        r.query_page(edited.id, edited.version, 1)
            .unwrap()
            .items
            .is_empty()
    );
    std::fs::remove_file(d.path().join("query-started")).unwrap();
    r.request_query(edited.id, edited.version, 1).unwrap();
    until(|| d.path().join("query-started").exists()).await;
    r.delete_query(edited.id, edited.version).unwrap();
    until(|| r.status().is_ok_and(|s| !s.busy && s.queued == 0)).await;
    assert!(r.query_page(edited.id, edited.version, 1).is_err());
    assert!(r.saved_queries(0).unwrap().items.is_empty());
    r.shutdown().await;
}
#[tokio::test]
async fn clear_and_identity_change_reject_inflight_query() {
    let (d, r) = start().await;
    let q = r
        .save_query(None, None, "Local".into(), "repo:a/b".into())
        .unwrap();
    let mut v = config();
    v["slow_query"] = json!(0.8);
    scenario(&d, v);
    r.request_query(q.id, q.version, 1).unwrap();
    until(|| d.path().join("query-started").exists()).await;
    r.clear().unwrap();
    until(|| r.status().is_ok_and(|s| !s.busy)).await;
    assert!(r.query_page(q.id, q.version, 1).unwrap().items.is_empty());
    let mut v = config();
    v["query_mismatch"] = json!(true);
    scenario(&d, v);
    r.request_query(q.id, q.version, 1).unwrap();
    until(|| r.status().is_ok_and(|s| s.locked)).await;
    assert!(r.query_page(q.id, q.version, 1).is_err());
    r.shutdown().await;
}

#[tokio::test]
async fn transient_failure_retains_cache_quota_blocks_search_and_inactive_queries_do_not_run() {
    let (d, r) = start().await;
    let q = r
        .save_query(None, None, "Local".into(), "repo:a/b".into())
        .unwrap();
    read(&r, &q, 1).await;
    let mut v = config();
    v["query_failure"] = json!(true);
    scenario(&d, v);
    r.request_query(q.id, q.version, 1).unwrap();
    until(|| {
        r.query_page(q.id, q.version, 1)
            .is_ok_and(|p| p.error.is_some_and(|e| e.contains("HTTP 500")))
    })
    .await;
    assert_eq!(r.query_page(q.id, q.version, 1).unwrap().items.len(), 100);
    let mut v = config();
    v["query_rate"] = json!(true);
    scenario(&d, v);
    r.request_query(q.id, q.version, 1).unwrap();
    until(|| {
        r.status().is_ok_and(|s| {
            s.budgets
                .iter()
                .any(|b| b.resource == "search" && b.blocked_until > gitjuke_core::now())
        })
    })
    .await;
    r.request_query(q.id, q.version, 1).unwrap();
    until(|| r.status().is_ok_and(|s| !s.busy)).await;
    let before = std::fs::read_to_string(d.path().join("reads"))
        .unwrap()
        .lines()
        .filter(|l| l.contains("search/issues") && !l.contains("user-review-requested"))
        .count();
    tokio::time::sleep(Duration::from_millis(1200)).await;
    let after = std::fs::read_to_string(d.path().join("reads"))
        .unwrap()
        .lines()
        .filter(|l| l.contains("search/issues") && !l.contains("user-review-requested"))
        .count();
    assert_eq!(before, after);
    assert_eq!(r.query_page(q.id, q.version, 1).unwrap().items.len(), 100);
    r.shutdown().await;
    // A new, inactive runtime does not start an automatic Search for local definitions.
    let (d, r) = start().await;
    r.save_query(None, None, "Inactive".into(), "repo:a/b".into())
        .unwrap();
    r.set_pause(false).unwrap();
    until(|| r.status().is_ok_and(|s| !s.busy && s.queued == 0)).await;
    let reads = std::fs::read_to_string(d.path().join("reads")).unwrap();
    assert!(
        !reads
            .lines()
            .any(|l| l.contains("search/issues") && !l.contains("user-review-requested"))
    );
    r.shutdown().await;
}

#[tokio::test]
async fn oversized_page_is_limited_without_clipping_or_automatic_retry() {
    let (d, r) = start().await;
    let q = r
        .save_query(None, None, "Local".into(), "repo:a/b".into())
        .unwrap();
    let mut v = config();
    v["query_ids"] = json!([1]);
    v["query_title_bytes"] = json!(900 * 1024);
    scenario(&d, v);
    r.activate_query(Some(q.id), Some(q.version)).unwrap();
    r.request_query(q.id, q.version, 1).unwrap();
    until(|| {
        r.query_page(q.id, q.version, 1)
            .is_ok_and(|p| p.error.is_some_and(|e| e.contains("IPC budget")))
    })
    .await;
    assert!(r.query_page(q.id, q.version, 1).unwrap().items.is_empty());
    let c = rusqlite::Connection::open(d.path().join("db")).unwrap();
    assert_eq!(
        c.query_row("SELECT limited FROM saved_query WHERE id=?", [q.id], |r| {
            r.get::<_, i64>(0)
        })
        .unwrap(),
        1
    );
    c.execute("UPDATE due SET at=0 WHERE key='saved-query'", [])
        .unwrap();
    r.set_pause(false).unwrap();
    until(|| r.status().is_ok_and(|s| !s.busy && s.queued == 0)).await;
    let reads = std::fs::read_to_string(d.path().join("reads")).unwrap();
    assert_eq!(
        reads
            .lines()
            .filter(|l| l.contains("search/issues") && !l.contains("user-review-requested"))
            .count(),
        1
    );
    r.shutdown().await;
}
