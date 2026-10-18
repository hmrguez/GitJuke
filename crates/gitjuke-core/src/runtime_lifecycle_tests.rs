use super::*;
#[tokio::test]
async fn background_starts_closed_and_preserves_pause_selection_and_settings() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let mut store = Store::open(&path).unwrap();
    let mut settings = store.settings().unwrap();
    settings.paused = true;
    settings.gh_path = Some("/not/installed/gh".into());
    settings.theme.accent = "#abcdef".into();
    store.save_settings(&settings).unwrap();
    store
        .bind(&Identity {
            id: 42,
            login: "reader".into(),
        })
        .unwrap();
    store
        .remember_selection(Some(&("a/b".into(), 7)), true)
        .unwrap();
    store.conn.execute_batch("INSERT INTO seen VALUES('history',0); INSERT INTO dedup VALUES('history'); INSERT INTO activity_intent VALUES(1,'pr','attempted',1); INSERT INTO alert_attempt VALUES('old-attempt',1); INSERT INTO due VALUES('activity:notifications',4070908800);").unwrap();
    drop(store);
    let runtime = Runtime::start_background(path.clone(), |_| {}).unwrap();
    {
        let i = runtime.lock().unwrap();
        assert!(!i.reader_open);
        assert!(!i.reader_visible);
        assert!(i.store.settings().unwrap().paused);
        assert_eq!(i.selected, Some(("a/b".into(), 7)));
        assert_eq!(i.store.settings().unwrap().theme.accent, "#abcdef");
        assert!(i.locked, "startup never bypasses stable-account validation");
    }
    runtime.reader_opened().unwrap();
    assert!(runtime.lock().unwrap().reader_open);
    runtime.reader_closed().unwrap();
    runtime.shutdown().await;
    let store = Store::open(&path).unwrap();
    assert!(store.settings().unwrap().paused);
    assert_eq!(store.selection().unwrap(), Some(("a/b".into(), 7)));
    let snapshot:(i64,i64,i64,i64)=store.conn.query_row("SELECT (SELECT seen FROM seen WHERE source_id='history'),(SELECT count(*) FROM dedup WHERE source_id='history'),(SELECT count(*) FROM activity_intent WHERE state='attempted'),(SELECT count(*) FROM alert_attempt)",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap();
    assert_eq!(snapshot, (0, 1, 1, 1));
    let deadline: i64 = store
        .conn
        .query_row(
            "SELECT at FROM due WHERE key='activity:notifications'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(deadline, 4070908800);
    assert_eq!(
        store.settings().unwrap().gh_path,
        Some("/not/installed/gh".into())
    );
    drop(store);
    let runtime = Runtime::start_background(path, |_| {}).unwrap();
    assert!(!runtime.lock().unwrap().reader_open);
    assert!(runtime.status().unwrap().settings.paused);
    runtime.shutdown().await;
}
#[tokio::test]
async fn shutdown_reaps_owned_process_group_and_never_reschedules() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let executable = dir.path().join("gh");
    std::fs::write(
        &executable,
        r#"#!/bin/sh
marker="${0%/*}"
echo $$ > "$marker/parent"
echo read >> "$marker/reads"
/bin/sleep 60 &
echo $! > "$marker/child"
wait
"#,
    )
    .unwrap();
    std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
    let mut store = Store::open(&path).unwrap();
    let mut settings = store.settings().unwrap();
    settings.gh_path = Some(executable.to_string_lossy().into_owned());
    settings.config_dir = Some(
        dir.path()
            .join("no-credentials")
            .to_string_lossy()
            .into_owned(),
    );
    store.save_settings(&settings).unwrap();
    drop(store);
    let runtime = Runtime::start_background(path, |_| {}).unwrap();
    let pids = tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            let read = |name: &str| {
                std::fs::read_to_string(dir.path().join(name))
                    .ok()
                    .and_then(|s| s.trim().parse::<i32>().ok())
            };
            if let (Some(parent), Some(child)) = (read("parent"), read("child")) {
                break (parent, child);
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    runtime.request_shutdown();
    assert!(
        runtime.lock().unwrap().current.is_cancelled(),
        "Quit cancels the active child before polling async shutdown"
    );
    assert!(
        runtime.next().unwrap().is_none(),
        "Quit stops further eligibility immediately"
    );
    tokio::time::timeout(Duration::from_secs(15), runtime.shutdown())
        .await
        .unwrap();
    let until = std::time::Instant::now() + Duration::from_secs(5);
    while [pids.0, pids.1]
        .iter()
        .any(|pid| unsafe { libc::kill(*pid, 0) } == 0)
        && std::time::Instant::now() < until
    {
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    for pid in [pids.0, pids.1] {
        assert_eq!(
            unsafe { libc::kill(pid, 0) },
            -1,
            "owned child {pid} survived Quit"
        );
    }
    tokio::time::sleep(Duration::from_millis(1200)).await;
    assert_eq!(
        std::fs::read_to_string(dir.path().join("reads"))
            .unwrap()
            .lines()
            .count(),
        1
    );
    runtime.shutdown().await;
}
