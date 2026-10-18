//! A logical PR barrier precedes bounded, restartable resource reclamation.
use crate::{
    Error, Result, now,
    store::{bump, storage_healthy},
    sync::WatchTarget,
};
use rusqlite::{Connection, OptionalExtension, params};
use std::path::Path;
const BATCH: usize = 128;
pub(crate) const MESSAGE: &str = "Resource unavailable";
pub(crate) fn pending(c: &Connection, repo: &str, number: u64) -> Result<bool> {
    Ok(c.query_row(
        "SELECT EXISTS(SELECT 1 FROM pr_purge WHERE repo=? COLLATE NOCASE AND number=?)",
        params![repo, number as i64],
        |r| r.get(0),
    )?)
}
pub(crate) fn any_pending(c: &Connection) -> Result<bool> {
    Ok(c.query_row("SELECT EXISTS(SELECT 1 FROM pr_purge)", [], |r| r.get(0))?)
}
pub(crate) fn source_restart(c: &Connection, source: &str) -> Result<bool> {
    Ok(c.query_row(
        "SELECT EXISTS(SELECT 1 FROM source_restart WHERE source=?)",
        [source],
        |r| r.get(0),
    )?)
}
// SQL expressions supplied here are static application code, never user text.
fn locator(repo: &str, number: &str) -> String {
    format!("SELECT id FROM watch_locator WHERE repo={repo} COLLATE NOCASE AND number={number}")
}
fn staged(repo: &str, number: &str) -> String {
    format!(
        "(json_extract(json,'$.repo')={repo} COLLATE NOCASE AND json_extract(json,'$.number')={number}) OR id IN ({})",
        locator(repo, number)
    )
}
fn payload(key: &str, repo: &str, number: &str) -> String {
    format!("({key}='pr:'||{repo}||':'||{number} COLLATE NOCASE
      OR {key}='activity-owner:'||{repo}||':'||{number} COLLATE NOCASE
      OR {key} IN (SELECT 'list:'||id FROM watch_locator WHERE repo={repo} COLLATE NOCASE AND number={number})
      OR {key} IN (SELECT 'list:'||id FROM source_stage WHERE {})
      OR {key} IN (SELECT 'activity-payload:'||id FROM activity WHERE repo={repo} COLLATE NOCASE AND number={number})
      OR (substr({key},1,length('api:repos/'||{repo}||'/pulls/'||{number}))='api:repos/'||{repo}||'/pulls/'||{number} COLLATE NOCASE AND substr({key},length('api:repos/'||{repo}||'/pulls/'||{number})+1,1) IN ('','/','?'))
      OR (substr({key},1,length('api:repos/'||{repo}||'/issues/'||{number}))='api:repos/'||{repo}||'/issues/'||{number} COLLATE NOCASE AND substr({key},length('api:repos/'||{repo}||'/issues/'||{number})+1,1) IN ('','/','?')))",staged(repo,number))
}
pub(crate) fn payload_hidden(c: &Connection, key: &str) -> Result<bool> {
    Ok(c.query_row(
        &format!(
            "SELECT EXISTS(SELECT 1 FROM pr_purge p WHERE {})",
            payload("?1", "p.repo", "p.number")
        ),
        [key],
        |r| r.get(0),
    )?)
}
fn admit(c: &Connection) -> Result<()> {
    storage_healthy(
        c.path()
            .and_then(|p| Path::new(p).parent())
            .unwrap_or(Path::new("/")),
    )
}
fn begin(
    c: &mut Connection,
    repo: &str,
    number: u64,
    target: Option<&WatchTarget>,
) -> Result<bool> {
    if pending(c, repo, number)? {
        return Ok(true);
    }
    if !crate::github::valid_repo(repo) || number == 0 || number > i64::MAX as u64 {
        return Err(Error::Safe("Invalid PR purge locator.".into()));
    }
    admit(c)?;
    let tx = c.transaction()?;
    if let Some(t) = target {
        let valid:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM watch_locator WHERE id=? AND repo=? COLLATE NOCASE AND number=? AND epoch=?)",params![t.id,repo,number as i64,t.generation as i64],|r|r.get(0))?;
        if !valid {
            return Ok(false);
        }
    }
    tx.execute(
        "UPDATE meta SET value=CAST(value AS INTEGER)+1 WHERE key='generation'",
        [],
    )?;
    tx.execute(
        "INSERT INTO pr_purge SELECT ?,?,CAST(value AS INTEGER),? FROM meta WHERE key='generation'",
        params![repo.to_ascii_lowercase(), number as i64, now() as i64 + 60],
    )?;
    tx.execute("INSERT INTO resource_state(key,complete,error) VALUES(?,0,'Resource unavailable') ON CONFLICT(key) DO UPDATE SET complete=0,error=excluded.error",[format!("pr:{}:{number}",repo.to_ascii_lowercase())])?;
    bump(&tx)?;
    tx.commit()?;
    #[cfg(test)]
    point("barrier_committed");
    Ok(true)
}
pub(crate) fn purge(
    c: &mut Connection,
    repo: &str,
    number: u64,
    target: Option<&WatchTarget>,
) -> Result<bool> {
    if !begin(c, repo, number, target)? {
        return Ok(false);
    }
    resume(c)?;
    Ok(true)
}
pub(crate) fn resume(c: &mut Connection) -> Result<()> {
    while any_pending(c)? {
        admit(c)?;
        batch(c)?;
    }
    Ok(())
}
fn batch(c: &mut Connection) -> Result<usize> {
    let tx = c.transaction()?;
    let marker: Option<(String, i64, i64, i64)> = tx
        .query_row(
            "SELECT repo,number,generation,retry_at FROM pr_purge ORDER BY repo,number LIMIT 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .optional()?;
    let Some((repo, number, generation, retry)) = marker else {
        return Ok(0);
    };
    let ids = locator("?1", "?2");
    let stages = staged("?1", "?2");
    let payloads = payload("key", "?1", "?2");
    let mut changed = 0;
    #[cfg(test)]
    let mut payload_changed = false;
    for sql in [
        format!("INSERT INTO source_restart SELECT DISTINCT source FROM source_stage WHERE ({stages}) AND source NOT IN (SELECT source FROM source_restart) ORDER BY source LIMIT ?3"),
        format!("DELETE FROM content WHERE rowid IN (SELECT rowid FROM content WHERE {payloads} ORDER BY rowid LIMIT ?3)"),
        format!("UPDATE resource_state SET complete=0,error='Resource unavailable' WHERE rowid IN (SELECT rowid FROM resource_state WHERE {payloads} AND (complete!=0 OR error IS NOT 'Resource unavailable') ORDER BY rowid LIMIT ?3)"),
        "DELETE FROM activity_stage WHERE rowid IN (SELECT rowid FROM activity_stage WHERE repo=?1 COLLATE NOCASE AND number=?2 ORDER BY rowid LIMIT ?3)".into(),
        format!("DELETE FROM stage WHERE rowid IN (SELECT rowid FROM stage WHERE {stages} ORDER BY rowid LIMIT ?3)"),
        format!("DELETE FROM source_stage WHERE rowid IN (SELECT rowid FROM source_stage WHERE {stages} ORDER BY rowid LIMIT ?3)"),
        format!("DELETE FROM check_context WHERE rowid IN (SELECT rowid FROM check_context WHERE pr_id IN ({ids}) ORDER BY rowid LIMIT ?3)"),
        format!("DELETE FROM pending WHERE rowid IN (SELECT rowid FROM pending WHERE key='pr:'||?1||':'||?2 COLLATE NOCASE OR key='activity:pr:'||?1||':'||?2 COLLATE NOCASE OR EXISTS(SELECT 1 FROM watch_locator l WHERE l.id IN ({ids}) AND substr(pending.key,1,length(l.id)+8)='checks:'||l.id||':') OR (json_extract(job,'$.Overview[0]')=?1 COLLATE NOCASE AND json_extract(job,'$.Overview[1]')=?2) OR (json_extract(job,'$.Stacks[0]')=?1 COLLATE NOCASE AND json_extract(job,'$.Stacks[1]')=?2) OR (json_extract(job,'$.StackDetail[0]')=?1 COLLATE NOCASE AND json_extract(job,'$.StackDetail[1]')=?2) OR (json_extract(job,'$.Activity.Hydrate.repo')=?1 COLLATE NOCASE AND json_extract(job,'$.Activity.Hydrate.number')=?2) ORDER BY rowid LIMIT ?3)"),
        format!("UPDATE check_state SET complete=0,error='Resource unavailable' WHERE rowid IN (SELECT rowid FROM check_state WHERE pr_id IN ({ids}) AND (complete!=0 OR error IS NOT 'Resource unavailable') ORDER BY rowid LIMIT ?3)"),
        // The generation watermark makes per-alias epoch updates restartable.
        "UPDATE watch_locator SET epoch=epoch+1,purged_generation=?4,error='Resource unavailable',review=NULL,due=?5 WHERE rowid IN (SELECT rowid FROM watch_locator WHERE repo=?1 COLLATE NOCASE AND number=?2 AND purged_generation!=?4 ORDER BY rowid LIMIT ?3)".into(),
        "UPDATE activity_target SET done=0 WHERE rowid IN (SELECT rowid FROM activity_target WHERE repo=?1 COLLATE NOCASE AND number=?2 AND done!=0 ORDER BY rowid LIMIT ?3)".into(),
        format!("DELETE FROM check_purge WHERE rowid IN (SELECT rowid FROM check_purge WHERE pr_id IN ({ids}) ORDER BY rowid LIMIT ?3)"),
    ] {
        let mut stmt=tx.prepare(&sql)?;
        // Most statements need only the shared first three parameters.
        let count=stmt.parameter_count();
        let rows=if count==5{stmt.execute(params![repo,number,(BATCH-changed) as i64,generation,retry])?}else{stmt.execute(params![repo,number,(BATCH-changed) as i64])?};
        #[cfg(test)]
        {payload_changed |= sql.starts_with("DELETE FROM content") && rows>0;}
        changed+=rows;
        if changed==BATCH{break;}
    }
    let finished = changed < BATCH;
    if finished {
        changed += tx.execute(
            "DELETE FROM pr_purge WHERE repo=? AND number=?",
            params![repo, number],
        )?;
    }
    #[cfg(test)]
    point(if finished {
        "finish_before_commit"
    } else {
        "batch_before_commit"
    });
    #[cfg(test)]
    if payload_changed {
        point("payload_before_commit");
    }
    tx.commit()?;
    #[cfg(test)]
    if payload_changed {
        point("payload_after_commit");
    }
    #[cfg(test)]
    point(if finished {
        "finish_after_commit"
    } else {
        "batch_after_commit"
    });
    Ok(changed)
}
#[cfg(test)]
fn point(phase: &str) {
    if std::env::var("GITJUKE_TEST_PR_PURGE_PHASE").as_deref() == Ok(phase) {
        std::fs::write(
            std::env::var_os("GITJUKE_TEST_PR_PURGE_MARKER").unwrap(),
            phase,
        )
        .unwrap();
        loop {
            std::thread::park();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{model::PullRequest, store::Store};
    fn n(c: &Connection, sql: &str) -> i64 {
        c.query_row(sql, [], |r| r.get(0)).unwrap()
    }
    fn target() -> WatchTarget {
        WatchTarget {
            id: "P_1".into(),
            repo: "a/b".into(),
            number: 1,
            generation: 0,
        }
    }
    fn pr(id: &str, number: u64) -> PullRequest {
        PullRequest {
            id: id.into(),
            repo_id: Some(1),
            repo: "a/b".into(),
            number,
            title: "Title".into(),
            url: format!("https://github.com/a/b/pull/{number}"),
            state: "open".into(),
            draft: false,
            author: None,
            updated_at: "2026-10-17T00:00:00Z".into(),
        }
    }
    fn seed(s: &mut Store) {
        crate::check_purge::tests::seed(s);
        let tx = s.conn.transaction().unwrap();
        for i in 0..257 {
            let id = if i == 0 {
                "P_1".into()
            } else {
                format!("alias-{i:03}")
            };
            let json = serde_json::to_string(&pr(&id, 1)).unwrap();
            if i > 0 {
                tx.execute(
                    "INSERT INTO watch_locator(id,repo,number,head) VALUES(?,'A/B',1,'sha-256')",
                    [&id],
                )
                .unwrap();
                tx.execute(
                    "INSERT INTO content VALUES(?,?,4070908800,4070908800)",
                    params![format!("list:{id}"), json],
                )
                .unwrap();
            }
            tx.execute("INSERT INTO stage VALUES(?,?)", params![id, json])
                .unwrap();
            tx.execute(
                "INSERT INTO source_stage VALUES(?,?,?)",
                params![format!("source-{i:03}"), id, json],
            )
            .unwrap();
            let api = format!("api:repos/a/b/pulls/1/comments?fixture={i:03}");
            tx.execute(
                "INSERT INTO content VALUES(?,'{}',4070908800,4070908800)",
                [&api],
            )
            .unwrap();
            tx.execute("INSERT INTO resource_state VALUES(?,1,1,NULL)", [&api])
                .unwrap();
        }
        tx.execute(
            "INSERT INTO source_stage VALUES('neighbor','P_10',?)",
            [serde_json::to_string(&pr("P_10", 10)).unwrap()],
        )
        .unwrap();
        tx.execute(
            "INSERT INTO content VALUES('list:P_10',?,4070908800,4070908800)",
            [serde_json::to_string(&pr("P_10", 10)).unwrap()],
        )
        .unwrap();
        tx.commit().unwrap();
    }
    fn preserved(s: &Store, r: u64) {
        assert_eq!(s.revision().unwrap(), r);
        assert_eq!(s.generation().unwrap(), 1);
        assert_eq!(s.identity().unwrap().unwrap().id, 42);
        assert!(s.settings().unwrap().paused);
        for (t, c) in [
            ("activity", 259),
            ("seen", 259),
            ("dedup", 259),
            ("progress", 2),
            ("activity_intent", 2),
            ("intent_member", 2),
            ("alert_attempt", 1),
            ("watch_source", 2),
            ("watch_locator", 258),
        ] {
            assert_eq!(n(&s.conn, &format!("SELECT COUNT(*) FROM {t}")), c, "{t}");
        }
        assert_eq!(n(&s.conn, "SELECT COUNT(*) FROM seen WHERE seen!=0"), 0);
        assert_eq!(
            n(
                &s.conn,
                "SELECT COUNT(*) FROM activity_intent WHERE state IN ('attempted','unavailable')"
            ),
            2
        );
        assert_eq!(n(&s.conn, "SELECT boundary FROM activity_cycle"), 123);
        assert_eq!(s.due("activity:notifications").unwrap(), 4070908800);
        assert_eq!(n(&s.conn, "SELECT next_at FROM api_validator"), 4070908800);
        assert_eq!(n(&s.conn, "SELECT due FROM activity_target"), 4070908800);
        assert_eq!(
            n(&s.conn, "SELECT epoch FROM watch_locator WHERE id='P_10'"),
            0
        );
        assert_eq!(
            n(
                &s.conn,
                "SELECT complete FROM check_state WHERE pr_id='P_10'"
            ),
            1
        );
        assert_eq!(
            n(
                &s.conn,
                "SELECT COUNT(*) FROM check_context WHERE pr_id='P_10' AND staged=0"
            ),
            1
        );
        assert_eq!(
            n(
                &s.conn,
                "SELECT COUNT(*) FROM content WHERE key IN ('activity-payload:neighbor','list:P_10')"
            ),
            2
        );
    }
    fn hidden(s: &mut Store) {
        for key in [
            "pr:a/b:1",
            "list:P_1",
            "activity-payload:comment",
            "activity-payload:check-256",
            "api:repos/A/B/pulls/1/comments?fixture=200",
        ] {
            let p = s.get::<serde_json::Value>(key).unwrap();
            assert!(p.data.is_none() && !p.complete, "{key}");
        }
        let p = s.check_page("a/b", 1, 0).unwrap();
        assert!(p.items.is_empty() && p.pr.is_none());
        assert_eq!(p.error.as_deref(), Some(MESSAGE));
        assert!(
            s.activity_check_records("a/b", 1, true, "sha-256")
                .unwrap()
                .is_empty()
        );
        assert!(
            s.watch_page(0)
                .unwrap()
                .items
                .iter()
                .filter(|w| w.number == 1)
                .all(|w| w.error.as_deref() == Some(MESSAGE))
        );
        let page = s.list_page("authored", 0).unwrap();
        assert!(page.items.is_empty() && !page.complete);
        let other = s.list_page("requested", 0).unwrap();
        assert_eq!(other.items.len(), 1);
        assert_eq!(other.items[0].number, 10);
        assert_eq!(s.check_page("a/b", 10, 0).unwrap().items.len(), 1);
        let a = s.activity_page(0).unwrap();
        assert!(
            a.items
                .iter()
                .filter(|a| a.number == 1)
                .all(|a| a.text.is_none() && a.url.is_empty() && !a.seen)
        );
        assert!(a.items.iter().any(|a| a.number == 10 && a.text.is_some()));
    }
    fn complete(s: &Store) {
        assert!(!any_pending(&s.conn).unwrap());
        assert_eq!(n(&s.conn, "SELECT COUNT(*) FROM content"), 2);
        assert_eq!(n(&s.conn, "SELECT COUNT(*) FROM source_restart"), 257);
        assert_eq!(
            n(
                &s.conn,
                "SELECT COUNT(*) FROM source_stage WHERE source!='neighbor'"
            ),
            0
        );
        for t in ["stage", "activity_stage"] {
            assert_eq!(n(&s.conn, &format!("SELECT COUNT(*) FROM {t}")), 0);
        }
        assert_eq!(
            n(
                &s.conn,
                "SELECT COUNT(*) FROM check_context WHERE pr_id='P_1'"
            ),
            0
        );
        assert_eq!(
            n(
                &s.conn,
                "SELECT COUNT(*) FROM check_state WHERE pr_id='P_1' AND complete=0 AND error='Resource unavailable'"
            ),
            257
        );
        assert_eq!(
            n(
                &s.conn,
                "SELECT COUNT(*) FROM watch_locator WHERE number=1 AND epoch=1 AND purged_generation=1"
            ),
            257
        );
        assert_eq!(n(&s.conn, "SELECT done FROM activity_target"), 0);
        assert_eq!(
            n(
                &s.conn,
                "SELECT COUNT(*) FROM resource_state WHERE key LIKE 'api:%' AND complete=0 AND error='Resource unavailable'"
            ),
            257
        );
    }
    #[test]
    fn pr_purge_global_bound_immediate_visibility_and_durable_isolation() {
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("db");
        let mut s = Store::open(&path).unwrap();
        seed(&mut s);
        let r = s.revision().unwrap() + 1;
        let changes = s.conn.total_changes();
        assert!(begin(&mut s.conn, "a/b", 1, Some(&target())).unwrap());
        assert_eq!(s.conn.total_changes() - changes, 4);
        assert_eq!(n(&s.conn, "SELECT COUNT(*) FROM content"), 775);
        hidden(&mut s);
        let mut protected = Store::open_cached(&path).unwrap();
        hidden(&mut protected);
        assert!(!s.put("pr:a/b:1", &"late", 0).unwrap());
        assert!(!s.put("pr:a/b:1", &"during", 1).unwrap());
        assert!(s.activity_stage("a/b", 1, &[], false).is_err());
        let mut batches = 0;
        while any_pending(&s.conn).unwrap() {
            let before = s.conn.total_changes();
            let changed = batch(&mut s.conn).unwrap();
            assert!(changed > 0 && changed <= 128);
            assert_eq!(s.conn.total_changes() - before, changed as u64);
            preserved(&s, r);
            batches += 1;
        }
        assert_eq!(batches, 25);
        complete(&s);
        hidden(&mut s);
        assert_eq!(
            n(
                &s.conn,
                "SELECT COUNT(*) FROM pending WHERE key='checks:P_10:sha' OR key='heads:P_1'"
            ),
            2
        );
        let before = s.conn.total_changes();
        s.purge_watch(&target()).unwrap();
        resume(&mut s.conn).unwrap();
        assert_eq!(s.conn.total_changes(), before);
        assert!(s.publish_source("source-000").is_err());
        assert_eq!(s.watch_page(0).unwrap().items.len(), 2);
        // The unaffected source may finish; the affected source must begin again.
        s.publish_source("neighbor").unwrap();
        s.begin_source("source-000").unwrap();
        s.stage_source("source-000", &[pr("P_1", 1)]).unwrap();
        s.publish_source("source-000").unwrap();
        assert!(!source_restart(&s.conn, "source-000").unwrap());
        assert!(
            s.get::<serde_json::Value>("list:P_1")
                .unwrap()
                .data
                .is_some()
        );
        assert!(
            s.get::<serde_json::Value>("pr:a/b:1")
                .unwrap()
                .data
                .is_none()
        );
    }
    #[test]
    fn pr_purge_barrier_later_batch_and_finish_faults_resume() {
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("db");
        let mut s = Store::open(&path).unwrap();
        seed(&mut s);
        let r = s.revision().unwrap();
        s.conn.execute_batch("CREATE TRIGGER fail_barrier BEFORE INSERT ON pr_purge BEGIN SELECT RAISE(ABORT,'fixture barrier'); END;").unwrap();
        assert!(s.activity_purge("a/b", 1).is_err());
        assert_eq!(s.revision().unwrap(), r);
        assert_eq!(s.generation().unwrap(), 0);
        assert!(!any_pending(&s.conn).unwrap());
        assert_eq!(n(&s.conn, "SELECT COUNT(*) FROM content"), 775);
        s.conn.execute_batch("DROP TRIGGER fail_barrier; CREATE TRIGGER fail_payload BEFORE DELETE ON content WHEN OLD.key='activity-payload:check-200' BEGIN SELECT RAISE(ABORT,'fixture payload'); END;").unwrap();
        assert!(s.activity_purge("a/b", 1).is_err());
        preserved(&s, r + 1);
        assert_eq!(n(&s.conn, "SELECT COUNT(*) FROM source_restart"), 257);
        assert_eq!(
            n(
                &s.conn,
                "SELECT COUNT(*) FROM content WHERE key GLOB 'activity-payload:check-*'"
            ),
            132
        );
        assert!(s.activity_purge("A/B", 1).is_err());
        hidden(&mut s);
        drop(s);
        assert!(Store::open(&path).is_err());
        let c = Connection::open(&path).unwrap();
        c.execute_batch("DROP TRIGGER fail_payload; CREATE TRIGGER fail_finish BEFORE DELETE ON pr_purge BEGIN SELECT RAISE(ABORT,'fixture finish'); END;").unwrap();
        assert!(Store::open(&path).is_err());
        assert_eq!(
            n(
                &c,
                "SELECT COUNT(*) FROM watch_locator WHERE number=1 AND purged_generation=0"
            ),
            15
        );
        assert_eq!(n(&c, "SELECT done FROM activity_target"), 1);
        let mut protected = Store::open_cached(&path).unwrap();
        hidden(&mut protected);
        preserved(&protected, r + 1);
        c.execute_batch("DROP TRIGGER fail_finish").unwrap();
        drop(c);
        let mut s = Store::open(&path).unwrap();
        complete(&s);
        preserved(&s, r + 1);
        hidden(&mut s);
    }
    #[test]
    fn pr_purge_unknown_pr_overlap_and_recovery_barriers() {
        let d = tempfile::tempdir().unwrap();
        let mut s = Store::open(&d.path().join("db")).unwrap();
        s.activity_recovery(123).unwrap();
        s.conn.execute_batch("UPDATE activity_cycle SET notifications_done=1;INSERT INTO resource_state VALUES('authored',1,1,NULL),('direct',1,1,NULL),('teams',1,1,NULL);").unwrap();
        assert!(begin(&mut s.conn, "a_/b_", 1, None).unwrap());
        let r = s.revision().unwrap();
        s.activity_finish_recovery(false).unwrap();
        assert_eq!(s.revision().unwrap(), r);
        assert_eq!(
            n(
                &s.conn,
                "SELECT COUNT(*) FROM activity_cycle WHERE mode='recovery'"
            ),
            1
        );
        resume(&mut s.conn).unwrap();
        s.activity_finish_recovery(false).unwrap();
        assert_eq!(
            n(
                &s.conn,
                "SELECT COUNT(*) FROM activity_cycle WHERE mode='normal'"
            ),
            1
        );
        s.conn.execute_batch("INSERT INTO watch_locator(id,repo,number) VALUES('P','a_/b_',1);INSERT INTO check_purge VALUES('P');").unwrap();
        s.activity_purge("a_/b_", 1).unwrap();
        assert!(!crate::check_purge::any_pending(&s.conn).unwrap());
        assert!(s.activity_purge("a_/b_", 0).is_err());
    }
    #[test]
    fn crash_child_entry() {
        let Some(path) = std::env::var_os("GITJUKE_TEST_PR_PURGE_DB") else {
            return;
        };
        let conn = Connection::open(Path::new(&path)).unwrap();
        conn.pragma_update(None, "synchronous", "FULL").unwrap();
        Store {
            conn,
            cache_writes: true,
        }
        .activity_purge("a/b", 1)
        .unwrap();
        panic!("child missed PR purge barrier");
    }
    #[test]
    fn pr_purge_sigkill_barrier_and_cleanup_preserve_only_committed_batches() {
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
        for (phase, restarts, content, old_epochs, marked) in [
            ("barrier_committed", 0, 775, 257, true),
            ("batch_before_commit", 0, 775, 257, true),
            ("batch_after_commit", 128, 775, 257, true),
            ("payload_before_commit", 256, 775, 257, true),
            ("payload_after_commit", 257, 648, 257, true),
            ("finish_before_commit", 257, 2, 15, true),
            ("finish_after_commit", 257, 2, 0, false),
        ] {
            let d = tempfile::tempdir().unwrap();
            let path = d.path().join("db");
            let marker = d.path().join("marker");
            let mut s = Store::open(&path).unwrap();
            seed(&mut s);
            let r = s.revision().unwrap() + 1;
            drop(s);
            let mut child = OwnedChild(
                Command::new(std::env::current_exe().unwrap())
                    .args([
                        "--exact",
                        "pr_purge::tests::crash_child_entry",
                        "--nocapture",
                    ])
                    .env("GITJUKE_TEST_PR_PURGE_DB", &path)
                    .env("GITJUKE_TEST_PR_PURGE_PHASE", phase)
                    .env("GITJUKE_TEST_PR_PURGE_MARKER", &marker)
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
            assert_eq!(
                raw.query_row("PRAGMA quick_check(1)", [], |r| r.get::<_, String>(0))
                    .unwrap(),
                "ok"
            );
            assert_eq!(any_pending(&raw).unwrap(), marked);
            assert_eq!(n(&raw, "SELECT COUNT(*) FROM source_restart"), restarts);
            assert_eq!(n(&raw, "SELECT COUNT(*) FROM content"), content);
            assert_eq!(
                n(
                    &raw,
                    "SELECT COUNT(*) FROM watch_locator WHERE number=1 AND purged_generation=0"
                ),
                old_epochs
            );
            assert_eq!(
                n(
                    &raw,
                    "SELECT COUNT(*) FROM pending WHERE key='checks:P_10:sha' OR key='heads:P_1'"
                ),
                2
            );
            let mut protected = Store::open_cached(&path).unwrap();
            preserved(&protected, r);
            hidden(&mut protected);
            drop(protected);
            drop(raw);
            let mut s = Store::open(&path).unwrap();
            complete(&s);
            preserved(&s, r);
            hidden(&mut s);
        }
    }
}
