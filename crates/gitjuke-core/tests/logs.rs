use gitjuke_core::{
    logs::{LOG_BYTES, LogJob, LogRead},
    runtime::Runtime,
    store::Store,
};
use serde_json::{Value, json};
use std::{os::unix::fs::PermissionsExt, time::Duration};
use tempfile::TempDir;
fn config() -> Value {
    json!({"head":"a".repeat(40),"no_teams":true,"direct_ids":[],"actions_logs":true})
}
fn scenario(d: &TempDir, v: Value) {
    let p = d.path().join("scenario.tmp");
    std::fs::write(&p, serde_json::to_vec(&v).unwrap()).unwrap();
    std::fs::rename(p, d.path().join("scenario.json")).unwrap();
}
async fn until(mut f: impl FnMut() -> bool) {
    tokio::time::timeout(Duration::from_secs(25), async {
        while !f() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
}
fn job() -> LogJob {
    LogJob {
        repo: "a/b".into(),
        number: 1,
        head: "a".repeat(40),
        check: "CR_0".into(),
        offset: 0,
        seq: 0,
    }
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
    until(|| r.status().is_ok_and(|s| !s.locked && !s.busy)).await;
    r.select("a/b".into(), 1).unwrap();
    until(|| {
        r.checks("a/b", 1, 0)
            .is_ok_and(|p| !p.items.is_empty() && p.commits.iter().all(|c| c.complete))
    })
    .await;
    (d, r)
}
async fn result(r: &Runtime, seq: u64) -> LogRead {
    until(|| {
        r.read_logs(seq)
            .is_ok_and(|v| v.state != "queued" && v.state != "reading")
    })
    .await;
    r.read_logs(seq).unwrap()
}
#[tokio::test]
async fn explicit_memory_only_read_release_and_provider_details() {
    let (d, r) = start().await;
    let trace = std::fs::read_to_string(d.path().join("reads")).unwrap();
    assert!(!trace.contains("/logs"));
    let mut external = job();
    external.check = "SC_1".into();
    assert!(r.request_logs(external.clone()).is_err());
    assert_eq!(
        r.check_details(&external).unwrap(),
        "https://ci.example/check"
    );
    let seq = r.request_logs(job()).unwrap();
    let v = result(&r, seq).await;
    assert_eq!(v.state, "complete");
    assert!(v.text.unwrap().contains("private-log-canary"));
    r.cancel_logs(seq).unwrap();
    assert_eq!(r.read_logs(seq).unwrap().state, "released");
    r.shutdown().await;
    for p in std::fs::read_dir(d.path()).unwrap().flatten() {
        if p.file_name().to_string_lossy().starts_with("db") {
            assert!(
                !std::fs::read(p.path())
                    .unwrap()
                    .windows(b"private-log-canary".len())
                    .any(|w| w == b"private-log-canary")
            );
        }
    }
}
#[tokio::test]
async fn exact_one_over_and_error_classification() {
    let (d, r) = start().await;
    for (status, n, expected) in [
        (200, 0, "empty"),
        (200, LOG_BYTES, "complete"),
        (200, LOG_BYTES + 1, "oversized"),
        (403, 5, "denied"),
        (404, 5, "missing"),
        (410, 5, "expired"),
        (500, 5, "failed"),
    ] {
        let mut c = config();
        c["log_status"] = json!(status);
        c["log_bytes"] = json!(n);
        scenario(&d, c);
        let seq = r.request_logs(job()).unwrap();
        let v = result(&r, seq).await;
        assert_eq!(v.state, expected, "status {status} bytes {n}");
        assert_eq!(
            v.text.as_ref().map(String::len),
            if expected == "complete" {
                Some(n)
            } else {
                None
            }
        );
        r.cancel_logs(seq).unwrap();
    }
    r.shutdown().await;
}
#[tokio::test]
async fn navigation_cancels_child_and_old_cancel_cannot_release_replacement() {
    let (d, r) = start().await;
    let mut c = config();
    c["slow_log"] = json!(8);
    scenario(&d, c);
    let old = r.request_logs(job()).unwrap();
    until(|| d.path().join("log-started").exists()).await;
    r.cancel_logs(old).unwrap();
    assert_eq!(r.read_logs(old).unwrap().state, "released");
    scenario(&d, config());
    let new = r.request_logs(job()).unwrap();
    r.cancel_logs(old).unwrap();
    assert_eq!(result(&r, new).await.state, "complete");
    r.select("a/b".into(), 2).unwrap();
    assert_eq!(r.read_logs(new).unwrap().state, "released");
    r.shutdown().await;
}
#[tokio::test]
async fn head_attempt_and_account_races_never_publish_body() {
    for change in [json!({"head":"b".repeat(40)}), json!({"remote_attempt":2})] {
        let (d, r) = start().await;
        let mut c = config();
        c["change_during_log"] = change;
        scenario(&d, c);
        let seq = r.request_logs(job()).unwrap();
        let v = result(&r, seq).await;
        assert_ne!(v.state, "complete");
        assert!(v.text.is_none());
        r.shutdown().await;
    }
    let (d, r) = start().await;
    let mut c = config();
    c["slow_log"] = json!(0.5);
    scenario(&d, c);
    let seq = r.request_logs(job()).unwrap();
    until(|| d.path().join("log-started").exists()).await;
    std::fs::write(d.path().join("mode"), "mismatch").unwrap();
    let v = result(&r, seq).await;
    assert_ne!(v.state, "complete");
    assert!(v.text.is_none());
    assert!(r.status().unwrap().locked);
    r.shutdown().await;
}
#[tokio::test]
async fn clear_close_configuration_and_transport_overflow_release() {
    for action in ["clear", "close", "configure"] {
        let (d, r) = start().await;
        let seq = r.request_logs(job()).unwrap();
        assert_eq!(result(&r, seq).await.state, "complete");
        match action {
            "clear" => r.clear().unwrap(),
            "close" => r.reader_closed().unwrap(),
            _ => r
                .configure(
                    d.path().join("gh").display().to_string(),
                    d.path().join("config").display().to_string(),
                )
                .unwrap(),
        };
        assert_eq!(r.read_logs(seq).unwrap().state, "released");
        r.shutdown().await;
    }
    let (d, r) = start().await;
    let mut c = config();
    c["log_bytes"] = json!(16 * 1024 * 1024 + 1);
    scenario(&d, c);
    let seq = r.request_logs(job()).unwrap();
    assert_eq!(result(&r, seq).await.state, "oversized");
    r.shutdown().await;
}

#[tokio::test]
async fn resource_epoch_backoff_provider_and_persistence_guards() {
    let (d, r) = start().await;
    let mut c = config();
    c["log_provider"] = json!("external-ci");
    scenario(&d, c);
    let seq = r.request_logs(job()).unwrap();
    assert_eq!(result(&r, seq).await.state, "failed");
    assert!(
        !std::fs::read_to_string(d.path().join("reads"))
            .unwrap()
            .contains("/logs\"")
    );
    scenario(&d, config());
    let seq = r.request_logs(job()).unwrap();
    assert_eq!(result(&r, seq).await.state, "complete");
    let sql = rusqlite::Connection::open(d.path().join("db")).unwrap();
    sql.execute(
        "UPDATE watch_locator SET epoch=epoch+1 WHERE repo='a/b' AND number=1",
        [],
    )
    .unwrap();
    assert_eq!(r.read_logs(seq).unwrap().state, "released");
    let s = Store::open(&d.path().join("db")).unwrap();
    s.record_response(
        "core",
        &gitjuke_core::github::Response {
            status: 429,
            headers: std::collections::BTreeMap::from([("retry-after".into(), "60".into())]),
            body: json!({"message":"rate limit"}),
        },
        0,
        gitjuke_core::now(),
    )
    .unwrap();
    drop(s);
    let seq = r.request_logs(job()).unwrap();
    tokio::time::sleep(Duration::from_millis(1200)).await;
    assert_eq!(r.read_logs(seq).unwrap().state, "queued");
    let count: i64 = sql
        .query_row(
            "SELECT count(*) FROM pending WHERE job LIKE '%Logs%'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 0);
    r.cancel_logs(seq).unwrap();
    r.shutdown().await;
}

#[tokio::test]
async fn in_flight_clear_epoch_and_storage_failure_drop_bodies() {
    for action in ["clear", "epoch", "storage"] {
        let (d, r) = start().await;
        let mut c = config();
        c["slow_log"] = json!(0.5);
        scenario(&d, c);
        let seq = r.request_logs(job()).unwrap();
        until(|| d.path().join("log-started").exists()).await;
        let sql = rusqlite::Connection::open(d.path().join("db")).unwrap();
        match action {
            "clear"=>r.clear().unwrap(),
            "epoch"=>{sql.execute("UPDATE watch_locator SET epoch=epoch+1 WHERE repo='a/b' AND number=1",[]).unwrap();},
            _=>sql.execute_batch("CREATE TRIGGER fail_log_budget BEFORE UPDATE ON progress WHEN NEW.key='budget:core' BEGIN SELECT RAISE(ABORT,'local log accounting failure'); END;").unwrap(),
        }
        assert_eq!(result(&r, seq).await.state, "released");
        until(|| r.status().is_ok_and(|s| !s.busy)).await;
        if action == "storage" {
            assert!(r.status().unwrap().storage_stopped);
        }
        assert!(r.read_logs(seq).unwrap().text.is_none());
        r.shutdown().await;
    }
}
