//! A per-PR epoch barrier hides denied checks before restartable reclamation.
use crate::{
    Result, now,
    store::{bump, storage_healthy},
    sync::WatchTarget,
};
use rusqlite::{Connection, OptionalExtension, params};
use std::path::Path;

const BATCH: usize = 128;
pub(crate) const MESSAGE: &str = "Checks unavailable";
pub(crate) fn pending(c: &Connection, id: &str) -> Result<bool> {
    Ok(c.query_row(
        "SELECT EXISTS(SELECT 1 FROM check_purge WHERE pr_id=?)",
        [id],
        |r| r.get(0),
    )?)
}
pub(crate) fn any_pending(c: &Connection) -> Result<bool> {
    Ok(c.query_row("SELECT EXISTS(SELECT 1 FROM check_purge)", [], |r| r.get(0))?)
}
pub(crate) fn payload_hidden(c: &Connection, key: &str) -> Result<bool> {
    Ok(c.query_row("SELECT EXISTS(SELECT 1 FROM activity a JOIN watch_locator l ON l.repo=a.repo COLLATE NOCASE AND l.number=a.number JOIN check_purge p ON p.pr_id=l.id WHERE a.id=substr(?1,18) AND ?1='activity-payload:'||a.id AND a.kind='check')",[key],|r|r.get(0))?)
}
fn admit(c: &Connection) -> Result<()> {
    storage_healthy(
        c.path()
            .and_then(|p| Path::new(p).parent())
            .unwrap_or(Path::new("/")),
    )
}
fn begin(c: &mut Connection, target: &WatchTarget) -> Result<bool> {
    if pending(c, &target.id)? {
        return Ok(true);
    }
    admit(c)?;
    let tx = c.transaction()?;
    let changed=tx.execute("UPDATE watch_locator SET epoch=epoch+1,error='Checks unavailable',due=? WHERE id=? AND epoch=?",params![now() as i64+60,target.id,target.generation as i64])?;
    if changed == 0 {
        return Ok(false);
    }
    tx.execute("INSERT INTO check_purge VALUES(?)", [&target.id])?;
    bump(&tx)?;
    tx.commit()?;
    #[cfg(test)]
    point("barrier_committed");
    Ok(true)
}
pub(crate) fn deny(c: &mut Connection, target: &WatchTarget) -> Result<bool> {
    if !begin(c, target)? {
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
    let batch_before = tx.total_changes();
    // Process one identity per transaction, never an unbounded marker list.
    let id: Option<String> = tx
        .query_row(
            "SELECT pr_id FROM check_purge ORDER BY pr_id LIMIT 1",
            [],
            |r| r.get(0),
        )
        .optional()?;
    let Some(id) = id else {
        return Ok(0);
    };
    let mut changed = 0;
    let mut exhausted = false;
    for sql in [
        "DELETE FROM check_context WHERE rowid IN (SELECT rowid FROM check_context WHERE pr_id=? ORDER BY rowid LIMIT ?)",
        "DELETE FROM content WHERE rowid IN (SELECT c.rowid FROM content c JOIN activity a ON c.key='activity-payload:'||a.id JOIN watch_locator l ON l.repo=a.repo COLLATE NOCASE AND l.number=a.number WHERE l.id=? AND a.kind='check' ORDER BY c.rowid LIMIT ?)",
        "DELETE FROM activity_stage WHERE rowid IN (SELECT a.rowid FROM activity_stage a JOIN watch_locator l ON l.repo=a.repo COLLATE NOCASE AND l.number=a.number WHERE l.id=? AND json_extract(a.json,'$.kind')='check' ORDER BY a.rowid LIMIT ?)",
        "DELETE FROM pending WHERE rowid IN (SELECT rowid FROM pending WHERE substr(key,1,length(?1)+8)='checks:'||?1||':' ORDER BY rowid LIMIT ?2)",
        "UPDATE check_state SET complete=0,error='Checks unavailable' WHERE rowid IN (SELECT rowid FROM check_state WHERE pr_id=? AND (complete!=0 OR error IS NOT 'Checks unavailable') ORDER BY rowid LIMIT ?)",
    ] {
        let limit = (BATCH - changed) / crate::shared_storage::cost(sql);
        if limit == 0 {
            exhausted = true;
            break;
        }
        let before = tx.total_changes();
        let rows = tx.execute(sql, params![id, limit as i64])?;
        changed += (tx.total_changes() - before) as usize;
        if rows == limit {
            exhausted = true;
            break;
        }
    }
    let finished = !exhausted;
    if finished {
        changed += tx.execute("DELETE FROM check_purge WHERE pr_id=?", [id])?;
    }
    #[cfg(test)]
    point(if finished {
        "finish_before_commit"
    } else {
        "batch_before_commit"
    });
    crate::shared_storage::bounded(&tx, batch_before)?;
    tx.commit()?;
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
    if std::env::var("GITJUKE_TEST_CHECK_PURGE_PHASE").as_deref() == Ok(phase) {
        std::fs::write(
            std::env::var_os("GITJUKE_TEST_CHECK_PURGE_MARKER").unwrap(),
            phase,
        )
        .unwrap();
        loop {
            std::thread::park();
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::{
        activity::ActivityRecord,
        model::{Identity, PullRequest},
        store::Store,
        sync::{CheckTarget, ContextPage, HeadRead, HeadSnapshot},
    };
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
    fn pr() -> PullRequest {
        PullRequest {
            id: "P_1".into(),
            repo_id: Some(1),
            repo: "a/b".into(),
            number: 1,
            title: "Retained PR".into(),
            url: "https://github.com/a/b/pull/1".into(),
            state: "open".into(),
            draft: false,
            author: Some("reader".into()),
            updated_at: "2026-10-17T00:00:00Z".into(),
        }
    }
    fn page() -> ContextPage {
        ContextPage {
            contexts: vec![],
            next: None,
            total: 0,
            commit: "sha-256".into(),
        }
    }
    fn check(t: WatchTarget) -> CheckTarget {
        CheckTarget {
            watch: t,
            head: "sha-256".into(),
            commit: "sha-256".into(),
            cursor: String::new(),
            total: 0,
            page_size: 25,
        }
    }
    fn head(t: &WatchTarget) -> HeadRead {
        HeadRead {
            snapshot: HeadSnapshot {
                pr: pr(),
                head: "sha-256".into(),
                merge: None,
                review: None,
                generation: t.generation,
                fetched_at: now(),
            },
            pages: vec![page()],
        }
    }
    fn record(id: String, repo: &str, number: u64, kind: &str) -> ActivityRecord {
        ActivityRecord {
            id,
            repo: repo.into(),
            number,
            kind: kind.into(),
            occurred: now(),
            actor: None,
            url: format!("https://github.com/{repo}/pull/{number}/checks"),
            text: Some("Retained excerpt".into()),
            eligible: true,
            reason: "eligible".into(),
            seen: false,
            head: Some("sha-256".into()),
        }
    }
    pub(crate) fn seed(s: &mut Store) {
        s.bind(&Identity {
            id: 42,
            login: "reader".into(),
        })
        .unwrap();
        let mut settings = s.settings().unwrap();
        settings.paused = true;
        s.save_settings(&settings).unwrap();
        s.activity_recovery(123).unwrap();
        let tx = s.conn.transaction().unwrap();
        tx.execute_batch("INSERT INTO watch_locator(id,repo,number,head) VALUES('P_1','a/b',1,'sha-256'),('P_10','a/b',10,'sha');
            INSERT INTO watch_source VALUES('P_1','authored'),('P_10','direct');
            INSERT INTO activity_target VALUES('a/b',1,4070908800,1);
            INSERT INTO activity_intent VALUES(1,'pr','attempted',1),(2,'summary','unavailable',1);
            INSERT INTO intent_member VALUES(1,'check-000'),(2,'check-000');INSERT INTO alert_attempt VALUES('attempt',1);
            INSERT INTO progress VALUES('activity:baseline','1'),('catchup:check-000','check-000');
            INSERT INTO api_validator VALUES('notifications','etag',NULL,4070908800);
            INSERT INTO due VALUES('activity:notifications',4070908800);
            INSERT INTO resource_state VALUES('authored',1,1,NULL),('direct',1,1,NULL),('teams',1,1,NULL);
            INSERT INTO content VALUES('pr:a/b:1','{\"body\":\"Retained overview\"}',4070908800,4070908800);
            INSERT INTO pending(key,job,manual,foreground,at) VALUES('checks:P_10:sha','{}',1,1,0),('heads:P_1','{}',1,1,0);").unwrap();
        tx.execute(
            "INSERT INTO content VALUES('list:P_1',?,?,?)",
            params![
                serde_json::to_string(&pr()).unwrap(),
                now() as i64,
                now() as i64
            ],
        )
        .unwrap();
        for i in 0..257 {
            let id = format!("check-{i:03}");
            let sha = format!("sha-{i}");
            let r = record(id.clone(), "A/B", 1, "check");
            let json = serde_json::to_string(&r).unwrap();
            tx.execute(
                "INSERT INTO activity VALUES(?,'A/B',1,'check',?,1,'eligible',1)",
                params![id, now() as i64],
            )
            .unwrap();
            tx.execute("INSERT INTO seen VALUES(?,0)", [&id]).unwrap();
            tx.execute("INSERT INTO dedup VALUES(?)", [&id]).unwrap();
            tx.execute(
                "INSERT INTO content VALUES(?,?,?,?)",
                params![
                    format!("activity-payload:{id}"),
                    json,
                    now() as i64,
                    now() as i64
                ],
            )
            .unwrap();
            tx.execute(
                "INSERT INTO activity_stage VALUES('a/b',1,?,?)",
                params![id, json],
            )
            .unwrap();
            tx.execute("INSERT INTO check_context VALUES('P_1',?,'C',?,?)",params![sha,format!(r#"{{"id":"C","kind":"StatusContext","name":"CI","status":"SUCCESS","commit":"{sha}","sourceCreatedAt":"2026-10-17T00:00:00Z"}}"#),i%2]).unwrap();
            tx.execute(
                "INSERT INTO check_state VALUES('P_1',?,?,1,NULL,?)",
                params![sha, now() as i64, now() as i64],
            )
            .unwrap();
            tx.execute(
                "INSERT INTO pending(key,job,manual,foreground,at) VALUES(?,'{}',1,1,0)",
                [format!("checks:P_1:{sha}")],
            )
            .unwrap();
        }
        for r in [
            record("comment".into(), "a/b", 1, "comment"),
            record("neighbor".into(), "a/b", 10, "check"),
        ] {
            let json = serde_json::to_string(&r).unwrap();
            tx.execute(
                "INSERT INTO activity VALUES(?,?,?,?,?,1,'eligible',1)",
                params![r.id, r.repo, r.number as i64, r.kind, now() as i64],
            )
            .unwrap();
            tx.execute("INSERT INTO seen VALUES(?,0)", [&r.id]).unwrap();
            tx.execute("INSERT INTO dedup VALUES(?)", [&r.id]).unwrap();
            tx.execute(
                "INSERT INTO content VALUES(?,?,?,?)",
                params![
                    format!("activity-payload:{}", r.id),
                    json,
                    now() as i64,
                    now() as i64
                ],
            )
            .unwrap();
        }
        tx.execute(
            "INSERT INTO activity_stage VALUES('a/b',1,'comment',?)",
            [serde_json::to_string(&record("comment".into(), "a/b", 1, "comment")).unwrap()],
        )
        .unwrap();
        tx.execute(
            "INSERT INTO check_state VALUES('P_10','sha',?,1,NULL,?)",
            [now() as i64, now() as i64],
        )
        .unwrap();
        tx.execute("INSERT INTO check_context VALUES('P_10','sha','neighbor',?,0)",[r#"{"id":"neighbor","kind":"StatusContext","name":"Neighbor","status":"SUCCESS","commit":"sha","sourceCreatedAt":"2026-10-17T00:00:00Z"}"#]).unwrap();
        tx.commit().unwrap();
    }
    fn preserved(s: &Store, r: u64) {
        assert_eq!(s.revision().unwrap(), r);
        assert_eq!(s.generation().unwrap(), 0);
        assert_eq!(s.identity().unwrap().unwrap().id, 42);
        assert!(s.settings().unwrap().paused);
        for (t, count) in [
            ("watch_locator", 2),
            ("watch_source", 2),
            ("activity", 259),
            ("seen", 259),
            ("dedup", 259),
            ("activity_intent", 2),
            ("intent_member", 2),
            ("alert_attempt", 1),
            ("progress", 2),
            ("api_validator", 1),
        ] {
            assert_eq!(
                n(&s.conn, &format!("SELECT COUNT(*) FROM {t}")),
                count,
                "{t}"
            );
        }
        assert_eq!(n(&s.conn, "SELECT COUNT(*) FROM seen WHERE seen!=0"), 0);
        assert_eq!(
            n(
                &s.conn,
                "SELECT COUNT(*) FROM activity_intent WHERE state IN ('attempted','unavailable')"
            ),
            2
        );
        assert_eq!(
            n(&s.conn, "SELECT epoch FROM watch_locator WHERE id='P_1'"),
            1
        );
        assert_eq!(
            n(&s.conn, "SELECT epoch FROM watch_locator WHERE id='P_10'"),
            0
        );
        assert_eq!(n(&s.conn, "SELECT boundary FROM activity_cycle"), 123);
        assert_eq!(n(&s.conn, "SELECT due FROM activity_target"), 4070908800);
        assert_eq!(n(&s.conn, "SELECT done FROM activity_target"), 1);
        assert_eq!(n(&s.conn, "SELECT next_at FROM api_validator"), 4070908800);
        assert_eq!(s.due("activity:notifications").unwrap(), 4070908800);
        assert_eq!(
            n(
                &s.conn,
                "SELECT COUNT(*) FROM content WHERE key IN ('pr:a/b:1','list:P_1','activity-payload:comment','activity-payload:neighbor')"
            ),
            4
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
                "SELECT COUNT(*) FROM resource_state WHERE complete=1"
            ),
            3
        );
    }
    fn hidden(s: &mut Store) {
        let p = s.check_page("a/b", 1, 0).unwrap();
        assert_eq!(p.error.as_deref(), Some(MESSAGE));
        assert!(p.items.is_empty() && p.next_offset.is_none());
        assert!(p.pr.is_some());
        assert!(
            s.activity_check_records("a/b", 1, true, "sha-256")
                .unwrap()
                .is_empty()
        );
        assert!(
            s.get::<serde_json::Value>("activity-payload:check-256")
                .unwrap()
                .data
                .is_none()
        );
        assert!(
            s.get::<serde_json::Value>("pr:a/b:1")
                .unwrap()
                .data
                .is_some()
        );
        assert_eq!(s.check_page("a/b", 10, 0).unwrap().items.len(), 1);
        let a = s.activity_page(0).unwrap();
        assert!(
            a.items
                .iter()
                .filter(|a| a.kind == "check" && a.number == 1)
                .all(|a| a.text.is_none() && a.url.is_empty() && !a.seen)
        );
        assert!(
            a.items
                .iter()
                .filter(|a| a.id == "neighbor" || a.id == "comment")
                .all(|a| a.text.is_some())
        );
    }
    fn complete(s: &Store) {
        assert!(!any_pending(&s.conn).unwrap());
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
                "SELECT COUNT(*) FROM content WHERE key GLOB 'activity-payload:check-*'"
            ),
            0
        );
        assert_eq!(
            n(
                &s.conn,
                "SELECT COUNT(*) FROM activity_stage WHERE json_extract(json,'$.kind')='check'"
            ),
            0
        );
        assert_eq!(
            n(
                &s.conn,
                "SELECT COUNT(*) FROM pending WHERE substr(key,1,11)='checks:P_1:'"
            ),
            0
        );
        assert_eq!(
            n(
                &s.conn,
                "SELECT COUNT(*) FROM check_state WHERE pr_id='P_1' AND complete=0 AND error='Checks unavailable'"
            ),
            257
        );
    }
    #[test]
    fn check_purge_global_bound_visibility_and_resource_isolation() {
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("db");
        let mut s = Store::open(&path).unwrap();
        seed(&mut s);
        assert_eq!(
            s.activity_check_records("a/b", 1, true, "sha-256")
                .unwrap()
                .len(),
            1
        );
        assert!(
            s.get::<serde_json::Value>("activity-payload:check-256")
                .unwrap()
                .data
                .is_some()
        );
        let r = s.revision().unwrap() + 1;
        let changes = s.conn.total_changes();
        assert!(begin(&mut s.conn, &target()).unwrap());
        assert_eq!(s.conn.total_changes() - changes, 3);
        let due = n(&s.conn, "SELECT due FROM watch_locator WHERE id='P_1'");
        assert_eq!(
            n(
                &s.conn,
                "SELECT COUNT(*) FROM check_state WHERE pr_id='P_1' AND complete=1"
            ),
            257
        );
        hidden(&mut s);
        let mut protected = Store::open_cached(&path).unwrap();
        hidden(&mut protected);
        assert!(!s.valid_check(&check(target())).unwrap());
        assert!(
            !s.stage_contexts(
                &check(WatchTarget {
                    generation: 1,
                    ..target()
                }),
                &page(),
                true
            )
            .unwrap()
        );
        assert!(
            s.accept_head(&head(&target()), &target())
                .unwrap()
                .is_none()
        );
        assert!(
            s.accept_head(
                &head(&target()),
                &WatchTarget {
                    generation: 1,
                    ..target()
                }
            )
            .unwrap()
            .is_none()
        );
        assert!(begin(&mut s.conn, &target()).unwrap());
        assert_eq!(
            n(&s.conn, "SELECT due FROM watch_locator WHERE id='P_1'"),
            due
        );
        let mut batches = 0;
        while any_pending(&s.conn).unwrap() {
            let before = s.conn.total_changes();
            let changed = batch(&mut s.conn).unwrap();
            assert!(changed > 0 && changed <= 128);
            assert_eq!(s.conn.total_changes() - before, changed as u64);
            preserved(&s, r);
            batches += 1;
        }
        assert_eq!(batches, 21);
        complete(&s);
        hidden(&mut s);
        assert_eq!(
            n(
                &s.conn,
                "SELECT COUNT(*) FROM pending WHERE key='checks:P_10:sha' OR key='heads:P_1'"
            ),
            2
        );
        assert_eq!(
            n(
                &s.conn,
                "SELECT COUNT(*) FROM activity_stage WHERE id='comment'"
            ),
            1
        );
        let before = s.conn.total_changes();
        assert!(!s.deny_checks(&target()).unwrap());
        resume(&mut s.conn).unwrap();
        assert_eq!(s.conn.total_changes(), before);
        assert!(!s.stage_contexts(&check(target()), &page(), true).unwrap());
        drop(s);
        let mut s = Store::open(&path).unwrap();
        complete(&s);
        preserved(&s, r);
        hidden(&mut s);
        // A newly authorized current-epoch head/check read may restore access.
        let current = WatchTarget {
            generation: 1,
            ..target()
        };
        let current = s.accept_head(&head(&current), &current).unwrap().unwrap();
        assert!(s.stage_contexts(&check(current), &page(), true).unwrap());
        let p = s.check_page("a/b", 1, 0).unwrap();
        assert!(p.error.is_none());
        assert_eq!(p.commits.len(), 1);
        assert!(p.commits[0].complete);
        assert!(!s.deny_checks(&target()).unwrap());
    }
    #[test]
    fn check_purge_barrier_late_batch_and_final_marker_faults_resume() {
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("db");
        let mut s = Store::open(&path).unwrap();
        seed(&mut s);
        let r = s.revision().unwrap();
        s.conn.execute_batch("CREATE TRIGGER fail_barrier BEFORE INSERT ON check_purge BEGIN SELECT RAISE(ABORT,'fixture barrier'); END;").unwrap();
        assert!(s.deny_checks(&target()).is_err());
        assert!(!any_pending(&s.conn).unwrap());
        assert_eq!(s.revision().unwrap(), r);
        assert_eq!(
            n(&s.conn, "SELECT epoch FROM watch_locator WHERE id='P_1'"),
            0
        );
        assert_eq!(
            n(
                &s.conn,
                "SELECT COUNT(*) FROM check_context WHERE pr_id='P_1'"
            ),
            257
        );
        s.conn.execute_batch("DROP TRIGGER fail_barrier; CREATE TRIGGER fail_late BEFORE DELETE ON check_context WHEN OLD.pr_id='P_1' AND OLD.commit_sha='sha-200' BEGIN SELECT RAISE(ABORT,'fixture late'); END;").unwrap();
        assert!(s.deny_checks(&target()).is_err());
        assert_eq!(
            n(
                &s.conn,
                "SELECT COUNT(*) FROM check_context WHERE pr_id='P_1'"
            ),
            89
        );
        assert!(s.deny_checks(&target()).is_err());
        preserved(&s, r + 1);
        hidden(&mut s);
        drop(s);
        assert!(Store::open(&path).is_err());
        let c = Connection::open(&path).unwrap();
        c.execute_batch("DROP TRIGGER fail_late; CREATE TRIGGER fail_payload BEFORE DELETE ON content WHEN OLD.key='activity-payload:check-100' BEGIN SELECT RAISE(ABORT,'fixture payload'); END;").unwrap();
        assert!(Store::open(&path).is_err());
        assert_eq!(
            n(&c, "SELECT COUNT(*) FROM check_context WHERE pr_id='P_1'"),
            0
        );
        assert_eq!(
            n(
                &c,
                "SELECT COUNT(*) FROM content WHERE key GLOB 'activity-payload:check-*'"
            ),
            200
        );
        c.execute_batch("DROP TRIGGER fail_payload; CREATE TRIGGER fail_finish BEFORE DELETE ON check_purge BEGIN SELECT RAISE(ABORT,'fixture finish'); END;").unwrap();
        assert!(Store::open(&path).is_err());
        assert_eq!(
            n(
                &c,
                "SELECT COUNT(*) FROM check_state WHERE pr_id='P_1' AND complete=1"
            ),
            11
        );
        let mut protected = Store::open_cached(&path).unwrap();
        hidden(&mut protected);
        preserved(&protected, r + 1);
        c.execute_batch("DROP TRIGGER fail_finish").unwrap();
        drop(c);
        let s = Store::open(&path).unwrap();
        complete(&s);
        preserved(&s, r + 1);
    }
    #[test]
    fn check_purge_markers_and_unchanged_states_have_bounded_progress() {
        let d = tempfile::tempdir().unwrap();
        let mut s = Store::open(&d.path().join("db")).unwrap();
        for id in ["P_1", "P_10"] {
            s.conn
                .execute(
                    "INSERT INTO watch_locator(id,repo,number) VALUES(?,'a/b',1)",
                    [id],
                )
                .unwrap();
            for i in 0..257 {
                s.conn
                    .execute(
                        "INSERT INTO check_state VALUES(?,?,1,0,'Checks unavailable',1)",
                        params![id, i.to_string()],
                    )
                    .unwrap();
            }
            assert!(
                begin(
                    &mut s.conn,
                    &WatchTarget {
                        id: id.into(),
                        ..target()
                    }
                )
                .unwrap()
            );
        }
        for expected in [1, 0] {
            let before = s.conn.total_changes();
            assert_eq!(batch(&mut s.conn).unwrap(), 1);
            assert_eq!(s.conn.total_changes() - before, 1);
            assert_eq!(n(&s.conn, "SELECT COUNT(*) FROM check_purge"), expected);
        }
        let before = s.conn.total_changes();
        assert!(
            !s.deny_checks(&WatchTarget {
                id: "missing".into(),
                ..target()
            })
            .unwrap()
        );
        assert_eq!(s.conn.total_changes(), before);
    }
    #[test]
    fn check_purge_blocks_otherwise_ready_recovery() {
        let d = tempfile::tempdir().unwrap();
        let mut s = Store::open(&d.path().join("db")).unwrap();
        s.activity_recovery(123).unwrap();
        s.conn.execute_batch("INSERT INTO watch_locator(id,repo,number,head) VALUES('P_1','a/b',1,'sha');INSERT INTO watch_source VALUES('P_1','selected');
            UPDATE activity_cycle SET notifications_done=1;INSERT INTO resource_state VALUES('authored',1,1,NULL),('direct',1,1,NULL),('teams',1,1,NULL);").unwrap();
        assert!(begin(&mut s.conn, &target()).unwrap());
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
    }
    #[test]
    fn crash_child_entry() {
        let Some(path) = std::env::var_os("GITJUKE_TEST_CHECK_PURGE_DB") else {
            return;
        };
        let conn = Connection::open(Path::new(&path)).unwrap();
        conn.pragma_update(None, "synchronous", "FULL").unwrap();
        Store {
            conn,
            cache_writes: true,
        }
        .deny_checks(&target())
        .unwrap();
        panic!("child missed check purge barrier");
    }
    #[test]
    fn check_purge_sigkill_barrier_and_cleanup_commits_preserve_scope_and_history() {
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
        for (phase, contexts, states, marked) in [
            ("barrier_committed", 257, 257, true),
            ("batch_before_commit", 257, 257, true),
            ("batch_after_commit", 215, 257, true),
            ("finish_before_commit", 0, 11, true),
            ("finish_after_commit", 0, 0, false),
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
                        "check_purge::tests::crash_child_entry",
                        "--nocapture",
                    ])
                    .env("GITJUKE_TEST_CHECK_PURGE_DB", &path)
                    .env("GITJUKE_TEST_CHECK_PURGE_PHASE", phase)
                    .env("GITJUKE_TEST_CHECK_PURGE_MARKER", &marker)
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
            assert_eq!(
                n(&raw, "SELECT COUNT(*) FROM check_context WHERE pr_id='P_1'"),
                contexts
            );
            assert_eq!(
                n(
                    &raw,
                    "SELECT COUNT(*) FROM check_state WHERE pr_id='P_1' AND complete=1"
                ),
                states
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
