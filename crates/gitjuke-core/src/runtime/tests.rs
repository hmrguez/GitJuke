use super::*;
// Each fixture owns an independent app and real gh subprocesses. Keep app
// fixtures isolated so parallel libtest load does not consume their existing
// state deadlines; concurrency within each runtime remains exercised.
static WORKER_FIXTURES: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
async fn wait_for(mut condition: impl FnMut() -> bool) {
    tokio::time::timeout(Duration::from_secs(25), async {
        while !condition() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("expected runtime state");
}
#[tokio::test]
async fn write_fault_stops_children_and_recovery_requires_a_committed_probe() {
    let _worker_fixture = WORKER_FIXTURES.lock().await;
    use std::os::unix::fs::PermissionsExt;
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("db");
    let executable = d.path().join("gh");
    std::fs::write(
        &executable,
        include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/support/gh.py")),
    )
    .unwrap();
    std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
    let mut store = Store::open(&path).unwrap();
    let mut settings = store.settings().unwrap();
    settings.gh_path = Some(executable.display().to_string());
    settings.config_dir = Some(d.path().join("config").display().to_string());
    store.save_settings(&settings).unwrap();
    drop(store);
    let r = Runtime::start(path.clone(), |_| {}).unwrap();
    wait_for(|| r.activity(0).is_ok_and(|a| a.baseline)).await;
    let sql = rusqlite::Connection::open(path).unwrap();
    sql.execute_batch("CREATE TRIGGER fail_queue BEFORE INSERT ON pending BEGIN SELECT RAISE(ABORT,'fixture storage write failure'); END;").unwrap();
    {
        let i = r.lock().unwrap();
        i.store.set_due("discovery", 0).unwrap();
    }
    wait_for(|| r.status().unwrap().storage_stopped).await;
    let reads = || {
        std::fs::read_to_string(d.path().join("reads"))
            .unwrap_or_default()
            .lines()
            .count()
    };
    let stopped_count = reads();
    assert!(r.page(0).unwrap().complete, "cached reading survives");
    r.set_pause(true).unwrap();
    r.set_theme(Theme::default()).unwrap();
    r.refresh().unwrap_err(); // queue writes still fail; permission to read cannot bypass protection
    tokio::time::sleep(Duration::from_millis(1200)).await;
    assert_eq!(reads(), stopped_count);
    // Failed health probes keep the stop, even when other writes work.
    sql.execute_batch("CREATE TRIGGER fail_probe BEFORE UPDATE ON meta BEGIN SELECT RAISE(ABORT,'fixture probe failure'); END; DROP TRIGGER fail_queue;").unwrap();
    r.lock().unwrap().storage_check_at = 0;
    tokio::time::sleep(Duration::from_millis(1200)).await;
    assert!(r.status().unwrap().storage_stopped);
    assert_eq!(reads(), stopped_count);
    sql.execute_batch("DROP TRIGGER fail_probe").unwrap();
    r.lock().unwrap().storage_check_at = 0;
    wait_for(|| !r.status().unwrap().storage_stopped).await;
    assert!(r.status().unwrap().settings.paused);
    assert!(r.activity(0).unwrap().recovery.is_some());
    assert_eq!(
        reads(),
        stopped_count,
        "repair never resumes a persistent Pause"
    );
    // A partial PASSIVE checkpoint must enter the same protection path.
    sql.execute_batch("BEGIN; SELECT * FROM meta;").unwrap();
    r.set_theme(Theme::default()).unwrap();
    r.lock().unwrap().storage_check_at = 0;
    wait_for(|| r.status().unwrap().storage_stopped).await;
    assert_eq!(reads(), stopped_count);
    sql.execute_batch("ROLLBACK").unwrap();
    r.lock().unwrap().storage_check_at = 0;
    wait_for(|| !r.status().unwrap().storage_stopped).await;
    // A local settings failure also stops the monitor immediately.
    sql.execute_batch("CREATE TRIGGER fail_settings BEFORE INSERT ON settings BEGIN SELECT RAISE(ABORT,'fixture settings failure'); END;").unwrap();
    assert!(r.set_theme(Theme::default()).is_err());
    assert!(r.status().unwrap().storage_stopped);
    assert_eq!(reads(), stopped_count);
    sql.execute_batch("DROP TRIGGER fail_settings").unwrap();
    r.shutdown().await;
}
#[tokio::test]
async fn control_sweeps_protect_startup_and_resume_before_remote_eligibility() {
    let _worker_fixture = WORKER_FIXTURES.lock().await;
    for kind in ["heads", "activity", "manual"] {
        let (d, path) = fixture_store();
        let mut s = Store::open(&path).unwrap();
        s.bind(&Identity {
            id: 42,
            login: "reader".into(),
        })
        .unwrap();
        let mut settings = s.settings().unwrap();
        settings.paused = true;
        s.save_settings(&settings).unwrap();
        s.conn.execute_batch("INSERT INTO due VALUES('activity:notifications',4070908800); INSERT INTO activity VALUES('A','a/b',1,'comment',4070908800,1,'eligible',1); INSERT INTO seen VALUES('A',0); INSERT INTO dedup VALUES('A'); INSERT INTO activity_intent VALUES(1,'pr','attempted',1); INSERT INTO intent_member VALUES(1,'A'); INSERT INTO alert_attempt VALUES('attempt',1);").unwrap();
        for k in 0..257 {
            s.conn
                .execute(
                    "INSERT INTO watch_locator(id,repo,number,due) VALUES(?,'a/b',?,999)",
                    rusqlite::params![format!("P{k:04}"), k],
                )
                .unwrap();
            s.conn
                .execute(
                    "INSERT INTO watch_source VALUES(?,'direct')",
                    [format!("P{k:04}")],
                )
                .unwrap();
            s.conn
                .execute(
                    "INSERT INTO activity_stage VALUES('a/b',1,?,'{}')",
                    [format!("S{k:04}")],
                )
                .unwrap();
            s.conn
                .execute(
                    "INSERT INTO pending(key,job,manual,foreground,at) VALUES(?,?,0,0,0)",
                    rusqlite::params![format!("heads-{k:04}"), "{\"Heads\":[]}"],
                )
                .unwrap();
        }
        let (table, action, pred) = match kind {
            "heads" => ("pending", "DELETE", "OLD.key='heads-0200'"),
            "activity" => ("activity_stage", "DELETE", "OLD.id='S0200'"),
            _ => ("watch_locator", "UPDATE", "OLD.id='P0200'"),
        };
        s.conn.execute_batch(&format!("CREATE TRIGGER fail_control BEFORE {action} ON {table} WHEN {pred} BEGIN SELECT RAISE(ABORT,'late control sweep'); END;")).unwrap();
        assert!(
            crate::control_sweep::run(
                &mut s.conn,
                kind,
                serde_json::json!({"repo":"a/b","number":1})
            )
            .is_err()
        );
        drop(s);
        let r = Runtime::start(path.clone(), |_| {}).unwrap();
        assert!(
            r.status().unwrap().locked
                && r.status().unwrap().storage_stopped
                && r.status().unwrap().settings.paused
        );
        assert!(r.refresh().is_err());
        assert_eq!(reads(d.path()), 0);
        r.lock().unwrap().storage_check_at = 0;
        wait_for(|| r.lock().unwrap().storage_check_at != 0).await;
        assert!(r.status().unwrap().storage_stopped);
        assert_eq!(reads(d.path()), 0);
        let sql = rusqlite::Connection::open(&path).unwrap();
        sql.execute_batch("DROP TRIGGER fail_control").unwrap();
        r.lock().unwrap().storage_check_at = 0;
        wait_for(|| !r.status().unwrap().storage_stopped).await;
        assert!(r.status().unwrap().locked && r.status().unwrap().settings.paused);
        assert_eq!(reads(d.path()), 0);
        assert!(!crate::control_sweep::pending(&sql).unwrap());
        assert_eq!(r.lock().unwrap().store.identity().unwrap().unwrap().id, 42);
        for query in [
            "SELECT COUNT(*) FROM seen WHERE source_id='A' AND seen=0",
            "SELECT COUNT(*) FROM dedup WHERE source_id='A'",
            "SELECT COUNT(*) FROM activity_intent WHERE state='attempted'",
            "SELECT COUNT(*) FROM intent_member WHERE activity='A'",
            "SELECT COUNT(*) FROM alert_attempt",
        ] {
            assert_eq!(sql.query_row(query, [], |r| r.get::<_, i64>(0)).unwrap(), 1);
        }
        assert_eq!(
            r.lock()
                .unwrap()
                .store
                .due("activity:notifications")
                .unwrap(),
            4070908800
        );
        r.shutdown().await;
    }
}

#[tokio::test]
async fn manual_control_fault_preserves_cached_navigation_and_paused_repair() {
    let _worker_fixture = WORKER_FIXTURES.lock().await;
    let (d, path) = fixture_store();
    let r = Runtime::start(path.clone(), |_| {}).unwrap();
    wait_for(|| r.activity(0).is_ok_and(|p| p.baseline)).await;
    r.set_pause(true).unwrap();
    wait_for(|| !r.status().unwrap().busy).await;
    let sql = rusqlite::Connection::open(&path).unwrap();
    sql.execute("UPDATE watch_locator SET due=0", []).unwrap();
    sql.execute(
        "INSERT INTO due VALUES('remote',4070908800) ON CONFLICT(key) DO UPDATE SET at=excluded.at",
        [],
    )
    .unwrap();
    for k in 0..257 {
        sql.execute(
            "INSERT INTO watch_locator(id,repo,number,due) VALUES(?,'fixture/repo',?,999)",
            rusqlite::params![format!("M{k:04}"), k],
        )
        .unwrap();
        sql.execute(
            "INSERT INTO watch_source VALUES(?,'direct')",
            [format!("M{k:04}")],
        )
        .unwrap();
    }
    let before = r.activity(0).unwrap();
    let read_count = reads(d.path());
    sql.execute_batch("CREATE TRIGGER fail_manual BEFORE UPDATE OF due ON watch_locator WHEN OLD.id='M0200' BEGIN SELECT RAISE(ABORT,'late manual reset'); END;").unwrap();
    assert!(r.refresh().is_err());
    assert!(r.status().unwrap().storage_stopped);
    assert_eq!(
        sql.query_row(
            "SELECT COUNT(*) FROM watch_locator WHERE id GLOB 'M*' AND due!=0",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        130
    );
    assert_eq!(r.activity(0).unwrap().items.len(), before.items.len());
    assert!(r.page(0).is_ok());
    assert_eq!(reads(d.path()), read_count);
    r.lock().unwrap().storage_check_at = 0;
    wait_for(|| r.lock().unwrap().storage_check_at != 0).await;
    assert!(r.status().unwrap().storage_stopped);
    assert!(r.refresh().is_err());
    assert_eq!(reads(d.path()), read_count);
    sql.execute_batch("DROP TRIGGER fail_manual").unwrap();
    r.lock().unwrap().storage_check_at = 0;
    wait_for(|| !r.status().unwrap().storage_stopped).await;
    assert!(r.status().unwrap().settings.paused);
    assert!(!crate::control_sweep::pending(&sql).unwrap());
    assert_eq!(reads(d.path()), read_count);
    assert_eq!(
        sql.query_row(
            "SELECT COUNT(*) FROM watch_locator WHERE id GLOB 'M*' AND due!=0",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    assert_eq!(r.activity(0).unwrap().items.len(), before.items.len());
    r.shutdown().await;
}

#[tokio::test]
async fn query_cursor_checkpoint_fault_protects_cached_session_and_repairs_paused() {
    let _worker_fixture = WORKER_FIXTURES.lock().await;
    let (d, path) = fixture_store();
    let r = Runtime::start(path.clone(), |_| {}).unwrap();
    wait_for(|| r.activity(0).is_ok_and(|p| p.baseline)).await;
    r.set_pause(true).unwrap();
    wait_for(|| !r.status().unwrap().busy).await;
    let sql = rusqlite::Connection::open(&path).unwrap();
    sql.execute(
        "INSERT INTO due VALUES('remote',4070908800) ON CONFLICT(key) DO UPDATE SET at=excluded.at",
        [],
    )
    .unwrap();
    sql.execute_batch("UPDATE activity_cycle SET mode='recovery',boundary=123; WITH RECURSIVE x(n) AS(VALUES(0) UNION ALL SELECT n+1 FROM x WHERE n<255) INSERT INTO api_validator(url,next_at) SELECT printf('zz-query-%04d',n),4070908800 FROM x;").unwrap();
    r.lock().unwrap().store.maintain_retention().unwrap();
    let cursor: String = sql
        .query_row(
            "SELECT position FROM retention_cursor WHERE class='validators'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let reads_before = reads(d.path());
    let poll = r
        .lock()
        .unwrap()
        .store
        .due("activity:notifications")
        .unwrap();
    let page = r.page(0).unwrap();
    sql.execute_batch("CREATE TRIGGER fail_query_cursor BEFORE INSERT ON retention_cursor WHEN NEW.class='validators' BEGIN SELECT RAISE(ABORT,'cursor checkpoint fault'); END;").unwrap();
    r.lock().unwrap().storage_check_at = 0;
    wait_for(|| r.status().unwrap().storage_stopped).await;
    assert_eq!(reads(d.path()), reads_before);
    assert_eq!(r.page(0).unwrap().items.len(), page.items.len());
    assert!(r.activity(0).unwrap().baseline);
    assert!(r.refresh().is_err());
    assert_eq!(
        sql.query_row(
            "SELECT position FROM retention_cursor WHERE class='validators'",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        cursor
    );
    r.lock().unwrap().storage_check_at = 0;
    wait_for(|| r.lock().unwrap().storage_check_at != 0).await;
    assert!(r.status().unwrap().storage_stopped);
    assert_eq!(reads(d.path()), reads_before);
    sql.execute_batch("DROP TRIGGER fail_query_cursor").unwrap();
    r.lock().unwrap().storage_check_at = 0;
    wait_for(|| !r.status().unwrap().storage_stopped).await;
    assert!(r.status().unwrap().settings.paused);
    assert_eq!(reads(d.path()), reads_before);
    assert_eq!(
        r.lock()
            .unwrap()
            .store
            .due("activity:notifications")
            .unwrap(),
        poll
    );
    assert_eq!(
        sql.query_row(
            "SELECT boundary FROM activity_cycle WHERE mode='recovery'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        123
    );
    assert_eq!(
        sql.query_row(
            "SELECT COUNT(*) FROM api_validator WHERE url GLOB 'zz-query-*' AND next_at=4070908800",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        256
    );
    assert_eq!(r.page(0).unwrap().items.len(), page.items.len());
    r.shutdown().await;
}

fn fixture_store() -> (tempfile::TempDir, PathBuf) {
    use std::os::unix::fs::PermissionsExt;
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("db");
    let executable = d.path().join("gh");
    std::fs::write(
        &executable,
        include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/support/gh.py")),
    )
    .unwrap();
    std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
    let mut store = Store::open(&path).unwrap();
    let mut settings = store.settings().unwrap();
    settings.gh_path = Some(executable.display().to_string());
    settings.config_dir = Some(d.path().join("config").display().to_string());
    store.save_settings(&settings).unwrap();
    (d, path)
}
fn reads(d: &std::path::Path) -> usize {
    std::fs::read_to_string(d.join("reads"))
        .unwrap_or_default()
        .lines()
        .count()
}

#[tokio::test]
async fn full_ceiling_worker_publication_fault_stops_reads_and_repairs_while_paused() {
    let _worker_fixture = WORKER_FIXTURES.lock().await;
    use crate::content_budget::disposable_bytes;
    use crate::content_capacity::{CAPACITY, count, disk_backed, mixed_payloads, small_checks};
    use crate::store::CONTENT_LIMIT;
    use rusqlite::Connection;
    use std::{os::unix::fs::PermissionsExt, time::Duration};
    let _serial = CAPACITY.lock().await;
    async fn until(mut predicate: impl FnMut() -> bool) {
        tokio::time::timeout(Duration::from_secs(25), async {
            while !predicate() {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap();
    }
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("db");
    let executable = d.path().join("gh");
    std::fs::write(
        &executable,
        include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/support/gh.py")),
    )
    .unwrap();
    std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
    let mut s = Store::open(&path).unwrap();
    let mut settings = s.settings().unwrap();
    settings.gh_path = Some(executable.display().to_string());
    settings.config_dir = Some(d.path().join("config").display().to_string());
    s.save_settings(&settings).unwrap();
    drop(s);
    let r = Runtime::start(path.clone(), |_| {}).unwrap();
    until(|| r.activity(0).is_ok_and(|a| a.baseline)).await;
    r.set_pause(true).unwrap();
    until(|| !r.status().unwrap().busy).await;
    r.clear().unwrap();
    {
        let mut i = r.lock().unwrap();
        i.store.activity_recovery(123).unwrap();
        mixed_payloads(&mut i.store);
        // Make published checks the oldest victims for the actual selected
        // registration writer; no response-size or budget override is used.
        i.store
            .conn
            .execute(
                "UPDATE content SET accessed=? WHERE key GLOB 'small-*'",
                [now() as i64],
            )
            .unwrap();
        disk_backed(&i.store, &path);
        i.store.conn.execute_batch("CREATE TRIGGER fail_worker_capacity BEFORE DELETE ON check_context WHEN OLD.staged=0 BEGIN SELECT RAISE(ABORT,'fixture worker capacity fault'); END;").unwrap();
    }
    let sql = Connection::open(&path).unwrap();
    let ledgers = || {
        [
            "activity",
            "seen",
            "dedup",
            "activity_intent",
            "intent_member",
            "alert_attempt",
            "progress",
        ]
        .map(|table| {
            count(
                &sql,
                &format!(
                    "SELECT COUNT(*) FROM {table}{}",
                    if table == "progress" {
                        " WHERE key!='selection'"
                    } else {
                        ""
                    }
                ),
            )
        })
    };
    let original = ledgers();
    let identity = r.status().unwrap().identity;
    let deadline = r.activity(0).unwrap().next_poll_at;
    let boundary = count(&sql, "SELECT boundary FROM activity_cycle");
    let revision = r.status().unwrap().revision;
    r.select("a/b".into(), 99).unwrap();
    until(|| r.status().unwrap().storage_stopped).await;
    assert_eq!(disposable_bytes(&sql).unwrap(), CONTENT_LIMIT);
    assert_eq!(small_checks(&sql), 64);
    assert_eq!(
        count(&sql, "SELECT complete FROM check_state WHERE pr_id='P'"),
        1
    );
    assert_eq!(
        count(&sql, "SELECT COUNT(*) FROM watch_locator WHERE number=99"),
        0
    );
    // Selecting commits its own remembered selection before hydration.
    assert_eq!(r.status().unwrap().revision, revision + 1);
    assert!(r.refresh().is_err());
    let reads = || {
        std::fs::read_to_string(d.path().join("reads"))
            .unwrap_or_default()
            .lines()
            .count()
    };
    let stopped = reads();
    // The interrupted manual job remains authorized and queued. Hold it
    // behind a fixture remote deadline while qualifying repair in isolation.
    sql.execute_batch("INSERT INTO due VALUES('remote',4070908800) ON CONFLICT(key) DO UPDATE SET at=excluded.at;").unwrap();
    sql.execute_batch("DROP TRIGGER fail_worker_capacity")
        .unwrap();
    r.lock().unwrap().storage_check_at = 0;
    until(|| !r.status().unwrap().storage_stopped).await;
    assert_eq!(reads(), stopped);
    assert!(r.status().unwrap().settings.paused);
    assert_eq!(r.status().unwrap().identity, identity);
    assert_eq!(r.activity(0).unwrap().next_poll_at, deadline);
    assert_eq!(count(&sql, "SELECT boundary FROM activity_cycle"), boundary);
    assert_eq!(ledgers(), original);
    // Repair recovery initialization discards abandoned Activity staging.
    assert!(disposable_bytes(&sql).unwrap() < CONTENT_LIMIT);
    sql.execute("UPDATE due SET at=0 WHERE key='remote'", [])
        .unwrap();
    r.select("a/b".into(), 99).unwrap();
    until(|| r.overview("a/b", 99).is_ok_and(|v| v.data.is_some())).await;
    assert!(r.status().unwrap().settings.paused);
    assert!(disposable_bytes(&sql).unwrap() <= CONTENT_LIMIT);
    assert_eq!(ledgers(), original);
    assert_eq!(
        count(
            &sql,
            "SELECT COUNT(*) FROM activity_intent WHERE state='pending'"
        ),
        0
    );
    r.shutdown().await;
}

#[tokio::test]
async fn pr_purge_overview_fault_protects_paused_cache_and_repairs_without_neighbor_loss() {
    let _worker_fixture = WORKER_FIXTURES.lock().await;
    let (d, path) = fixture_store();
    let r = Runtime::start(path.clone(), |_| {}).unwrap();
    wait_for(|| r.activity(0).is_ok_and(|a| a.baseline)).await;
    r.select("a/b".into(), 1).unwrap();
    wait_for(|| r.overview("a/b", 1).is_ok_and(|v| v.data.is_some())).await;
    r.set_pause(true).unwrap();
    wait_for(|| !r.status().unwrap().busy).await;
    let sql = rusqlite::Connection::open(&path).unwrap();
    let n = |q: &str| sql.query_row(q, [], |r| r.get::<_, i64>(0)).unwrap();
    let ledgers = || {
        [
            "activity",
            "seen",
            "dedup",
            "activity_intent",
            "intent_member",
            "alert_attempt",
        ]
        .map(|t| n(&format!("SELECT COUNT(*) FROM {t}")))
    };
    let original = ledgers();
    let before = r.status().unwrap();
    let deadline = r.activity(0).unwrap().next_poll_at;
    let epoch = n("SELECT epoch FROM watch_locator WHERE id='PR_1'");
    let neighbor = n("SELECT COUNT(*) FROM check_context WHERE pr_id='PR_2'");
    assert!(neighbor > 0);
    for k in 0..257 {
        sql.execute(
            "INSERT INTO content VALUES(?,'{}',4070908800,4070908800)",
            [format!("api:repos/a/b/pulls/1/comments?fixture={k:03}")],
        )
        .unwrap();
    }
    sql.execute_batch("CREATE TRIGGER fail_pr_purge BEFORE DELETE ON content WHEN OLD.key='api:repos/a/b/pulls/1/comments?fixture=200' BEGIN SELECT RAISE(ABORT,'fixture PR purge'); END;").unwrap();
    std::fs::write(d.path().join("scenario.json"), r#"{"deny_overview":1}"#).unwrap();
    r.select("a/b".into(), 1).unwrap();
    wait_for(|| r.status().unwrap().storage_stopped).await;
    let remaining =
        n("SELECT COUNT(*) FROM content WHERE key GLOB 'api:repos/a/b/pulls/1/comments?fixture=*'");
    assert!(remaining > 0 && remaining < 257);
    assert!(!r.status().unwrap().locked);
    assert!(r.overview("a/b", 1).unwrap().data.is_none());
    assert!(r.checks("a/b", 1, 0).unwrap().items.is_empty());
    assert!(!r.checks("a/b", 2, 0).unwrap().items.is_empty());
    assert!(!r.page(0).unwrap().complete);
    assert!(r.refresh().is_err());
    assert_eq!(ledgers(), original);
    assert!(
        r.activity(0)
            .unwrap()
            .items
            .iter()
            .filter(|a| a.number == 1)
            .all(|a| a.text.is_none())
    );
    sql.execute(
        "INSERT INTO due VALUES('remote',4070908800) ON CONFLICT(key) DO UPDATE SET at=excluded.at",
        [],
    )
    .unwrap();
    let stopped = reads(d.path());
    r.lock().unwrap().storage_check_at = 0;
    wait_for(|| r.lock().unwrap().storage_check_at != 0).await;
    assert!(r.status().unwrap().storage_stopped);
    assert_eq!(reads(d.path()), stopped);
    sql.execute_batch("DROP TRIGGER fail_pr_purge").unwrap();
    r.lock().unwrap().storage_check_at = 0;
    wait_for(|| !r.status().unwrap().storage_stopped).await;
    assert_eq!(reads(d.path()), stopped);
    assert_eq!(r.status().unwrap().identity, before.identity);
    assert!(r.status().unwrap().settings.paused);
    assert_eq!(r.activity(0).unwrap().next_poll_at, deadline);
    assert_eq!(ledgers(), original);
    assert_eq!(
        n("SELECT epoch FROM watch_locator WHERE id='PR_1'"),
        epoch + 1
    );
    assert_eq!(
        n("SELECT COUNT(*) FROM check_context WHERE pr_id='PR_2'"),
        neighbor
    );
    assert!(!crate::pr_purge::any_pending(&r.lock().unwrap().store.conn).unwrap());
    assert!(r.overview("a/b", 1).unwrap().data.is_none());
    r.shutdown().await;
    drop(r);
    let r = Runtime::start(path, |_| {}).unwrap();
    assert!(r.status().unwrap().locked && r.status().unwrap().settings.paused);
    assert_eq!(reads(d.path()), stopped);
    assert_eq!(ledgers(), original);
    r.shutdown().await;
}

#[tokio::test]
async fn interrupted_activity_and_summary_repair_before_remote_eligibility() {
    let _worker_fixture = WORKER_FIXTURES.lock().await;
    for kind in ["activity", "summary"] {
        let (d, path) = fixture_store();
        let mut s = crate::activity_bounded::fixture(&path, kind, "recovery");
        s.conn.execute_batch("CREATE TRIGGER fail_activity_repair BEFORE UPDATE OF applied ON publication_row WHEN NEW.applied=1 AND NEW.seq>(SELECT MIN(seq)+200 FROM publication_row) BEGIN SELECT RAISE(ABORT,'late Activity repair'); END;").unwrap();
        let old = serde_json::to_value(s.activity_page(0).unwrap()).unwrap();
        let result = if kind == "summary" {
            s.activity_finish_recovery(false)
        } else {
            s.activity_publish("a/b", 1, false, &crate::activity_crash::identity())
        };
        assert!(result.is_err());
        let mut settings = s.settings().unwrap();
        settings.paused = true;
        s.save_settings(&settings).unwrap();
        drop(s);
        let r = Runtime::start(path.clone(), |_| {}).unwrap();
        assert!(
            r.status().unwrap().locked
                && r.status().unwrap().settings.paused
                && r.status().unwrap().storage_stopped
        );
        assert_eq!(reads(d.path()), 0);
        assert!(r.refresh().is_err());
        {
            let mut i = r.lock().unwrap();
            let mut page = i.store.activity_page(0).unwrap();
            page.revision = old["revision"].as_u64().unwrap();
            assert_eq!(serde_json::to_value(page).unwrap(), old);
            i.storage_check_at = 0;
        }
        wait_for(|| r.lock().unwrap().storage_check_at != 0).await;
        assert!(r.status().unwrap().storage_stopped);
        assert_eq!(reads(d.path()), 0);
        let c = rusqlite::Connection::open(&path).unwrap();
        c.execute_batch("DROP TRIGGER fail_activity_repair")
            .unwrap();
        r.lock().unwrap().storage_check_at = 0;
        wait_for(|| !r.status().unwrap().storage_stopped).await;
        assert_eq!(reads(d.path()), 0);
        assert!(r.status().unwrap().settings.paused && r.status().unwrap().locked);
        assert!(!crate::publication::pending(&c).unwrap());
        assert_eq!(
            c.query_row(
                "SELECT COUNT(*) FROM activity WHERE id LIKE 'N%'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            if kind == "summary" { 513 } else { 257 }
        );
        if kind == "summary" {
            // Repair runs retention after completing publication. The expired
            // minimal record goes away; its seen/dedup replay barriers survive.
            assert_eq!(
                c.query_row(
                    "SELECT COUNT(*) FROM activity WHERE id='N1-0001'",
                    [],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
                0
            );
            assert_eq!(
                c.query_row(
                    "SELECT COUNT(*) FROM dedup WHERE source_id='N1-0001'",
                    [],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
                1
            );
            assert_eq!(
                c.query_row(
                    "SELECT COUNT(*) FROM seen WHERE source_id='N1-0001'",
                    [],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
                1
            );
        }
        assert_eq!(
            c.query_row(
                "SELECT COUNT(*) FROM intent_member WHERE intent>2",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            if kind == "summary" { 512 } else { 1 }
        );
        assert_eq!(
            c.query_row(
                "SELECT COUNT(*) FROM activity_intent WHERE state IN ('pending','recovery')",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            0
        );
        assert_eq!(
            c.query_row("SELECT state FROM activity_intent WHERE id=1", [], |r| {
                r.get::<_, String>(0)
            })
            .unwrap(),
            "attempted"
        );
        assert_eq!(
            r.lock()
                .unwrap()
                .store
                .due("activity:notifications")
                .unwrap(),
            4070908800
        );
        r.shutdown().await;
    }
}

#[tokio::test]
async fn partial_publication_startup_stays_locked_paused_and_repairs_before_reads() {
    let _worker_fixture = WORKER_FIXTURES.lock().await;
    let (d, path) = fixture_store();
    let (mut s, args) = crate::publication::tests::seed(&path, "head");
    s.conn
        .execute(
            "UPDATE activity SET occurred=?,created=?",
            rusqlite::params![now() as i64, now() as i64],
        )
        .unwrap();
    crate::publication::tests::start(&mut s, "head", &args);
    s.conn.execute_batch("CREATE TRIGGER fail_start_publication BEFORE UPDATE ON publication_row WHEN NEW.applied=1 BEGIN SELECT RAISE(ABORT,'fixture startup publication'); END;").unwrap();
    drop(s);
    let r = Runtime::start(path.clone(), |_| {}).unwrap();
    assert!(
        r.status().unwrap().locked
            && r.status().unwrap().settings.paused
            && r.status().unwrap().storage_stopped
    );
    assert_eq!(reads(d.path()), 0);
    assert!(r.refresh().is_err());
    r.lock().unwrap().storage_check_at = 0;
    wait_for(|| r.lock().unwrap().storage_check_at != 0).await;
    assert!(r.status().unwrap().storage_stopped);
    assert_eq!(reads(d.path()), 0);
    let sql = rusqlite::Connection::open(&path).unwrap();
    sql.execute_batch("DROP TRIGGER fail_start_publication")
        .unwrap();
    r.lock().unwrap().storage_check_at = 0;
    wait_for(|| !r.status().unwrap().storage_stopped).await;
    assert!(r.status().unwrap().locked && r.status().unwrap().settings.paused);
    assert_eq!(reads(d.path()), 0);
    assert!(!crate::publication::pending(&sql).unwrap());
    assert_eq!(
        sql.query_row("SELECT epoch FROM watch_locator WHERE id='P1'", [], |r| r
            .get::<_, i64>(
            0
        ))
        .unwrap(),
        1
    );
    assert_eq!(
        sql.query_row("SELECT seen FROM seen WHERE source_id='A'", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        sql.query_row(
            "SELECT COUNT(*) FROM activity_intent WHERE state IN ('attempted','unavailable')",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        2
    );
    assert!(r.activity(0).is_err());
    r.shutdown().await;
}

#[tokio::test]
async fn publication_failure_protects_old_source_and_repair_exposes_one_complete_result() {
    let _worker_fixture = WORKER_FIXTURES.lock().await;
    let (d, path) = fixture_store();
    let r = Runtime::start(path.clone(), |_| {}).unwrap();
    wait_for(|| r.activity(0).is_ok_and(|a| a.baseline)).await;
    r.select("a/b".into(), 1).unwrap();
    wait_for(|| r.overview("a/b", 1).is_ok_and(|v| v.data.is_some())).await;
    r.set_pause(true).unwrap();
    wait_for(|| !r.status().unwrap().busy).await;
    let sql = rusqlite::Connection::open(&path).unwrap();
    let count = |q: &str| sql.query_row(q, [], |r| r.get::<_, i64>(0)).unwrap();
    let history = || {
        [
            "activity",
            "seen",
            "dedup",
            "activity_intent",
            "intent_member",
            "alert_attempt",
        ]
        .map(|t| count(&format!("SELECT COUNT(*) FROM {t}")))
    };
    let original = history();
    let old = r
        .page(0)
        .unwrap()
        .items
        .iter()
        .map(|p| p.id.clone())
        .collect::<Vec<_>>();
    let identity = r.status().unwrap().identity;
    let deadline = r.activity(0).unwrap().next_poll_at;
    sql.execute_batch("CREATE TRIGGER fail_publication BEFORE UPDATE ON publication_row WHEN NEW.applied=1 AND OLD.tbl='watch_locator' AND json_extract(OLD.k,'$[0]')='PR_200' BEGIN SELECT RAISE(ABORT,'fixture publication'); END;").unwrap();
    std::fs::write(d.path().join("scenario.json"), r#"{"authored":257}"#).unwrap();
    {
        let mut i = r.lock().unwrap();
        i.store
            .enqueue(
                "discover:authored",
                &Job::Discover(crate::sync::Discovery::new("authored", "")),
                true,
                false,
                0,
                true,
            )
            .unwrap();
    }
    r.notify_sync();
    wait_for(|| r.status().unwrap().storage_stopped).await;
    assert_eq!(
        r.page(0)
            .unwrap()
            .items
            .iter()
            .map(|p| p.id.clone())
            .collect::<Vec<_>>(),
        old
    );
    assert_eq!(history(), original);
    assert!(crate::publication::pending(&sql).unwrap());
    assert!(count("SELECT COUNT(*) FROM publication_row WHERE applied=1") > 128);
    assert!(r.reader_closed().is_err());
    assert!(r.refresh().is_err());
    let stopped = reads(d.path());
    r.lock().unwrap().storage_check_at = 0;
    wait_for(|| r.lock().unwrap().storage_check_at != 0).await;
    assert!(r.status().unwrap().storage_stopped);
    assert_eq!(reads(d.path()), stopped);
    sql.execute(
        "INSERT INTO due VALUES('remote',4070908800) ON CONFLICT(key) DO UPDATE SET at=excluded.at",
        [],
    )
    .unwrap();
    sql.execute_batch("DROP TRIGGER fail_publication").unwrap();
    r.lock().unwrap().storage_check_at = 0;
    wait_for(|| !r.status().unwrap().storage_stopped).await;
    assert_eq!(reads(d.path()), stopped);
    assert_eq!(history(), original);
    assert_eq!(r.status().unwrap().identity, identity);
    assert!(r.status().unwrap().settings.paused);
    assert_eq!(r.activity(0).unwrap().next_poll_at, deadline);
    assert_eq!(
        count("SELECT COUNT(*) FROM watch_source WHERE source='authored'"),
        257
    );
    assert!(!crate::publication::pending(&sql).unwrap());
    assert_eq!(
        count("SELECT COUNT(*) FROM watch_source WHERE source='selected'"),
        0
    );
    assert_eq!(r.status().unwrap().watch_count, 257);
    r.shutdown().await;
    drop(r);
    let r = Runtime::start(path, |_| {}).unwrap();
    assert!(r.status().unwrap().locked && r.status().unwrap().settings.paused);
    assert_eq!(reads(d.path()), stopped);
    r.shutdown().await;
}

#[tokio::test]
async fn full_ceiling_bounded_admission_fault_protects_and_repairs_without_partial_registration() {
    let _worker_fixture = WORKER_FIXTURES.lock().await;
    use crate::content_budget::{
        admission_tests::{n, seed_full_with_rows},
        disposable_bytes,
    };
    let _serial = crate::content_capacity::CAPACITY.lock().await;
    let (d, path) = fixture_store();
    let r = Runtime::start(path.clone(), |_| {}).unwrap();
    wait_for(|| r.activity(0).is_ok_and(|a| a.baseline)).await;
    r.set_pause(true).unwrap();
    wait_for(|| !r.status().unwrap().busy).await;
    let sql = rusqlite::Connection::open(&path).unwrap();
    let ledgers = || {
        [
            "activity",
            "seen",
            "dedup",
            "activity_intent",
            "intent_member",
            "alert_attempt",
        ]
        .map(|t| n(&sql, &format!("SELECT COUNT(*) FROM {t}")))
    };
    let original = ledgers();
    let before = r.status().unwrap();
    let deadline = r.activity(0).unwrap().next_poll_at;
    {
        let mut i = r.lock().unwrap();
        seed_full_with_rows(&mut i.store, 2);
    }
    assert_eq!(disposable_bytes(&sql).unwrap(), crate::store::CONTENT_LIMIT);
    sql.execute_batch("CREATE TRIGGER fail_bounded_admission BEFORE DELETE ON content WHEN OLD.key='lru-0086' BEGIN SELECT RAISE(ABORT,'fixture bounded admission'); END;").unwrap();
    r.select("a/b".into(), 99).unwrap();
    wait_for(|| r.status().unwrap().storage_stopped).await;
    let left = n(
        &sql,
        "SELECT (SELECT COUNT(*) FROM content WHERE key GLOB 'lru-*')+(SELECT COUNT(*) FROM check_context WHERE pr_id GLOB 'lru-*')",
    );
    assert!(
        left < 513 && left > 400,
        "first bounded batch committed: {left}"
    );
    assert_eq!(
        n(&sql, "SELECT COUNT(*) FROM watch_locator WHERE number=99"),
        0
    );
    assert_eq!(
        n(
            &sql,
            "SELECT COUNT(*) FROM content WHERE key='pr:a/b:99' OR key='list:PR_99'"
        ),
        0
    );
    assert_eq!(r.status().unwrap().watch_count, before.watch_count);
    assert!(r.status().unwrap().revision > before.revision + 1);
    assert_eq!(ledgers(), original);
    assert!(r.refresh().is_err());
    let stopped = reads(d.path());
    sql.execute(
        "INSERT INTO due VALUES('remote',4070908800) ON CONFLICT(key) DO UPDATE SET at=excluded.at",
        [],
    )
    .unwrap();
    sql.execute_batch("DROP TRIGGER fail_bounded_admission")
        .unwrap();
    r.lock().unwrap().storage_check_at = 0;
    wait_for(|| !r.status().unwrap().storage_stopped).await;
    assert_eq!(reads(d.path()), stopped);
    assert!(r.status().unwrap().settings.paused);
    assert_eq!(r.status().unwrap().identity, before.identity);
    assert_eq!(r.activity(0).unwrap().next_poll_at, deadline);
    assert_eq!(ledgers(), original);
    sql.execute("DELETE FROM due WHERE key='remote'", [])
        .unwrap();
    r.notify_sync();
    wait_for(|| r.overview("a/b", 99).is_ok_and(|p| p.data.is_some())).await;
    assert_eq!(
        n(&sql, "SELECT COUNT(*) FROM watch_locator WHERE number=99"),
        1
    );
    assert_eq!(ledgers(), original);
    assert!(disposable_bytes(&sql).unwrap() <= crate::store::CONTENT_LIMIT);
    assert!(r.status().unwrap().settings.paused);
    r.shutdown().await;
    drop(r);
    let r = Runtime::start(path, |_| {}).unwrap();
    assert!(r.status().unwrap().locked && r.status().unwrap().settings.paused);
    let stopped = reads(d.path());
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(reads(d.path()), stopped);
    r.shutdown().await;
}

#[tokio::test]
async fn source_initialization_failure_protects_membership_and_retries_fresh_after_repair() {
    let _worker_fixture = WORKER_FIXTURES.lock().await;
    use std::sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    };
    let (d, path) = fixture_store();
    let emitted = Arc::new(AtomicU64::new(0));
    let event = emitted.clone();
    let r = Runtime::start(path.clone(), move |revision| {
        event.store(revision, Ordering::SeqCst);
    })
    .unwrap();
    wait_for(|| r.activity(0).is_ok_and(|a| a.baseline)).await;
    r.set_pause(true).unwrap();
    wait_for(|| !r.status().unwrap().busy).await;
    let sql = rusqlite::Connection::open(&path).unwrap();
    let n = |query: &str| {
        sql.query_row(query, [], |row| row.get::<_, i64>(0))
            .unwrap()
    };
    for i in 0..257 {
        sql.execute(
            "INSERT INTO source_stage VALUES('authored',?,'{}')",
            [format!("old-{i:03}")],
        )
        .unwrap();
    }
    sql.execute_batch("INSERT INTO source_stage VALUES('independent','neighbor','{}');
        CREATE TRIGGER fail_source_init BEFORE DELETE ON source_stage WHEN OLD.id='old-200' BEGIN SELECT RAISE(ABORT,'fixture initialization'); END;").unwrap();
    let before = r.status().unwrap();
    let deadline = r.activity(0).unwrap().next_poll_at;
    let ledgers = || {
        [
            "activity",
            "seen",
            "dedup",
            "activity_intent",
            "intent_member",
            "alert_attempt",
        ]
        .map(|t| n(&format!("SELECT COUNT(*) FROM {t}")))
    };
    let original = ledgers();
    {
        let mut i = r.lock().unwrap();
        i.store
            .enqueue(
                "discover:authored",
                &Job::Discover(crate::sync::Discovery::new("authored", "")),
                true,
                false,
                0,
                true,
            )
            .unwrap();
    }
    r.notify_sync();
    wait_for(|| r.status().unwrap().storage_stopped).await;
    assert_eq!(
        n("SELECT COUNT(*) FROM source_stage WHERE source='authored'"),
        65
    );
    assert_eq!(
        n("SELECT COUNT(*) FROM source_stage WHERE source='independent'"),
        1
    );
    assert_eq!(
        n("SELECT COUNT(*) FROM source_restart WHERE source='authored'"),
        1
    );
    assert_eq!(r.status().unwrap().watch_count, before.watch_count);
    assert_eq!(ledgers(), original);
    assert!(!r.page(0).unwrap().complete);
    assert!(!r.page(0).unwrap().items.is_empty());
    assert!(emitted.load(Ordering::SeqCst) > before.revision);
    assert!(r.refresh().is_err());
    let stopped = reads(d.path());
    sql.execute(
        "INSERT INTO due VALUES('remote',4070908800) ON CONFLICT(key) DO UPDATE SET at=excluded.at",
        [],
    )
    .unwrap();
    sql.execute_batch("DROP TRIGGER fail_source_init").unwrap();
    r.lock().unwrap().storage_check_at = 0;
    wait_for(|| !r.status().unwrap().storage_stopped).await;
    assert_eq!(reads(d.path()), stopped);
    assert!(r.status().unwrap().settings.paused);
    assert_eq!(r.status().unwrap().identity, before.identity);
    assert_eq!(r.activity(0).unwrap().next_poll_at, deadline);
    assert_eq!(ledgers(), original);
    assert_eq!(
        n("SELECT COUNT(*) FROM source_restart WHERE source='authored'"),
        1
    );
    sql.execute("DELETE FROM due WHERE key='remote'", [])
        .unwrap();
    r.notify_sync();
    wait_for(|| r.page(0).is_ok_and(|p| p.complete)).await;
    assert_eq!(
        n("SELECT COUNT(*) FROM source_restart WHERE source='authored'"),
        0
    );
    assert_eq!(
        n("SELECT COUNT(*) FROM source_stage WHERE source='authored'"),
        0
    );
    assert_eq!(
        n("SELECT COUNT(*) FROM source_stage WHERE source='independent'"),
        1
    );
    assert_eq!(r.status().unwrap().watch_count, before.watch_count);
    assert_eq!(ledgers(), original);
    assert!(r.status().unwrap().settings.paused);
    r.shutdown().await;
    drop(r);
    let r = Runtime::start(path, |_| {}).unwrap();
    assert!(r.status().unwrap().locked && r.status().unwrap().settings.paused);
    let stopped = reads(d.path());
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(reads(d.path()), stopped);
    r.shutdown().await;
}

#[tokio::test]
async fn pr_purge_discovery_restarts_before_reading_an_obsolete_page() {
    let _worker_fixture = WORKER_FIXTURES.lock().await;
    let (d, path) = fixture_store();
    let r = Runtime::start(path.clone(), |_| {}).unwrap();
    wait_for(|| r.activity(0).is_ok_and(|a| a.baseline)).await;
    r.set_pause(true).unwrap();
    wait_for(|| !r.status().unwrap().busy).await;
    let sql = rusqlite::Connection::open(&path).unwrap();
    sql.execute_batch("INSERT INTO source_stage SELECT 'authored',substr(key,6),json FROM content WHERE key IN ('list:PR_1','list:PR_2');INSERT INTO source_stage SELECT 'independent',substr(key,6),json FROM content WHERE key='list:PR_2';").unwrap();
    {
        let mut i = r.lock().unwrap();
        i.store.activity_purge("a/b", 1).unwrap();
        assert!(crate::pr_purge::source_restart(&i.store.conn, "authored").unwrap());
        let mut old = crate::sync::Discovery::new("authored", "");
        old.cursor = Some("1".into());
        old.received = 1;
        old.total = Some(2);
        i.store
            .enqueue(
                "discover:authored",
                &Job::Discover(old),
                true,
                false,
                0,
                true,
            )
            .unwrap();
    }
    r.notify_sync();
    wait_for(|| r.page(0).is_ok_and(|p| p.complete && p.items.len() == 1)).await;
    assert_eq!(
        sql.query_row(
            "SELECT COUNT(*) FROM source_restart WHERE source='authored'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    assert_eq!(
        sql.query_row(
            "SELECT COUNT(*) FROM source_stage WHERE source='independent'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    assert_eq!(r.status().unwrap().watch_count, 3);
    assert!(r.status().unwrap().settings.paused);
    assert!(!r.status().unwrap().storage_stopped);
    let reads = std::fs::read_to_string(d.path().join("reads")).unwrap();
    let authored = reads
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .filter(|v| v["query"].as_str().is_some_and(|q| q.contains("Authored")))
        .collect::<Vec<_>>();
    assert!(!authored.is_empty());
    assert!(authored.iter().all(|v| v["variables"]["after"].is_null()));
    r.shutdown().await;
}

#[tokio::test]
async fn pr_purge_partial_startup_is_locked_paused_and_restartable() {
    let _worker_fixture = WORKER_FIXTURES.lock().await;
    let (d, path) = fixture_store();
    let mut s = Store::open(&path).unwrap();
    s.bind(&Identity {
        id: 42,
        login: "reader".into(),
    })
    .unwrap();
    let mut settings = s.settings().unwrap();
    settings.paused = true;
    s.save_settings(&settings).unwrap();
    s.activity_recovery(123).unwrap();
    s.conn.execute_batch("INSERT INTO watch_locator(id,repo,number,head) VALUES('PR_1','a/b',1,'abc');INSERT INTO watch_source VALUES('PR_1','authored');
        INSERT INTO due VALUES('activity:notifications',4070908800);INSERT INTO api_validator VALUES('notifications','etag',NULL,4070908800);
        INSERT INTO activity VALUES('A','a/b',1,'comment',1,1,'eligible',1);INSERT INTO seen VALUES('A',0);INSERT INTO dedup VALUES('A');INSERT INTO activity_intent VALUES(1,'pr','attempted',1);INSERT INTO intent_member VALUES(1,'A');INSERT INTO alert_attempt VALUES('attempt',1);").unwrap();
    s.conn
        .execute(
            "UPDATE activity SET occurred=? WHERE id='A'",
            [now() as i64],
        )
        .unwrap();
    for k in 0..257 {
        s.conn
            .execute(
                "INSERT INTO content VALUES(?,'{}',4070908800,4070908800)",
                [format!("api:repos/a/b/issues/1/comments?fixture={k:03}")],
            )
            .unwrap();
    }
    s.conn.execute_batch("CREATE TRIGGER fail_startup_pr_purge BEFORE DELETE ON content WHEN OLD.key='api:repos/a/b/issues/1/comments?fixture=200' BEGIN SELECT RAISE(ABORT,'fixture startup PR purge'); END;").unwrap();
    assert!(s.activity_purge("a/b", 1).is_err());
    let revision = s.revision().unwrap();
    drop(s);
    let r = Runtime::start(path.clone(), |_| {}).unwrap();
    let status = r.status().unwrap();
    assert!(status.locked && status.settings.paused && status.storage_stopped);
    assert_eq!(status.revision, revision);
    assert_eq!(reads(d.path()), 0);
    assert!(r.overview("a/b", 1).is_err());
    assert!(r.refresh().is_err());
    r.lock().unwrap().storage_check_at = 0;
    wait_for(|| r.lock().unwrap().storage_check_at != 0).await;
    assert!(r.status().unwrap().storage_stopped);
    assert_eq!(reads(d.path()), 0);
    let sql = rusqlite::Connection::open(&path).unwrap();
    sql.execute_batch("DROP TRIGGER fail_startup_pr_purge")
        .unwrap();
    r.lock().unwrap().storage_check_at = 0;
    wait_for(|| !r.status().unwrap().storage_stopped).await;
    let status = r.status().unwrap();
    assert!(status.locked && status.settings.paused);
    assert_eq!(status.identity.unwrap().id, 42);
    assert_eq!(reads(d.path()), 0);
    for (q, expected) in [
        ("SELECT COUNT(*) FROM content", 0),
        ("SELECT COUNT(*) FROM watch_source", 1),
        ("SELECT COUNT(*) FROM activity", 1),
        ("SELECT seen FROM seen", 0),
        (
            "SELECT COUNT(*) FROM activity_intent WHERE state='attempted'",
            1,
        ),
        ("SELECT COUNT(*) FROM alert_attempt", 1),
        ("SELECT boundary FROM activity_cycle", 123),
        (
            "SELECT at FROM due WHERE key='activity:notifications'",
            4070908800,
        ),
        ("SELECT next_at FROM api_validator", 4070908800),
    ] {
        assert_eq!(
            sql.query_row(q, [], |r| r.get::<_, i64>(0)).unwrap(),
            expected,
            "{q}"
        );
    }
    r.shutdown().await;
}

#[tokio::test]
async fn check_purge_worker_denial_protects_and_repairs_paused_without_neighbor_loss() {
    let _worker_fixture = WORKER_FIXTURES.lock().await;
    let (d, path) = fixture_store();
    let emitted = Arc::new(std::sync::atomic::AtomicU64::new(0));
    let event = emitted.clone();
    let r = Runtime::start(path.clone(), move |revision| {
        event.store(revision, std::sync::atomic::Ordering::SeqCst);
    })
    .unwrap();
    wait_for(|| r.activity(0).is_ok_and(|a| a.baseline)).await;
    r.select("a/b".into(), 1).unwrap();
    wait_for(|| r.overview("a/b", 1).is_ok_and(|a| a.data.is_some())).await;
    r.set_pause(true).unwrap();
    wait_for(|| !r.status().unwrap().busy).await;
    let sql = rusqlite::Connection::open(&path).unwrap();
    let count = |sql_text: &str| sql.query_row(sql_text, [], |r| r.get::<_, i64>(0)).unwrap();
    let ledgers = || {
        [
            "activity",
            "seen",
            "dedup",
            "activity_intent",
            "intent_member",
            "alert_attempt",
        ]
        .map(|t| count(&format!("SELECT COUNT(*) FROM {t}")))
    };
    let original = ledgers();
    let before = r.status().unwrap();
    let deadline = r.activity(0).unwrap().next_poll_at;
    let original_contexts = count("SELECT COUNT(*) FROM check_context WHERE pr_id='PR_1'");
    let neighbor_contexts = count("SELECT COUNT(*) FROM check_context WHERE pr_id='PR_2'");
    assert!(neighbor_contexts > 0);
    let target = crate::sync::WatchTarget {
        id: "PR_1".into(),
        repo: "a/b".into(),
        number: 1,
        generation: count("SELECT epoch FROM watch_locator WHERE id='PR_1'") as u64,
    };
    for n in 0..257 {
        sql.execute(
            "INSERT INTO check_context VALUES('PR_1','abc',?,'{}',0)",
            [format!("extra-{n:03}")],
        )
        .unwrap();
    }
    sql.execute_batch("CREATE TRIGGER fail_check_purge BEFORE DELETE ON check_context WHEN OLD.pr_id='PR_1' AND OLD.id='extra-200' BEGIN SELECT RAISE(ABORT,'fixture check purge'); END;").unwrap();
    std::fs::write(d.path().join("scenario.json"), r#"{"deny_checks":true}"#).unwrap();
    {
        let mut i = r.lock().unwrap();
        i.store
            .enqueue(
                "heads:check-denial",
                &Job::Heads(vec![target.clone()]),
                true,
                true,
                0,
                true,
            )
            .unwrap();
    }
    r.notify_sync();
    wait_for(|| r.status().unwrap().storage_stopped).await;
    assert!(!r.status().unwrap().locked);
    assert_eq!(
        count("SELECT COUNT(*) FROM check_context WHERE pr_id='PR_1'"),
        original_contexts + 257 - 168
    );
    assert_eq!(
        count("SELECT epoch FROM watch_locator WHERE id='PR_1'"),
        target.generation as i64 + 1
    );
    assert!(emitted.load(std::sync::atomic::Ordering::SeqCst) > before.revision);
    let denied = r.checks("a/b", 1, 0).unwrap();
    assert!(denied.items.is_empty());
    assert_eq!(denied.error.as_deref(), Some(crate::check_purge::MESSAGE));
    assert_eq!(
        count("SELECT COUNT(*) FROM check_context WHERE pr_id='PR_2'"),
        neighbor_contexts
    );
    assert!(!r.checks("a/b", 2, 0).unwrap().items.is_empty());
    assert!(r.overview("a/b", 1).unwrap().data.is_some());
    assert!(r.refresh().is_err());
    sql.execute(
        "INSERT INTO due VALUES('remote',4070908800) ON CONFLICT(key) DO UPDATE SET at=excluded.at",
        [],
    )
    .unwrap();
    let stopped = reads(d.path());
    r.lock().unwrap().storage_check_at = 0;
    wait_for(|| r.lock().unwrap().storage_check_at != 0).await;
    assert!(r.status().unwrap().storage_stopped);
    assert_eq!(reads(d.path()), stopped);
    sql.execute_batch("DROP TRIGGER fail_check_purge").unwrap();
    r.lock().unwrap().storage_check_at = 0;
    wait_for(|| !r.status().unwrap().storage_stopped).await;
    assert_eq!(reads(d.path()), stopped);
    assert!(r.status().unwrap().settings.paused);
    assert_eq!(r.status().unwrap().identity, before.identity);
    assert_eq!(r.activity(0).unwrap().next_poll_at, deadline);
    assert_eq!(ledgers(), original);
    assert_eq!(
        count("SELECT COUNT(*) FROM check_context WHERE pr_id='PR_1'"),
        0
    );
    assert_eq!(
        count("SELECT COUNT(*) FROM check_context WHERE pr_id='PR_2'"),
        neighbor_contexts
    );
    assert!(!crate::check_purge::any_pending(&r.lock().unwrap().store.conn).unwrap());
    assert!(!r.lock().unwrap().store.deny_checks(&target).unwrap());
    r.shutdown().await;
    drop(r);
    let r = Runtime::start(path, |_| {}).unwrap();
    assert!(r.status().unwrap().locked && r.status().unwrap().settings.paused);
    assert_eq!(reads(d.path()), stopped);
    assert_eq!(ledgers(), original);
    r.shutdown().await;
}

#[tokio::test]
async fn check_purge_partial_startup_stays_locked_and_paused_until_repair() {
    let _worker_fixture = WORKER_FIXTURES.lock().await;
    let (d, path) = fixture_store();
    let mut s = Store::open(&path).unwrap();
    s.bind(&Identity {
        id: 42,
        login: "reader".into(),
    })
    .unwrap();
    let mut settings = s.settings().unwrap();
    settings.paused = true;
    s.save_settings(&settings).unwrap();
    s.activity_recovery(123).unwrap();
    s.conn.execute_batch("INSERT INTO watch_locator(id,repo,number,head) VALUES('PR_1','a/b',1,'abc');INSERT INTO watch_source VALUES('PR_1','authored');
        INSERT INTO check_state VALUES('PR_1','abc',1,1,NULL,4070908800);INSERT INTO due VALUES('activity:notifications',4070908800);INSERT INTO api_validator VALUES('notifications','etag',NULL,4070908800);").unwrap();
    for n in 0..257 {
        s.conn
            .execute(
                "INSERT INTO check_context VALUES('PR_1','abc',?,'{}',0)",
                [format!("extra-{n:03}")],
            )
            .unwrap();
    }
    s.conn.execute_batch("CREATE TRIGGER fail_startup_check_purge BEFORE DELETE ON check_context WHEN OLD.id='extra-200' BEGIN SELECT RAISE(ABORT,'fixture startup check purge'); END;").unwrap();
    assert!(
        s.deny_checks(&crate::sync::WatchTarget {
            id: "PR_1".into(),
            repo: "a/b".into(),
            number: 1,
            generation: 0
        })
        .is_err()
    );
    let revision = s.revision().unwrap();
    drop(s);
    let r = Runtime::start(path.clone(), |_| {}).unwrap();
    let status = r.status().unwrap();
    assert!(status.storage_stopped && status.locked && status.settings.paused);
    assert_eq!(status.revision, revision);
    assert_eq!(reads(d.path()), 0);
    assert!(r.checks("a/b", 1, 0).is_err());
    assert!(r.refresh().is_err());
    r.lock().unwrap().storage_check_at = 0;
    wait_for(|| r.lock().unwrap().storage_check_at != 0).await;
    assert!(r.status().unwrap().storage_stopped);
    assert_eq!(reads(d.path()), 0);
    let sql = rusqlite::Connection::open(&path).unwrap();
    sql.execute_batch("DROP TRIGGER fail_startup_check_purge")
        .unwrap();
    r.lock().unwrap().storage_check_at = 0;
    wait_for(|| !r.status().unwrap().storage_stopped).await;
    let status = r.status().unwrap();
    assert!(status.locked && status.settings.paused);
    assert_eq!(status.identity.unwrap().id, 42);
    assert_eq!(reads(d.path()), 0);
    assert_eq!(
        sql.query_row(
            "SELECT COUNT(*) FROM check_context WHERE pr_id='PR_1'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    assert_eq!(
        sql.query_row("SELECT boundary FROM activity_cycle", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        123
    );
    assert_eq!(
        sql.query_row(
            "SELECT at FROM due WHERE key='activity:notifications'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        4070908800
    );
    assert_eq!(
        sql.query_row("SELECT next_at FROM api_validator", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        4070908800
    );
    r.shutdown().await;
}

#[tokio::test]
async fn clear_partial_failure_repairs_paused_without_reads_or_old_generation_publication() {
    let _worker_fixture = WORKER_FIXTURES.lock().await;
    let (d, path) = fixture_store();
    let emitted = Arc::new(std::sync::atomic::AtomicU64::new(0));
    let event = emitted.clone();
    let r = Runtime::start(path.clone(), move |revision| {
        event.store(revision, std::sync::atomic::Ordering::SeqCst);
    })
    .unwrap();
    wait_for(|| r.activity(0).is_ok_and(|a| a.baseline)).await;
    r.select("a/b".into(), 1).unwrap();
    wait_for(|| r.overview("a/b", 1).is_ok_and(|v| v.data.is_some())).await;
    r.set_pause(true).unwrap();
    wait_for(|| !r.status().unwrap().busy).await;
    let sql = rusqlite::Connection::open(&path).unwrap();
    sql.execute_batch("INSERT INTO due VALUES('remote',4070908800) ON CONFLICT(key) DO UPDATE SET at=excluded.at;").unwrap();
    for n in 0..257 {
        sql.execute("INSERT INTO stage VALUES(?,'{}')", [n.to_string()])
            .unwrap();
        sql.execute(
            "INSERT INTO activity_stage VALUES('a/b',1,?,'{}')",
            [n.to_string()],
        )
        .unwrap();
    }
    sql.execute_batch("CREATE TRIGGER fail_runtime_clear BEFORE DELETE ON activity_stage BEGIN SELECT RAISE(ABORT,'fixture clear_content fault'); END;").unwrap();
    let before = r.status().unwrap();
    let deadline = r.activity(0).unwrap().next_poll_at;
    let ledgers = || {
        [
            "activity",
            "seen",
            "dedup",
            "activity_intent",
            "intent_member",
            "alert_attempt",
        ]
        .map(|table| {
            sql.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| {
                r.get::<_, i64>(0)
            })
            .unwrap()
        })
    };
    let original = ledgers();
    let stopped = reads(d.path());
    let generation = r.lock().unwrap().store.generation().unwrap();
    emitted.store(0, std::sync::atomic::Ordering::SeqCst);
    assert!(r.clear().is_err());
    assert_eq!(
        emitted.load(std::sync::atomic::Ordering::SeqCst),
        before.revision + 1
    );
    assert!(r.status().unwrap().storage_stopped);
    assert_eq!(
        r.lock().unwrap().store.generation().unwrap(),
        generation + 1
    );
    assert_eq!(
        sql.query_row("SELECT COUNT(*) FROM content", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        0
    );
    let remaining = sql
        .query_row("SELECT COUNT(*) FROM stage", [], |r| r.get::<_, i64>(0))
        .unwrap();
    assert!(remaining > 0 && remaining < 257);
    assert!(r.overview("a/b", 1).unwrap().data.is_none());
    assert!(r.checks("a/b", 1, 0).unwrap().items.is_empty());
    assert!(!r.page(0).unwrap().complete);
    assert!(r.refresh().is_err());
    assert_eq!(reads(d.path()), stopped);
    r.lock().unwrap().storage_check_at = 0;
    wait_for(|| r.lock().unwrap().storage_check_at != 0).await;
    assert!(r.status().unwrap().storage_stopped);
    assert_eq!(reads(d.path()), stopped);
    sql.execute_batch("DROP TRIGGER fail_runtime_clear")
        .unwrap();
    r.lock().unwrap().storage_check_at = 0;
    wait_for(|| !r.status().unwrap().storage_stopped).await;
    assert_eq!(reads(d.path()), stopped);
    assert!(r.status().unwrap().settings.paused);
    assert_eq!(r.status().unwrap().identity, before.identity);
    assert_eq!(r.activity(0).unwrap().next_poll_at, deadline);
    assert_eq!(ledgers(), original);
    assert!(!crate::clear_content::pending(&r.lock().unwrap().store.conn).unwrap());
    assert!(
        !r.lock()
            .unwrap()
            .store
            .put("late", &"obsolete", generation)
            .unwrap()
    );
    assert!(r.overview("a/b", 1).unwrap().data.is_none());
    r.shutdown().await;
    drop(r);
    // Restart after repair remains locked and paused until explicit identity
    // validation, with no additional synthetic child invocation.
    let r = Runtime::start(path, |_| {}).unwrap();
    assert!(r.status().unwrap().locked && r.status().unwrap().settings.paused);
    assert_eq!(reads(d.path()), stopped);
    assert_eq!(ledgers(), original);
    r.shutdown().await;
}

#[tokio::test]
async fn invalidation_partial_failure_repairs_paused_without_reads_or_old_generation_publication() {
    let _worker_fixture = WORKER_FIXTURES.lock().await;
    let (d, path) = fixture_store();
    let r = Runtime::start(path.clone(), |_| {}).unwrap();
    wait_for(|| r.activity(0).is_ok_and(|a| a.baseline)).await;
    r.select("a/b".into(), 1).unwrap();
    wait_for(|| r.overview("a/b", 1).is_ok_and(|v| v.data.is_some())).await;
    r.set_pause(true).unwrap();
    wait_for(|| !r.status().unwrap().busy).await;
    let sql = rusqlite::Connection::open(&path).unwrap();
    sql.execute_batch("INSERT INTO due VALUES('remote',4070908800) ON CONFLICT(key) DO UPDATE SET at=excluded.at;").unwrap();
    for n in 0..257 {
        sql.execute("INSERT INTO stage VALUES(?,'{}')", [n.to_string()])
            .unwrap();
        sql.execute(
            "INSERT INTO activity_stage VALUES('a/b',1,?,'{}')",
            [n.to_string()],
        )
        .unwrap();
    }
    sql.execute_batch("CREATE TRIGGER fail_runtime_invalidation BEFORE DELETE ON activity_stage BEGIN SELECT RAISE(ABORT,'fixture invalidation fault'); END;").unwrap();
    let before = r.status().unwrap();
    let deadline = r.activity(0).unwrap().next_poll_at;
    let ledgers = || {
        [
            "activity",
            "seen",
            "dedup",
            "activity_intent",
            "intent_member",
            "alert_attempt",
        ]
        .map(|table| {
            sql.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| {
                r.get::<_, i64>(0)
            })
            .unwrap()
        })
    };
    let original = ledgers();
    let stopped = reads(d.path());
    let generation = r.lock().unwrap().store.generation().unwrap();
    assert!(
        r.configure(
            before.settings.gh_path.clone().unwrap(),
            before.settings.config_dir.clone().unwrap()
        )
        .is_err()
    );
    assert!(r.status().unwrap().storage_stopped);
    assert_eq!(
        r.lock().unwrap().store.generation().unwrap(),
        generation + 1
    );
    assert_eq!(
        sql.query_row("SELECT COUNT(*) FROM stage", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert!(r.overview("a/b", 1).unwrap().data.is_some());
    assert!(
        r.checks("a/b", 1, 0)
            .unwrap()
            .commits
            .iter()
            .all(|c| !c.complete)
    );
    assert!(r.refresh().is_err());
    assert_eq!(reads(d.path()), stopped);
    r.lock().unwrap().storage_check_at = 0;
    wait_for(|| r.lock().unwrap().storage_check_at != 0).await;
    assert!(r.status().unwrap().storage_stopped);
    assert_eq!(reads(d.path()), stopped);
    sql.execute_batch("DROP TRIGGER fail_runtime_invalidation")
        .unwrap();
    r.lock().unwrap().storage_check_at = 0;
    wait_for(|| !r.status().unwrap().storage_stopped).await;
    assert_eq!(reads(d.path()), stopped);
    assert!(r.status().unwrap().settings.paused);
    assert_eq!(r.status().unwrap().identity, before.identity);
    assert_eq!(r.activity(0).unwrap().next_poll_at, deadline);
    assert_eq!(ledgers(), original);
    assert!(!crate::invalidation::pending(&r.lock().unwrap().store.conn).unwrap());
    assert!(
        !r.lock()
            .unwrap()
            .store
            .put("late", &"obsolete", generation)
            .unwrap()
    );
    assert!(r.overview("a/b", 1).unwrap().data.is_some());
    r.shutdown().await;
    drop(r);
    // Restart after repair remains locked and paused until explicit identity
    // validation, with no additional synthetic child invocation.
    let r = Runtime::start(path, |_| {}).unwrap();
    assert!(r.status().unwrap().locked && r.status().unwrap().settings.paused);
    assert_eq!(reads(d.path()), stopped);
    assert_eq!(ledgers(), original);
    r.shutdown().await;
}

#[tokio::test]
async fn enrollment_fault_protects_paused_runtime_and_repair_preserves_history() {
    let _worker_fixture = WORKER_FIXTURES.lock().await;
    let (d, path) = fixture_store();
    let r = Runtime::start(path.clone(), |_| {}).unwrap();
    wait_for(|| r.activity(0).is_ok_and(|a| a.baseline)).await;
    r.set_pause(true).unwrap();
    wait_for(|| !r.status().unwrap().busy).await;
    let sql = rusqlite::Connection::open(&path).unwrap();
    // Keep manual scheduling observable without dispatching synthetic reads.
    sql.execute_batch("INSERT INTO due VALUES('remote',4070908800) ON CONFLICT(key) DO UPDATE SET at=excluded.at;
        INSERT INTO due VALUES('activity:notifications',4070908800) ON CONFLICT(key) DO UPDATE SET at=excluded.at;
        UPDATE activity_cycle SET mode='recovery',boundary=123;").unwrap();
    let tx = sql.unchecked_transaction().unwrap();
    for n in 1..=257 {
        tx.execute(
            "INSERT INTO watch_locator(id,repo,number) VALUES(?,'enroll/repo',?)",
            rusqlite::params![format!("enroll-{n}"), n],
        )
        .unwrap();
        tx.execute(
            "INSERT INTO watch_source VALUES(?,'direct')",
            [format!("enroll-{n}")],
        )
        .unwrap();
    }
    tx.commit().unwrap();
    sql.execute_batch("CREATE TRIGGER fail_enrollment BEFORE INSERT ON activity_target WHEN NEW.repo='enroll/repo' AND NEW.number=200 BEGIN SELECT RAISE(ABORT,'fixture enrollment failure'); END;").unwrap();
    let identity = r.status().unwrap().identity;
    let ledger = || {
        [
            "seen",
            "dedup",
            "activity_intent",
            "intent_member",
            "alert_attempt",
        ]
        .map(|table| {
            sql.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap()
        })
    };
    let history = ledger();
    let stopped_reads = reads(d.path());
    r.refresh().unwrap(); // first bounded enrollment batch
    assert!(r.refresh().is_err()); // second batch fails after earlier commit
    assert!(r.status().unwrap().storage_stopped);
    assert_eq!(
        sql.query_row(
            "SELECT COUNT(*) FROM activity_target WHERE repo='enroll/repo'",
            [],
            |row| row.get::<_, i64>(0)
        )
        .unwrap(),
        128
    );
    assert!(r.refresh().is_err());
    assert_eq!(reads(d.path()), stopped_reads);
    sql.execute_batch("DROP TRIGGER fail_enrollment").unwrap();
    r.lock().unwrap().storage_check_at = 0;
    wait_for(|| !r.status().unwrap().storage_stopped).await;
    assert!(r.status().unwrap().settings.paused);
    assert_eq!(r.status().unwrap().identity, identity);
    assert_eq!(reads(d.path()), stopped_reads);
    assert_eq!(ledger(), history);
    assert_eq!(
        sql.query_row("SELECT boundary FROM activity_cycle", [], |row| row
            .get::<_, i64>(0))
            .unwrap(),
        123
    );
    assert_eq!(r.activity(0).unwrap().next_poll_at, 4070908800);
    // Repair does not enroll or hydrate automatically through Pause. Later
    // explicit passes finish enrollment under the same remote deadline.
    r.refresh().unwrap();
    r.refresh().unwrap();
    assert_eq!(
        sql.query_row(
            "SELECT COUNT(*) FROM activity_target WHERE repo='enroll/repo'",
            [],
            |row| row.get::<_, i64>(0)
        )
        .unwrap(),
        257
    );
    assert_eq!(reads(d.path()), stopped_reads);
    assert_eq!(ledger(), history);
    assert!(r.status().unwrap().settings.paused);
    r.shutdown().await;
}
#[tokio::test]
async fn selected_publication_eviction_fault_protects_and_repairs_without_resuming() {
    let _worker_fixture = WORKER_FIXTURES.lock().await;
    let (d, path) = fixture_store();
    let r = Runtime::start(path.clone(), |_| {}).unwrap();
    wait_for(|| r.activity(0).is_ok_and(|a| a.baseline)).await;
    r.select("a/b".into(), 1).unwrap();
    wait_for(|| r.overview("a/b", 1).is_ok_and(|v| v.data.is_some())).await;
    r.set_pause(true).unwrap();
    wait_for(|| !r.status().unwrap().busy).await;
    let sql = rusqlite::Connection::open(&path).unwrap();
    sql.execute_batch(
        "INSERT INTO content VALUES('expired-eviction','{}',1,1);
        CREATE TRIGGER fail_eviction BEFORE DELETE ON content WHEN OLD.key='expired-eviction'
        BEGIN SELECT RAISE(ABORT,'fixture eviction failure'); END;",
    )
    .unwrap();
    // Manual selection traverses the real child runner and overview parser.
    // Publication must roll back before the worker enters storage protection.
    r.select("a/b".into(), 99).unwrap();
    wait_for(|| r.status().unwrap().storage_stopped).await;
    assert_eq!(
        sql.query_row(
            "SELECT COUNT(*) FROM watch_locator WHERE number=99",
            [],
            |row| row.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    assert_eq!(
        sql.query_row(
            "SELECT COUNT(*) FROM content WHERE key='pr:a/b:99'",
            [],
            |row| row.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    assert!(r.overview("a/b", 1).unwrap().data.is_some());
    assert!(r.refresh().is_err());
    let stopped_reads = reads(d.path());
    r.select("a/b".into(), 1).unwrap();
    assert_eq!(reads(d.path()), stopped_reads);
    sql.execute_batch("DROP TRIGGER fail_eviction").unwrap();
    r.lock().unwrap().storage_check_at = 0;
    wait_for(|| !r.status().unwrap().storage_stopped).await;
    assert!(r.status().unwrap().settings.paused);
    assert_eq!(reads(d.path()), stopped_reads);
    r.select("a/b".into(), 99).unwrap();
    wait_for(|| r.overview("a/b", 99).is_ok_and(|v| v.data.is_some())).await;
    assert!(r.status().unwrap().settings.paused);
    assert_eq!(
        sql.query_row(
            "SELECT COUNT(*) FROM activity_intent WHERE state='pending'",
            [],
            |row| row.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    r.shutdown().await;
}
#[tokio::test]
async fn retention_runs_while_paused_and_failure_protects_until_repair() {
    let _worker_fixture = WORKER_FIXTURES.lock().await;
    let (d, path) = fixture_store();
    let r = Runtime::start(path.clone(), |_| {}).unwrap();
    wait_for(|| r.activity(0).is_ok_and(|a| a.baseline)).await;
    r.set_pause(true).unwrap();
    wait_for(|| !r.status().unwrap().busy).await;
    let stopped_reads = reads(d.path());
    let sql = rusqlite::Connection::open(&path).unwrap();
    let seed = |id: &str| {
        sql.execute(
            "INSERT INTO activity VALUES(?,'a/b',1,'comment',1,1,'eligible',1)",
            [id],
        )
        .unwrap();
        sql.execute("INSERT INTO seen VALUES(?,0)", [id]).unwrap();
        sql.execute("INSERT INTO dedup VALUES(?)", [id]).unwrap();
        sql.execute(
            "INSERT INTO content VALUES(?,'{}',?,?)",
            rusqlite::params![format!("activity-payload:{id}"), now() as i64, now() as i64],
        )
        .unwrap();
    };
    let exists = |id: &str| {
        sql.query_row(
            "SELECT EXISTS(SELECT 1 FROM activity WHERE id=?)",
            [id],
            |r| r.get::<_, bool>(0),
        )
        .unwrap()
    };
    seed("expired-paused");
    r.lock().unwrap().storage_check_at = 0;
    wait_for(|| !exists("expired-paused")).await;
    assert_eq!(reads(d.path()), stopped_reads);
    assert!(r.status().unwrap().settings.paused);
    assert_eq!(
        sql.query_row(
            "SELECT seen FROM seen WHERE source_id='expired-paused'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );

    seed("failed-expiry");
    sql.execute_batch("CREATE TRIGGER fail_retention BEFORE DELETE ON activity BEGIN SELECT RAISE(ABORT,'fixture cleanup failure'); END;").unwrap();
    r.lock().unwrap().storage_check_at = 0;
    wait_for(|| r.status().unwrap().storage_stopped).await;
    assert!(exists("failed-expiry"));
    assert!(r.activity(0).is_ok(), "protected cache remains readable");
    assert!(r.refresh().is_err());
    assert_eq!(reads(d.path()), stopped_reads);
    sql.execute_batch("DROP TRIGGER fail_retention").unwrap();
    r.lock().unwrap().storage_check_at = 0;
    wait_for(|| !r.status().unwrap().storage_stopped).await;
    assert!(!exists("failed-expiry"));
    assert!(r.status().unwrap().settings.paused);
    assert_eq!(reads(d.path()), stopped_reads);
    assert_eq!(
        sql.query_row(
            "SELECT COUNT(*) FROM activity_intent WHERE state='pending'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    r.shutdown().await;
}
#[tokio::test]
async fn paused_check_reclamation_failure_rolls_back_and_repairs_without_reads() {
    let _worker_fixture = WORKER_FIXTURES.lock().await;
    let (d, path) = fixture_store();
    let r = Runtime::start(path.clone(), |_| {}).unwrap();
    wait_for(|| r.activity(0).is_ok_and(|a| a.baseline)).await;
    r.set_pause(true).unwrap();
    wait_for(|| !r.status().unwrap().busy).await;
    let stopped_reads = reads(d.path());
    let sql = rusqlite::Connection::open(&path).unwrap();
    sql.execute_batch("INSERT INTO check_state VALUES('expired','sha',1,1,NULL,1);
        INSERT INTO check_context VALUES('expired','sha','ctx','{}',0);
        INSERT INTO content VALUES('expired-check-companion','{}',1,1);
        CREATE TRIGGER fail_check_retention BEFORE DELETE ON check_context BEGIN SELECT RAISE(ABORT,'fixture check cleanup failure'); END;").unwrap();
    r.lock().unwrap().storage_check_at = 0;
    wait_for(|| r.status().unwrap().storage_stopped).await;
    assert!(r.refresh().is_err());
    assert_eq!(
        sql.query_row(
            "SELECT COUNT(*) FROM check_state WHERE pr_id='expired'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    assert_eq!(
        sql.query_row(
            "SELECT COUNT(*) FROM content WHERE key='expired-check-companion'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    assert_eq!(reads(d.path()), stopped_reads);
    sql.execute_batch("DROP TRIGGER fail_check_retention")
        .unwrap();
    r.lock().unwrap().storage_check_at = 0;
    wait_for(|| !r.status().unwrap().storage_stopped).await;
    assert!(r.status().unwrap().settings.paused);
    assert_eq!(reads(d.path()), stopped_reads);
    assert_eq!(
        sql.query_row(
            "SELECT COUNT(*) FROM check_context WHERE pr_id='expired'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    assert_eq!(
        sql.query_row(
            "SELECT COUNT(*) FROM check_state WHERE pr_id='expired'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    assert_eq!(
        sql.query_row(
            "SELECT COUNT(*) FROM activity_intent WHERE state='pending'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    r.shutdown().await;
}
#[tokio::test]
async fn metadata_cleanup_fault_repairs_paused_and_rehydrates_unconditionally() {
    let _worker_fixture = WORKER_FIXTURES.lock().await;
    let (d, path) = fixture_store();
    std::fs::write(d.path().join("scenario.json"), r#"{"validators":true}"#).unwrap();
    let r = Runtime::start(path.clone(), |_| {}).unwrap();
    wait_for(|| r.activity(0).is_ok_and(|a| a.baseline)).await;
    r.set_pause(true).unwrap();
    wait_for(|| !r.status().unwrap().busy).await;
    let stopped_reads = reads(d.path());
    let activity_count = r.activity(0).unwrap().items.len();
    let sql = rusqlite::Connection::open(&path).unwrap();
    let url = "repos/a/b/issues/1/comments?per_page=50&page=1";
    assert!(
        sql.query_row(
            "SELECT etag IS NOT NULL FROM api_validator WHERE url=?",
            [url],
            |r| r.get::<_, bool>(0)
        )
        .unwrap()
    );
    sql.execute(
        "UPDATE content SET accessed=1 WHERE key=?",
        [format!("api:{url}")],
    )
    .unwrap();
    sql.execute_batch("INSERT INTO check_state VALUES('metadata-orphan','sha',1,1,NULL,1);
        CREATE TRIGGER fail_metadata BEFORE DELETE ON check_state BEGIN SELECT RAISE(ABORT,'fixture metadata failure'); END;").unwrap();
    let deadline = r.activity(0).unwrap().next_poll_at;
    r.lock().unwrap().storage_check_at = 0;
    wait_for(|| r.status().unwrap().storage_stopped).await;
    assert!(r.refresh_activity("a/b", 1).is_err());
    assert_eq!(reads(d.path()), stopped_reads);
    for (table, column, value) in [
        ("content", "key", format!("api:{url}")),
        ("resource_state", "key", format!("api:{url}")),
        ("api_validator", "url", url.into()),
        ("check_state", "pr_id", "metadata-orphan".into()),
    ] {
        assert_eq!(
            sql.query_row(
                &format!("SELECT COUNT(*) FROM {table} WHERE {column}=?"),
                [&value],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            1
        );
    }
    sql.execute_batch("DROP TRIGGER fail_metadata").unwrap();
    r.lock().unwrap().storage_check_at = 0;
    wait_for(|| !r.status().unwrap().storage_stopped).await;
    assert!(r.status().unwrap().settings.paused);
    assert_eq!(reads(d.path()), stopped_reads);
    assert_eq!(r.activity(0).unwrap().next_poll_at, deadline);
    assert_eq!(
        sql.query_row(
            "SELECT COUNT(*) FROM api_validator WHERE url=?",
            [url],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    assert_eq!(
        sql.query_row(
            "SELECT COUNT(*) FROM resource_state WHERE key=?",
            [format!("api:{url}")],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    assert_eq!(
        sql.query_row(
            "SELECT COUNT(*) FROM check_state WHERE pr_id='metadata-orphan'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    // Explicit reads after repair repopulate reclaimed source data, without
    // replaying Activity or resuming monitoring or native delivery.
    r.refresh_activity("a/b", 1).unwrap();
    wait_for(|| reads(d.path()) > stopped_reads + 14 && !r.status().unwrap().busy).await;
    let log = std::fs::read_to_string(d.path().join("reads")).unwrap();
    let requests = log
        .lines()
        .skip(stopped_reads)
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
        .filter(|v| {
            v["args"]
                .as_array()
                .unwrap()
                .iter()
                .any(|a| a.as_str() == Some(url))
        })
        .collect::<Vec<_>>();
    assert_eq!(requests.len(), 1);
    assert!(
        !requests[0]["args"]
            .as_array()
            .unwrap()
            .iter()
            .any(|a| a.as_str().is_some_and(
                |s| s.starts_with("If-None-Match:") || s.starts_with("If-Modified-Since:")
            ))
    );
    assert!(
        sql.query_row(
            "SELECT etag IS NOT NULL FROM api_validator WHERE url=?",
            [url],
            |r| r.get::<_, bool>(0)
        )
        .unwrap()
    );
    assert_eq!(r.activity(0).unwrap().items.len(), activity_count);
    assert!(r.status().unwrap().settings.paused);
    assert_eq!(
        sql.query_row(
            "SELECT COUNT(*) FROM activity_intent WHERE state='pending'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    r.shutdown().await;
}
#[tokio::test]
async fn all_writes_denied_preserves_cached_navigation_and_repair_does_not_replay() {
    let _worker_fixture = WORKER_FIXTURES.lock().await;
    let (d, path) = fixture_store();
    let r = Runtime::start(path.clone(), |_| {}).unwrap();
    wait_for(|| r.activity(0).is_ok_and(|a| a.baseline)).await;
    r.select("a/b".into(), 1).unwrap();
    wait_for(|| r.overview("a/b", 1).is_ok_and(|v| v.data.is_some())).await;
    r.set_pause(true).unwrap();
    wait_for(|| !r.status().unwrap().busy).await;
    let before = r.overview("a/b", 1).unwrap();
    let poll = r.activity(0).unwrap().next_poll_at;
    let revision = r.status().unwrap().revision;
    // Actual SQLite READONLY responses on every write of this runtime's
    // connection, rather than a fault on one statement or table.
    r.lock()
        .unwrap()
        .store
        .conn
        .execute_batch("PRAGMA query_only=ON")
        .unwrap();
    // A check read discovers the denied access-time write and retries protected.
    assert!(r.checks("a/b", 1, 0).is_ok());
    assert!(r.status().unwrap().storage_stopped);
    assert_eq!(
        r.overview("a/b", 1).unwrap().data.unwrap().head,
        before.data.unwrap().head
    );
    assert!(r.status().unwrap().storage_stopped);
    assert!(r.page(0).unwrap().complete);
    assert!(r.requested_page(0).unwrap().complete);
    assert!(r.activity(0).unwrap().baseline);
    assert!(r.checks("a/b", 1, 0).is_ok());
    r.select("a/b".into(), 1).unwrap();
    assert!(r.refresh().is_err());
    assert!(r.refresh_activity("a/b", 1).is_err());
    assert!(r.set_pause(false).is_err());
    assert!(r.set_theme(Theme::default()).is_err());
    assert!(r.mark_activity_seen(&["missing".into()]).is_err());
    assert!(r.clear().is_err());
    assert!(r.status().unwrap().settings.paused);
    assert_eq!(r.status().unwrap().revision, revision);
    let count = reads(d.path());
    r.lock().unwrap().storage_check_at = 0;
    tokio::time::sleep(Duration::from_millis(1200)).await;
    assert!(r.status().unwrap().storage_stopped);
    assert_eq!(reads(d.path()), count);
    r.lock()
        .unwrap()
        .store
        .conn
        .execute_batch("PRAGMA query_only=OFF")
        .unwrap();
    r.lock().unwrap().storage_check_at = 0;
    wait_for(|| !r.status().unwrap().storage_stopped).await;
    assert!(r.status().unwrap().settings.paused);
    assert_eq!(r.activity(0).unwrap().next_poll_at, poll);
    assert_eq!(reads(d.path()), count);
    r.shutdown().await;
    let reopened = Store::open(&path).unwrap();
    assert!(reopened.settings().unwrap().paused);
    assert_eq!(
        reopened
            .conn
            .query_row(
                "SELECT COUNT(*) FROM activity_intent WHERE state='pending'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
}
#[tokio::test]
async fn low_space_restart_defers_writes_and_preserves_pause_identity_and_deadlines() {
    let _worker_fixture = WORKER_FIXTURES.lock().await;
    use std::sync::atomic::{AtomicBool, Ordering};
    let (d, path) = fixture_store();
    let r = Runtime::start(path.clone(), |_| {}).unwrap();
    wait_for(|| r.activity(0).is_ok_and(|a| a.baseline)).await;
    r.set_pause(true).unwrap();
    r.shutdown().await;
    drop(r);
    let mut store = Store::open(&path).unwrap();
    // Model an interrupted recovery, whose original boundary must survive.
    store.activity_recovery(now()).unwrap();
    let identity = store.identity().unwrap();
    let revision = store.revision().unwrap();
    let poll = store.due("activity:notifications").unwrap();
    let boundary: i64 = store
        .conn
        .query_row("SELECT boundary FROM activity_cycle", [], |r| r.get(0))
        .unwrap();
    drop(store);
    let before = std::fs::read(&path).unwrap();
    let count = reads(d.path());
    let healthy = Arc::new(AtomicBool::new(false));
    let probe_health = healthy.clone();
    let check: StorageCheck = Arc::new(move |_| {
        if probe_health.load(Ordering::SeqCst) {
            Ok(())
        } else {
            Err(Error::StorageUnavailable(
                "Fixture: below free-space threshold".into(),
            ))
        }
    });
    let r = Runtime::start_checked(path.clone(), |_| {}, check).unwrap();
    let s = r.status().unwrap();
    assert!(s.storage_stopped && s.locked && s.settings.paused);
    assert_eq!(s.identity, identity);
    assert_eq!(s.revision, revision);
    assert!(s.storage.managed_bytes >= before.len() as u64);
    assert!(
        r.page(0).is_err(),
        "storage protection cannot bypass startup identity lock"
    );
    assert!(r.refresh().is_err());
    assert!(r.set_pause(false).is_err());
    tokio::time::sleep(Duration::from_millis(1200)).await;
    assert_eq!(reads(d.path()), count);
    assert_eq!(std::fs::read(&path).unwrap(), before);
    healthy.store(true, Ordering::SeqCst);
    r.lock().unwrap().storage_check_at = 0;
    wait_for(|| !r.status().unwrap().storage_stopped).await;
    assert!(r.status().unwrap().settings.paused);
    assert_eq!(
        reads(d.path()),
        count,
        "repair does not bypass Pause for identity reads"
    );
    assert_eq!(
        r.lock()
            .unwrap()
            .store
            .due("activity:notifications")
            .unwrap(),
        poll
    );
    assert_eq!(
        r.lock()
            .unwrap()
            .store
            .conn
            .query_row("SELECT boundary FROM activity_cycle", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        boundary
    );
    r.refresh().unwrap();
    wait_for(|| !r.status().unwrap().locked && r.page(0).is_ok_and(|p| p.complete)).await;
    assert!(r.status().unwrap().settings.paused);
    assert_eq!(r.activity(0).unwrap().next_poll_at, poll);
    r.shutdown().await;
}
#[tokio::test]
async fn partial_startup_cleanup_stays_locked_and_paused_until_repaired() {
    let _worker_fixture = WORKER_FIXTURES.lock().await;
    let (d, path) = fixture_store();
    let first = Runtime::start(path.clone(), |_| {}).unwrap();
    wait_for(|| first.activity(0).is_ok_and(|a| a.baseline)).await;
    first.set_pause(true).unwrap();
    wait_for(|| !first.status().unwrap().busy).await;
    first.shutdown().await;
    let stopped_reads = reads(d.path());
    let sql = rusqlite::Connection::open(&path).unwrap();
    for n in 0..129 {
        sql.execute("INSERT INTO stage VALUES(?,'{}')", [n.to_string()])
            .unwrap();
    }
    sql.execute_batch("INSERT INTO source_stage VALUES('authored','unfinished','{}');
        UPDATE activity_cycle SET mode='recovery',boundary=123;
        UPDATE due SET at=4070908800 WHERE key='activity:notifications';
        CREATE TRIGGER fail_partial_cleanup BEFORE DELETE ON source_stage BEGIN SELECT RAISE(ABORT,'fixture late cleanup fault'); END;").unwrap();
    let ledger_counts = || {
        [
            "seen",
            "dedup",
            "watch_locator",
            "activity_intent",
            "intent_member",
            "alert_attempt",
        ]
        .map(|table| {
            sql.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| {
                r.get::<_, i64>(0)
            })
            .unwrap()
        })
    };
    let before = ledger_counts();
    let r = Runtime::start(path.clone(), |_| {}).unwrap();
    assert!(r.status().unwrap().storage_stopped);
    assert!(r.status().unwrap().settings.paused);
    assert!(r.status().unwrap().locked);
    assert!(r.page(0).is_err());
    assert!(r.refresh().is_err());
    // Two 64-row transactions committed; the failing next batch
    // rolled back its companion final stage row.
    assert_eq!(
        sql.query_row("SELECT COUNT(*) FROM stage", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
    r.lock().unwrap().storage_check_at = 0;
    wait_for(|| r.lock().unwrap().storage_check_at != 0).await;
    assert!(r.status().unwrap().storage_stopped);
    assert_eq!(reads(d.path()), stopped_reads);
    assert_eq!(
        sql.query_row("SELECT COUNT(*) FROM stage", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
    sql.execute_batch("DROP TRIGGER fail_partial_cleanup")
        .unwrap();
    r.lock().unwrap().storage_check_at = 0;
    wait_for(|| !r.status().unwrap().storage_stopped).await;
    for table in ["stage", "source_stage"] {
        assert_eq!(
            sql.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
    assert_eq!(ledger_counts(), before);
    assert!(r.status().unwrap().settings.paused);
    assert!(r.status().unwrap().locked);
    assert_eq!(reads(d.path()), stopped_reads);
    assert_eq!(
        sql.query_row("SELECT boundary FROM activity_cycle", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        123
    );
    assert_eq!(
        sql.query_row(
            "SELECT at FROM due WHERE key='activity:notifications'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        4070908800
    );
    r.refresh().unwrap();
    wait_for(|| !r.status().unwrap().locked && r.page(0).is_ok_and(|p| p.complete)).await;
    assert!(r.status().unwrap().settings.paused);
    assert_eq!(r.activity(0).unwrap().next_poll_at, 4070908800);
    assert_eq!(
        sql.query_row(
            "SELECT COUNT(*) FROM activity_intent WHERE state='pending'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    r.shutdown().await;
}
#[tokio::test]
async fn partial_recovery_initialization_blocks_reads_and_repairs_without_resuming() {
    let _worker_fixture = WORKER_FIXTURES.lock().await;
    let (d, path) = fixture_store();
    let first = Runtime::start(path.clone(), |_| {}).unwrap();
    wait_for(|| first.activity(0).is_ok_and(|a| a.baseline)).await;
    first.set_pause(true).unwrap();
    wait_for(|| !first.status().unwrap().busy).await;
    first.shutdown().await;
    let stopped_reads = reads(d.path());
    let sql = rusqlite::Connection::open(&path).unwrap();
    sql.execute_batch(
        "DELETE FROM activity_target;
        UPDATE activity_cycle SET mode='normal',boundary=1;
        UPDATE due SET at=4070908800 WHERE key='activity:notifications';",
    )
    .unwrap();
    for n in 0..257 {
        sql.execute("INSERT INTO activity_target VALUES('a/b',?,9,1)", [n])
            .unwrap();
    }
    sql.execute_batch("CREATE TRIGGER fail_recovery BEFORE UPDATE ON activity_target WHEN OLD.number=256 BEGIN SELECT RAISE(ABORT,'fixture late recovery fault'); END;").unwrap();
    let r = Runtime::start(path.clone(), |_| {}).unwrap();
    let boundary: i64 = sql
        .query_row("SELECT boundary FROM activity_cycle", [], |r| r.get(0))
        .unwrap();
    assert!(boundary > 1);
    assert!(r.status().unwrap().storage_stopped);
    assert!(r.status().unwrap().locked);
    assert!(r.status().unwrap().settings.paused);
    assert!(r.page(0).is_err());
    assert!(r.refresh().is_err());
    assert_eq!(
        sql.query_row(
            "SELECT COUNT(*) FROM activity_target WHERE done=1",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    r.lock().unwrap().storage_check_at = 0;
    wait_for(|| r.lock().unwrap().storage_check_at != 0).await;
    assert!(r.status().unwrap().storage_stopped);
    assert_eq!(reads(d.path()), stopped_reads);
    sql.execute_batch("DROP TRIGGER fail_recovery").unwrap();
    r.lock().unwrap().storage_check_at = 0;
    wait_for(|| !r.status().unwrap().storage_stopped).await;
    assert!(r.status().unwrap().settings.paused);
    assert!(r.status().unwrap().locked);
    assert_eq!(reads(d.path()), stopped_reads);
    assert_eq!(
        sql.query_row(
            "SELECT COUNT(*) FROM activity_target WHERE done!=0 OR due!=0",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    assert_eq!(
        sql.query_row("SELECT boundary FROM activity_cycle", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        boundary
    );
    assert_eq!(
        sql.query_row(
            "SELECT COUNT(*) FROM activity_intent WHERE state='pending'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    r.refresh().unwrap();
    wait_for(|| !r.status().unwrap().locked && r.page(0).is_ok_and(|p| p.complete)).await;
    assert!(r.status().unwrap().settings.paused);
    assert_eq!(r.activity(0).unwrap().next_poll_at, 4070908800);
    r.shutdown().await;
}
#[tokio::test]
async fn startup_cleanup_failure_opens_protected_cache_until_cleanup_and_probe_succeed() {
    let _worker_fixture = WORKER_FIXTURES.lock().await;
    let (_d, path) = fixture_store();
    let sql = rusqlite::Connection::open(&path).unwrap();
    sql.execute_batch("INSERT INTO stage VALUES('incomplete','{}');
        CREATE TRIGGER fail_cleanup BEFORE DELETE ON stage BEGIN SELECT RAISE(ABORT,'fixture cleanup fault'); END;").unwrap();
    let r = Runtime::start(path.clone(), |_| {}).unwrap();
    assert!(r.status().unwrap().storage_stopped);
    assert!(r.lock().unwrap().startup_deferred);
    assert_eq!(
        sql.query_row("SELECT COUNT(*) FROM stage", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
    r.lock().unwrap().storage_check_at = 0;
    tokio::time::sleep(Duration::from_millis(1200)).await;
    assert!(r.status().unwrap().storage_stopped);
    sql.execute_batch("DROP TRIGGER fail_cleanup").unwrap();
    r.lock().unwrap().storage_check_at = 0;
    wait_for(|| !r.status().unwrap().storage_stopped && !r.status().unwrap().locked).await;
    assert_eq!(
        sql.query_row("SELECT COUNT(*) FROM stage", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        0
    );
    r.shutdown().await;
}
#[test]
fn durable_selection_reserves_oldest_background_for_three_foreground_dispatches() {
    let d = tempfile::tempdir().unwrap();
    let mut s = Store::open(&d.path().join("db")).unwrap();
    for n in 0..140 {
        s.enqueue(
            &format!("persist:{n}"),
            &Job::Overview("a/b".into(), n),
            true,
            true,
            0,
            false,
        )
        .unwrap();
    }
    for n in [1000, 1001] {
        s.enqueue(
            &format!("persist:bg:{n}"),
            &Job::Overview("a/b".into(), n),
            false,
            false,
            0,
            false,
        )
        .unwrap();
    }
    let before = s.conn.total_changes();
    let page = s.pending(1).unwrap();
    assert_eq!(
        page.iter().map(|r| r.0).collect::<Vec<_>>(),
        (1..=127).chain([141]).collect::<Vec<_>>()
    );
    assert_eq!(s.conn.total_changes(), before);
    let mut q = Queue::default();
    for (seq, _, job, manual, foreground) in page {
        assert!(q.push(Work {
            ticket: Some(seq),
            job: serde_json::from_str(&job).unwrap(),
            manual,
            foreground
        }));
    }
    for seq in [1, 2, 3, 141, 4] {
        let work = q.pop().unwrap();
        assert_eq!(work.ticket, Some(seq));
        assert_eq!(work.foreground, seq != 141);
    }
    assert_eq!(
        s.conn
            .query_row("SELECT COUNT(*) FROM pending", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        142
    );
}
#[test]
fn queue_bounds_coalescing_and_fairness() {
    let mut q = Queue::default();
    for n in 0..128 {
        assert!(q.push(Work {
            ticket: None,
            job: Job::Overview("o/r".into(), n),
            manual: true,
            foreground: n != 127
        }));
    }
    assert!(!q.push(Work {
        ticket: None,
        job: Job::Bootstrap,
        manual: false,
        foreground: false
    }));
    assert!(q.push(Work {
        ticket: None,
        job: Job::Overview("o/r".into(), 0),
        manual: true,
        foreground: true
    }));
    for _ in 0..3 {
        assert!(q.pop().unwrap().foreground)
    }
    assert!(!q.pop().unwrap().foreground);
    assert_eq!(q.jobs.len(), 124);
}

#[tokio::test]
async fn authored_tray_discovers_both_stacks_without_selecting_their_members() {
    let _worker_fixture = WORKER_FIXTURES.lock().await;
    let (d, path) = fixture_store();
    std::fs::write(
        d.path().join("scenario.json"),
        r#"{"authored":6,"direct_ids":[],"no_teams":true,"stacks":{"5":[1,2,3],"9":[4,5,6]}}"#,
    )
    .unwrap();
    let r = Runtime::start(path, |_| {}).unwrap();
    wait_for(|| {
        r.page(0)
            .is_ok_and(|page| page.complete && page.items.len() == 6)
    })
    .await;
    wait_for(|| r.page(0).is_ok_and(|page| page.stacks.len() == 2)).await;
    let mut numbers = r
        .page(0)
        .unwrap()
        .stacks
        .iter()
        .map(|entry| match entry.stack {
            StackState::Published { number, .. } => number,
            _ => panic!("expected published stack"),
        })
        .collect::<Vec<_>>();
    numbers.sort_unstable();
    assert_eq!(numbers, vec![5, 9]);
    assert!(r.lock().unwrap().selected.is_none());
    // Explicit refresh must bypass the slow membership cadence and remove a
    // stack that is no longer published, without disturbing its neighbor.
    std::fs::write(
        d.path().join("scenario.json"),
        r#"{"authored":6,"direct_ids":[],"no_teams":true,"stacks":{"5":[1,2,3]}}"#,
    )
    .unwrap();
    r.refresh().unwrap();
    wait_for(|| {
        r.page(0).is_ok_and(|page| {
            page.stacks.len() == 1
                && matches!(
                    page.stacks[0].stack,
                    StackState::Published { number: 5, .. }
                )
        })
    })
    .await;
    r.shutdown().await;
}

#[tokio::test]
async fn initial_tray_page_includes_all_related_stacks() {
    let _worker_fixture = WORKER_FIXTURES.lock().await;
    for view in ["authored", "requested", "query"] {
        let (d, path) = fixture_store();
        let ids = (1..=6).collect::<Vec<_>>();
        let scenario = serde_json::json!({
            "authored": if view == "authored" {6} else {0},
            "direct_ids": if view == "requested" {ids.clone()} else {vec![]},
            "query_ids": ids, "no_teams":true, "slow_stack":0.1,
            "stacks":{"5":[1,2,3],"9":[4,5,6]}
        });
        std::fs::write(d.path().join("scenario.json"), scenario.to_string()).unwrap();
        let r = Runtime::start(path, |_| {}).unwrap();
        wait_for(|| r.status().is_ok_and(|status| !status.locked)).await;
        let stacks = if view == "query" {
            let query = r
                .save_query(None, None, "Test".into(), "repo:a/b".into())
                .unwrap();
            r.request_query(query.id, query.version, 1).unwrap();
            wait_for(|| {
                r.query_page(query.id, query.version, 1)
                    .is_ok_and(|page| page.items.len() == 6)
            })
            .await;
            let page = r
                .prepared_query_page(query.id, query.version, 1)
                .await
                .unwrap();
            assert!(page.error.is_none(), "{:?}", page.error);
            assert_eq!(
                r.status().unwrap().watch_count,
                0,
                "reading stacks must not enroll search results"
            );
            page.stacks
        } else {
            wait_for(|| {
                match view {
                    "authored" => r.page(0),
                    _ => r.requested_page(0),
                }
                .is_ok_and(|page| page.complete && page.items.len() == 6)
            })
            .await;
            let page = r.prepared_page(view, 0).await.unwrap();
            assert!(page.error.is_none(), "{:?}", page.error);
            page.stacks
        };
        assert_eq!(
            stacks.len(),
            2,
            "the first {view} page response must include both stacks"
        );
        assert!(r.lock().unwrap().selected.is_none());
        // A warm reread must not start another membership traversal.
        let before = std::fs::read_to_string(d.path().join("reads"))
            .unwrap()
            .lines()
            .filter(|line| line.contains("/stacks"))
            .count();
        if view != "query" {
            let page = r.prepared_page(view, 0).await.unwrap();
            assert_eq!(page.stacks.len(), 2);
        }
        let after = std::fs::read_to_string(d.path().join("reads"))
            .unwrap()
            .lines()
            .filter(|line| line.contains("/stacks"))
            .count();
        assert_eq!(before, after);
        r.shutdown().await;
    }
}

#[tokio::test]
async fn unavailable_stack_metadata_keeps_prs_readable_and_pause_keeps_reads_local() {
    let _worker_fixture = WORKER_FIXTURES.lock().await;
    let (d, path) = fixture_store();
    std::fs::write(
        d.path().join("scenario.json"),
        r#"{"authored":1,"direct_ids":[],"no_teams":true,"deny_stacks":true}"#,
    )
    .unwrap();
    let r = Runtime::start(path, |_| {}).unwrap();
    wait_for(|| {
        r.page(0)
            .is_ok_and(|page| page.complete && page.items.len() == 1)
    })
    .await;
    let page = r.prepared_page("authored", 0).await.unwrap();
    assert_eq!(page.items.len(), 1);
    assert!(
        page.error
            .as_ref()
            .is_some_and(|error| error.contains("Stack information is unavailable"))
    );
    r.set_pause(true).unwrap();
    wait_for(|| r.status().is_ok_and(|status| !status.busy)).await;
    let before = reads(d.path());
    let page = r.prepared_page("authored", 0).await.unwrap();
    assert_eq!(page.items.len(), 1);
    assert_eq!(reads(d.path()), before);
    r.shutdown().await;
}
