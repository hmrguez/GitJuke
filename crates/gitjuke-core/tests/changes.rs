use gitjuke_core::{
    changes::{ChangesJob, Files, Patch},
    model::Snapshot,
    runtime::Runtime,
    store::Store,
};
use serde_json::{Value, json};
use std::{os::unix::fs::PermissionsExt, time::Duration};
use tempfile::TempDir;
static FIXTURES: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
fn scenario(d: &TempDir, v: Value) {
    let p = d.path().join("scenario.tmp");
    std::fs::write(&p, serde_json::to_vec(&v).unwrap()).unwrap();
    std::fs::rename(p, d.path().join("scenario.json")).unwrap();
}
fn config() -> Value {
    json!({"head":"a".repeat(40),"no_teams":true,"direct_ids":[],"files_count":45})
}
async fn until(mut f: impl FnMut() -> bool) {
    tokio::time::timeout(Duration::from_secs(30), async {
        while !f() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("Changes fixture reached expected state");
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
    until(|| r.status().is_ok_and(|s| !s.locked)).await;
    r.select("a/b".into(), 1).unwrap();
    until(|| r.overview("a/b", 1).is_ok_and(|s| s.data.is_some())).await;
    (d, r)
}
fn job(page: u32) -> ChangesJob {
    ChangesJob {
        repo: "a/b".into(),
        number: 1,
        head: "a".repeat(40),
        base: None,
        page,
        index: None,
        path: None,
        large: false,
        seq: 0,
    }
}
async fn files(r: &Runtime, j: &ChangesJob) -> Files {
    r.request_changes(j.clone()).unwrap();
    until(|| {
        r.changes_files(j)
            .is_ok_and(|p| p.data.is_some() && p.error.is_none())
    })
    .await;
    r.changes_files(j).unwrap().data.unwrap()
}
async fn patch(r: &Runtime, j: &ChangesJob) -> Patch {
    r.request_changes(j.clone()).unwrap();
    until(|| {
        r.changes_patch(j)
            .is_ok_and(|p| p.data.is_some() && p.error.is_none())
    })
    .await;
    r.changes_patch(j).unwrap().data.unwrap()
}
fn selected(index: u32, base: &str) -> ChangesJob {
    let mut j = job(1);
    j.index = Some(index);
    j.path = Some(format!("file-{index}.txt"));
    j.base = Some(base.into());
    j
}
fn error<T>(p: Snapshot<Option<T>>) -> bool {
    p.error.is_some_and(|s| !s.starts_with("Queued —"))
}
#[tokio::test]
async fn paginated_navigation_cached_patches_and_persisted_mode() {
    let _gate = FIXTURES.lock().await;
    let (d, r) = start().await;
    let mut conf = config();
    conf["files_count"] = json!(245);
    scenario(&d, conf);
    let first = files(&r, &job(1)).await;
    assert_eq!(first.items.len(), 100);
    assert!(!first.complete);
    assert_eq!(first.total, 245);
    let mut second_job = job(2);
    second_job.base = Some(first.base.clone());
    let second = files(&r, &second_job).await;
    assert_eq!(second.items.first().unwrap().index, 100);
    let mut last_job = job(3);
    last_job.base = Some(first.base.clone());
    let last = files(&r, &last_job).await;
    assert_eq!(last.items.len(), 45);
    assert_eq!(last.next_page, None);
    let j = selected(35, &first.base);
    let p = patch(&r, &j).await;
    assert_eq!(p.state, "complete");
    assert_eq!(p.chunks, 1);
    let chunk = r
        .changes_chunk(&j, p.digest.as_ref().unwrap(), 0)
        .unwrap()
        .data
        .unwrap();
    assert_eq!(chunk, "@@ -0,0 +1 @@\n+hello \tworld");
    assert!(
        serde_json::to_vec(&r.changes_chunk(&j, p.digest.as_ref().unwrap(), 0).unwrap())
            .unwrap()
            .len()
            < 1024 * 1024
    );
    assert!(r.changes_chunk(&j, "forged", 0).is_err());
    assert!(r.changes_chunk(&j, p.digest.as_ref().unwrap(), 1).is_err());
    assert_eq!(
        r.changes_url(&j).unwrap(),
        format!("https://github.com/a/b/blob/{}/file-35.txt", "a".repeat(40))
    );
    let reads = std::fs::read_to_string(d.path().join("reads")).unwrap();
    let file_reads: Vec<Value> = reads
        .lines()
        .map(|s| serde_json::from_str::<Value>(s).unwrap())
        .filter(|v| {
            v["args"]
                .as_array()
                .unwrap()
                .iter()
                .any(|a| a.as_str().is_some_and(|s| s.contains("/files?")))
        })
        .collect();
    assert_eq!(file_reads.len(), 4); // Three batches plus an explicit selected-patch refresh, never a full diff.
    assert!(!reads.contains("--paginate"));
    assert!(!reads.contains("raw.githubusercontent"));
    r.set_changes_mode(true).unwrap();
    r.shutdown().await;
    drop(r);
    assert!(
        Store::open(&d.path().join("db"))
            .unwrap()
            .settings()
            .unwrap()
            .paged_changes
    );
}
#[tokio::test]
async fn cancellation_reaps_obsolete_file_and_late_results_cannot_replace_it() {
    let _gate = FIXTURES.lock().await;
    let (d, r) = start().await;
    let f = files(&r, &job(1)).await;
    let mut slow = config();
    slow["slow_files"] = json!(3);
    scenario(&d, slow);
    let obsolete = selected(0, &f.base);
    r.request_changes(obsolete.clone()).unwrap();
    until(|| d.path().join("files-started").exists()).await;
    scenario(&d, config());
    let current = selected(1, &f.base);
    let p = patch(&r, &current).await;
    assert_eq!(p.file.path, "file-1.txt");
    assert_eq!(
        r.changes_patch(&obsolete).unwrap().data.unwrap().file.index,
        0
    ); // Original batch cache survives cancellation.
    assert_eq!(r.status().unwrap().selected, Some(("a/b".into(), 1)));
    assert!(r.status().unwrap().settings.paused);
    r.shutdown().await;
}
#[tokio::test]
async fn moving_head_or_base_and_file_order_never_publish_mixed_revisions() {
    let _gate = FIXTURES.lock().await;
    let (d, r) = start().await;
    let f = files(&r, &job(1)).await;
    let mut slow = config();
    slow["slow_files"] = json!(1);
    scenario(&d, slow);
    let j = selected(2, &f.base);
    r.request_changes(j.clone()).unwrap();
    until(|| d.path().join("files-started").exists()).await;
    let mut changed = config();
    changed["head"] = json!("c".repeat(40));
    scenario(&d, changed);
    until(|| r.changes_patch(&j).is_ok_and(error)).await;
    assert_eq!(
        r.changes_patch(&j).unwrap().data.unwrap().head,
        "a".repeat(40)
    );
    let mut changed = config();
    changed["base_sha"] = json!("d".repeat(40));
    scenario(&d, changed);
    r.request_changes(j.clone()).unwrap();
    until(|| r.changes_patch(&j).is_ok_and(error)).await;
    assert_eq!(
        r.changes_patch(&j).unwrap().data.unwrap().base,
        "b".repeat(40)
    );
    scenario(&d, config());
    let mut wrong = selected(2, &f.base);
    wrong.path = Some("wrong.txt".into());
    r.request_changes(wrong.clone()).unwrap();
    until(|| r.changes_patch(&wrong).is_ok_and(error)).await;
    assert!(r.changes_patch(&wrong).unwrap().data.is_none());
    r.shutdown().await;
}
#[tokio::test]
async fn clear_denial_and_identity_loss_hide_changes_and_reject_inflight_results() {
    let _gate = FIXTURES.lock().await;
    let (d, r) = start().await;
    let f = files(&r, &job(1)).await;
    let mut slow = config();
    slow["slow_files"] = json!(3);
    scenario(&d, slow);
    let j = selected(0, &f.base);
    r.request_changes(j.clone()).unwrap();
    until(|| d.path().join("files-started").exists()).await;
    r.clear().unwrap();
    until(|| !r.status().unwrap().busy).await;
    assert!(r.changes_files(&job(1)).unwrap().data.is_none());
    assert!(r.changes_patch(&j).unwrap().data.is_none());
    scenario(&d, config());
    files(&r, &job(1)).await;
    r.select("a/b".into(), 1).unwrap();
    until(|| r.overview("a/b", 1).is_ok_and(|p| p.data.is_some())).await;
    let mut deny = config();
    deny["deny_files"] = json!(true);
    scenario(&d, deny);
    r.request_changes(j.clone()).unwrap();
    until(|| r.changes_patch(&j).is_ok_and(error)).await;
    assert!(r.changes_files(&job(1)).unwrap().data.is_none());
    assert!(r.overview("a/b", 1).unwrap().data.is_some());
    std::fs::write(d.path().join("mode"), "mismatch").unwrap();
    r.request_changes(j.clone()).unwrap();
    until(|| r.status().unwrap().locked).await;
    assert!(r.changes_files(&job(1)).is_err());
    assert!(r.changes_chunk(&j, "x", 0).is_err());
    r.shutdown().await;
}
#[tokio::test]
async fn metadata_transport_overflow_subdivides_and_large_tier_is_explicit() {
    let _gate = FIXTURES.lock().await;
    let (d, r) = start().await;
    let mut conf = config();
    conf["overflow_files"] = json!(true);
    scenario(&d, conf);
    let f = files(&r, &job(1)).await;
    assert_eq!(f.items.len(), 45);
    assert_eq!(
        r.changes_patch(&selected(0, &f.base))
            .unwrap()
            .data
            .unwrap()
            .state,
        "complete"
    );
    let text = format!("@@ -0,0 +1,6000 @@\n{}", vec!["+x"; 6000].join("\n"));
    let mut conf = config();
    conf["patch"] = json!(text);
    conf["patch_additions"] = json!(6000);
    scenario(&d, conf);
    let f = files(&r, &job(1)).await;
    let mut j = selected(0, &f.base);
    let p = r.changes_patch(&j).unwrap().data.unwrap();
    assert_eq!(p.state, "large");
    assert_eq!(p.chunks, 0);
    j.large = true;
    let p = patch(&r, &j).await;
    assert_eq!(p.state, "complete");
    assert_eq!(p.rows, Some(6001));
    let text = format!(
        "@@ -0,0 +1,200 @@\n{}",
        vec![format!("+{}", "é\t\"\\".repeat(2000)); 200].join("\n")
    );
    let mut conf = config();
    conf["patch"] = json!(text);
    conf["patch_additions"] = json!(200);
    scenario(&d, conf);
    let p = patch(&r, &j).await;
    assert_eq!(p.state, "complete");
    assert!(p.chunks > 10);
    let mut source = String::new();
    for chunk in 0..p.chunks {
        let part = r
            .changes_chunk(&j, p.digest.as_ref().unwrap(), chunk)
            .unwrap();
        assert!(serde_json::to_vec(&part).unwrap().len() < 1024 * 1024);
        source.push_str(part.data.as_ref().unwrap());
    }
    assert_eq!(source, text);
    let sql = rusqlite::Connection::open(d.path().join("db")).unwrap();
    sql.execute(
        "DELETE FROM content WHERE key=?",
        [format!(
            "{}/chunk/{}/0",
            j.key(),
            p.digest.as_ref().unwrap()
        )],
    )
    .unwrap();
    assert!(
        r.changes_chunk(&j, p.digest.as_ref().unwrap(), 0)
            .unwrap()
            .data
            .is_none()
    );
    let mut conf = config();
    conf["patch"] = json!("x".repeat(2 * 1024 * 1024 + 1));
    scenario(&d, conf);
    let p = patch(&r, &j).await;
    assert_eq!(p.state, "tooLarge");
    assert_eq!(p.chunks, 0);
    r.shutdown().await;
}

#[tokio::test]
async fn two_hundred_files_use_two_batches_and_supply_cached_patches() {
    let _gate = FIXTURES.lock().await;
    let (d, r) = start().await;
    let mut conf = config();
    conf["files_count"] = json!(200);
    scenario(&d, conf);
    let started = std::time::Instant::now();
    let mut j = job(1);
    let mut all = Vec::new();
    loop {
        let f = files(&r, &j).await;
        all.extend(f.items);
        j.base = Some(f.base);
        if let Some(page) = f.next_page {
            j.page = page;
        } else {
            break;
        }
    }
    let fetch_elapsed = started.elapsed();
    assert_eq!(all.len(), 200);
    for file in &all {
        let mut selected = selected(file.index, j.base.as_ref().unwrap());
        // The eager reader requests the full variant; the batch must supply it too.
        selected.large = true;
        let p = r
            .changes_patch(&selected)
            .unwrap()
            .data
            .expect("batch must retain supplied patches before publishing file metadata");
        assert_eq!(p.file, *file);
        assert_eq!(p.state, "complete");
        assert!(
            r.changes_chunk(&selected, p.digest.as_ref().unwrap(), 0)
                .unwrap()
                .data
                .is_some()
        );
    }
    let reads = std::fs::read_to_string(d.path().join("reads")).unwrap();
    let endpoints: Vec<String> = reads
        .lines()
        .map(|line| {
            let v: Value = serde_json::from_str(line).unwrap();
            v["args"]
                .as_array()
                .unwrap()
                .last()
                .unwrap()
                .as_str()
                .unwrap_or("")
                .into()
        })
        .collect();
    let positions: Vec<_> = endpoints
        .iter()
        .enumerate()
        .filter(|(_, s)| s.contains("/files?"))
        .map(|(n, _)| n)
        .collect();
    assert_eq!(
        positions.len(),
        2,
        "200 files should require just two file-list requests"
    );
    for n in positions {
        assert!(endpoints[n].contains("per_page=100"));
        assert_eq!(&endpoints[n - 2..n], &["user", "repos/a/b/pulls/1"]);
        assert_eq!(&endpoints[n + 1..n + 3], &["repos/a/b/pulls/1", "user"]);
    }
    eprintln!(
        "200-file batch fetch and cache: {fetch_elapsed:?}; all 200 patches available without another file request"
    );
    r.shutdown().await;
}

#[tokio::test]
async fn rejected_batch_never_publishes_metadata_or_prefetched_patches() {
    let _gate = FIXTURES.lock().await;
    let (d, r) = start().await;
    let mut conf = config();
    conf["slow_files"] = json!(0.3);
    scenario(&d, conf);
    r.request_changes(job(1)).unwrap();
    until(|| d.path().join("files-started").exists()).await;
    let mut changed = config();
    changed["base_sha"] = json!("c".repeat(40));
    scenario(&d, changed);
    until(|| r.changes_files(&job(1)).is_ok_and(error)).await;
    assert!(r.changes_files(&job(1)).unwrap().data.is_none());
    assert!(
        r.changes_patch(&selected(0, &"b".repeat(40)))
            .unwrap()
            .data
            .is_none()
    );
    std::fs::remove_file(d.path().join("files-started")).unwrap();
    let mut conf = config();
    conf["slow_files"] = json!(0.3);
    scenario(&d, conf);
    r.request_changes(job(1)).unwrap();
    until(|| d.path().join("files-started").exists()).await;
    r.clear().unwrap();
    until(|| !r.status().unwrap().busy).await;
    assert!(r.changes_files(&job(1)).unwrap().data.is_none());
    assert!(
        r.changes_patch(&selected(0, &"b".repeat(40)))
            .unwrap()
            .data
            .is_none()
    );
    r.shutdown().await;
}
