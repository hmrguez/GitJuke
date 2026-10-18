//! Shared admission with bounded LRU mutations and atomic candidate publication.
//! Large reclamation rolls back the candidate before separate bounded commits.
use crate::{Error, Result, retention::CONTENT_AGE, store::CONTENT_LIMIT};
#[cfg(test)]
use rusqlite::OptionalExtension;
use rusqlite::{Connection, Transaction, params};

pub(crate) fn disposable_bytes(conn: &Connection) -> Result<i64> {
    Ok(crate::admission_query::bytes(conn)?.total)
}

/// Publication and admission share a 128-mutation allowance. Independent
/// reclamation commits only after rolling back the candidate.
pub(crate) const ADMISSION_BATCH: usize = 128;

pub(crate) fn publish_bounded_observed<T>(
    conn: &mut Connection,
    time: u64,
    write: impl FnMut(&Transaction<'_>) -> Result<T>,
    before_commit: impl Fn(),
) -> Result<T> {
    publish_inner(
        conn,
        time,
        CONTENT_LIMIT,
        write,
        before_commit,
        |_| true,
        true,
    )
}

#[cfg(test)]
pub(crate) fn publish_optional<T>(
    conn: &mut Connection,
    time: u64,
    write: impl FnMut(&Transaction<'_>) -> Result<Option<T>>,
) -> Result<Option<T>> {
    publish_inner(
        conn,
        time,
        CONTENT_LIMIT,
        write,
        || {},
        Option::is_some,
        false,
    )
}

pub(crate) fn publish_bounded<T>(
    conn: &mut Connection,
    time: u64,
    write: impl FnMut(&Transaction<'_>) -> Result<T>,
) -> Result<T> {
    publish_inner(conn, time, CONTENT_LIMIT, write, || {}, |_| true, true)
}

fn admit(conn: &Connection) -> Result<()> {
    crate::store::storage_healthy(
        conn.path()
            .and_then(|p| std::path::Path::new(p).parent())
            .unwrap_or(std::path::Path::new("/")),
    )
}

#[cfg(test)]
fn publish_to_limit<T>(
    conn: &mut Connection,
    time: u64,
    limit: i64,
    write: impl FnMut(&Transaction<'_>) -> Result<T>,
) -> Result<T> {
    publish_inner(conn, time, limit, write, || {}, |_| true, false)
}

pub(super) fn publish_inner<T>(
    conn: &mut Connection,
    time: u64,
    limit: i64,
    mut write: impl FnMut(&Transaction<'_>) -> Result<T>,
    before_commit: impl Fn(),
    should_admit: impl Fn(&T) -> bool,
    bounded: bool,
) -> Result<T> {
    loop {
        admit(conn)?;
        let tx = conn.transaction()?;
        let before = tx.total_changes();
        let result = write(&tx)?;
        if !should_admit(&result) {
            tx.rollback()?;
            return Ok(result);
        }
        let used = (tx.total_changes() - before) as usize;
        if bounded && used > 128 {
            return Err(Error::Safe(
                "Publication mutation allowance exceeded.".into(),
            ));
        }
        match evict_with_allowance(&tx, time, limit, if bounded { 128 - used } else { 128 })? {
            None => {
                #[cfg(test)]
                if bounded {
                    crate::publication::record(&tx, before);
                    let phase: Option<String> = tx
                        .query_row("SELECT phase FROM publication", [], |r| r.get(0))
                        .optional()?;
                    if let Some(p) = phase {
                        crate::publication::bounded_point(if p == "planning" {
                            "plan_before"
                        } else {
                            "apply_before"
                        });
                        if p == "planning"
                            && tx.query_row(
                                "SELECT EXISTS(SELECT 1 FROM publication_row)",
                                [],
                                |r| r.get::<_, bool>(0),
                            )?
                        {
                            crate::publication::bounded_point("prepared_before");
                        }
                    }
                }
                if bounded {
                    crate::shared_storage::bounded(&tx, before)?;
                }
                before_commit();
                #[cfg(test)]
                point("publication_before_commit");
                tx.commit()?;
                #[cfg(test)]
                if bounded {
                    let phase: Option<String> = conn
                        .query_row("SELECT phase FROM publication", [], |r| r.get(0))
                        .optional()?;
                    if let Some(p) = phase {
                        crate::publication::bounded_point(if p == "planning" {
                            "plan_after"
                        } else {
                            "apply_after"
                        });
                        if p == "planning"
                            && conn.query_row(
                                "SELECT EXISTS(SELECT 1 FROM publication_row)",
                                [],
                                |r| r.get::<_, bool>(0),
                            )?
                        {
                            crate::publication::bounded_point("prepared_after");
                        }
                    }
                }
                #[cfg(test)]
                point("publication_after_commit");
                return Ok(result);
            }
            Some(excess) => {
                // Explicit rollback is essential: never commit candidate progress,
                // dedup, membership, head or intent writes just to make space.
                tx.rollback()?;
                #[cfg(test)]
                point("candidate_rolled_back");
                if reclaim_batch(conn, excess)?.0 == 0 {
                    return Err(capacity_error());
                }
            }
        }
    }
}

fn capacity_error() -> Error {
    Error::Safe(
        "Disposable staging reached the content budget; incomplete traversal was not committed."
            .into(),
    )
}

// Returns remaining pressure only after a bounded speculative admission pass.
// Production publishers reserve their own mutations from the same allowance.
#[cfg(test)]
fn evict_to_limit(tx: &Transaction<'_>, time: u64, limit: i64) -> Result<Option<i64>> {
    evict_with_allowance(tx, time, limit, 128)
}
fn evict_with_allowance(
    tx: &Transaction<'_>,
    time: u64,
    limit: i64,
    allowance: usize,
) -> Result<Option<i64>> {
    let usage = crate::admission_query::bytes(tx)?;
    if usage.protected > limit {
        return Err(capacity_error());
    }
    let (expired, expired_bytes) = prune(
        tx,
        i64::MAX,
        allowance,
        Some(time.saturating_sub(CONTENT_AGE)),
    )?;
    let excess = usage.total - expired_bytes - limit;
    if excess <= 0 {
        return Ok(None);
    }
    let (_, freed) = prune(tx, excess, allowance - expired, None)?;
    // Selection and deletion share this transaction. Each canonical victim's
    // exact byte length is subtracted once; retained backups remain charged.
    let remaining = excess - freed;
    Ok((remaining > 0).then_some(remaining))
}

fn reclaim_batch(conn: &mut Connection, needed: i64) -> Result<(usize, i64)> {
    admit(conn)?;
    let tx = conn.transaction()?;
    let before = tx.total_changes();
    // Reserve one row for the committed presentation revision. Check snapshot
    // invalidation shares this same global allowance with payload deletions.
    let (mut changed, freed) = prune(&tx, needed, ADMISSION_BATCH - 1, None)?;
    if changed != 0 {
        crate::store::bump(&tx)?;
        changed += 1;
    }
    #[cfg(test)]
    point("reclaim_before_commit");
    crate::shared_storage::bounded(&tx, before)?;
    tx.commit()?;
    #[cfg(test)]
    point("reclaim_after_commit");
    Ok((changed, freed))
}

fn prune(
    tx: &Transaction<'_>,
    needed: i64,
    allowance: usize,
    cutoff: Option<u64>,
) -> Result<(usize, i64)> {
    // No mutation occurs while the exact shared prefix is being selected.
    let victims = crate::admission_query::victims(tx, allowance, cutoff)?;
    let mut changed = 0;
    let mut freed = 0;
    for (_, kind, a, b, c, bytes) in victims {
        if changed == allowance || freed >= needed {
            break;
        }
        let before = tx.total_changes();
        if kind == 1 {
            let dirty: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM check_state WHERE pr_id=? AND commit_sha=? AND (complete!=0 OR error IS NOT 'Cached checks evicted'))",
                params![a,b], |r| r.get(0),
            )?;
            let shadow_dirty:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM publication_row WHERE tbl='check_state' AND k=json_array(?1,?2) AND old_json IS NOT NULL AND (json_extract(old_json,'$[3]')!=0 OR json_extract(old_json,'$[4]') IS NOT 'Cached checks evicted') AND EXISTS(SELECT 1 FROM publication WHERE phase!='committed'))",params![a,b],|r|r.get(0))?;
            if allowance - changed < 3 + usize::from(dirty) + usize::from(shadow_dirty) {
                break;
            }
            if dirty {
                tx.execute("UPDATE check_state SET complete=0,error='Cached checks evicted' WHERE pr_id=? AND commit_sha=?",params![a,b])?;
            }
            if shadow_dirty {
                tx.execute("UPDATE publication_row SET old_json=json_set(old_json,'$[3]',0,'$[4]','Cached checks evicted') WHERE tbl='check_state' AND k=json_array(?,?)",params![a,b])?;
            }
            tx.execute(
                "DELETE FROM check_context WHERE pr_id=? AND commit_sha=? AND id=? AND staged=0",
                params![a, b, c],
            )?;
        } else {
            if allowance - changed < if bytes == 0 { 1 } else { 2 } {
                break;
            }
            tx.execute("DELETE FROM content WHERE key=?", [a])?;
        }
        changed += (tx.total_changes() - before) as usize;
        freed += bytes;
    }
    Ok((changed, freed))
}

#[cfg(test)]
pub(crate) fn evict(tx: &Transaction<'_>, time: u64) -> Result<()> {
    match evict_to_limit(tx, time, CONTENT_LIMIT)? {
        None => Ok(()),
        Some(_) => Err(capacity_error()),
    }
}

#[cfg(test)]
fn point(phase: &str) {
    if std::env::var("GITJUKE_TEST_ADMISSION_PHASE").as_deref() == Ok(phase) {
        std::fs::write(
            std::env::var_os("GITJUKE_TEST_ADMISSION_MARKER").unwrap(),
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
    use crate::retention::RETENTION_BATCH;
    use crate::{model::*, now, store::Store, sync::*};

    fn content(s: &Store, key: &str, text: &str, accessed: i64) {
        s.conn
            .execute(
                "INSERT INTO content VALUES(?,?,?,?)",
                params![key, text, accessed, accessed],
            )
            .unwrap();
    }
    fn keys(s: &Store) -> Vec<String> {
        s.conn
            .prepare("SELECT key FROM content ORDER BY key")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<std::result::Result<_, _>>()
            .unwrap()
    }
    fn reclaim(s: &mut Store, time: u64, limit: i64) -> Result<()> {
        publish_to_limit(&mut s.conn, time, limit, |_| Ok(()))
    }
    #[test]
    fn exact_limit_utf8_and_tied_lru_prefix() {
        let d = tempfile::tempdir().unwrap();
        let mut s = Store::open(&d.path().join("db")).unwrap();
        content(&s, "empty", "", 9);
        content(&s, "a", "éé", 10); // four bytes, two characters
        content(&s, "b", "123", 10);
        content(&s, "c", "12345", 11);
        assert_eq!(disposable_bytes(&s.conn).unwrap(), 12);
        reclaim(&mut s, 100, 12).unwrap();
        assert_eq!(keys(&s), ["a", "b", "c", "empty"]);
        reclaim(&mut s, 100, 11).unwrap();
        assert_eq!(keys(&s), ["b", "c"]);
        assert_eq!(disposable_bytes(&s.conn).unwrap(), 8);
    }
    #[test]
    fn large_victim_prefix_preserves_activity_and_replay_barriers_after_reopen() {
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("db");
        let mut s = Store::open(&path).unwrap();
        let time = now();
        content(&s, "activity-payload:item", "123", time as i64 - 1);
        s.conn.execute_batch("INSERT INTO activity VALUES('item','a/b',1,'comment',1,1,'eligible',1);
            INSERT INTO seen VALUES('item',0); INSERT INTO dedup VALUES('item');
            INSERT INTO activity_intent VALUES(1,'pr','attempted',1);
            INSERT INTO intent_member VALUES(1,'item'); INSERT INTO alert_attempt VALUES('attempt',1);
            INSERT INTO progress VALUES('activity:baseline','1');").unwrap();
        for n in 0..257 {
            content(&s, &format!("cache-{n:03}"), "x", time as i64);
        }
        reclaim(&mut s, time, 57).unwrap();
        assert_eq!(keys(&s).len(), 57);
        assert_eq!(keys(&s)[0], "cache-200");
        drop(s);
        let s = Store::open(&path).unwrap();
        assert_eq!(disposable_bytes(&s.conn).unwrap(), 57);
        for table in [
            "activity",
            "seen",
            "dedup",
            "activity_intent",
            "intent_member",
            "alert_attempt",
        ] {
            assert_eq!(
                s.conn
                    .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r
                        .get::<_, i64>(0))
                    .unwrap(),
                1
            );
        }
        assert_eq!(
            s.conn
                .query_row("SELECT seen FROM seen WHERE source_id='item'", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
        assert_eq!(s.activity_progress("activity:baseline").unwrap(), 1);
    }
    #[test]
    fn each_staging_class_shares_admission_and_rollback() {
        for insert in [
            "INSERT INTO stage VALUES('stage','12345')",
            "INSERT INTO source_stage VALUES('authored','stage','12345')",
            "INSERT INTO check_context VALUES('P','sha','stage','12345',1)",
            "INSERT INTO activity_stage VALUES('a/b',1,'stage','12345')",
        ] {
            let d = tempfile::tempdir().unwrap();
            let path = d.path().join("db");
            let mut s = Store::open(&path).unwrap();
            content(&s, "old", "123", 10);
            let revision = s.revision().unwrap();
            {
                let tx = s.conn.transaction().unwrap();
                tx.execute_batch(insert).unwrap();
                assert!(evict_to_limit(&tx, 100, 5).unwrap().is_none());
                crate::store::bump(&tx).unwrap();
                tx.commit().unwrap();
            }
            assert!(keys(&s).is_empty());
            assert_eq!(disposable_bytes(&s.conn).unwrap(), 5);
            {
                let tx = s.conn.transaction().unwrap();
                tx.execute("INSERT INTO content VALUES('new','data',100,100)", [])
                    .unwrap();
                tx.execute("INSERT INTO progress VALUES('must-rollback','1')", [])
                    .unwrap();
                crate::store::bump(&tx).unwrap();
                assert!(evict_to_limit(&tx, 100, 4).is_err());
                // Drop without commit, as every publication caller does on error.
            }
            assert_eq!(s.revision().unwrap(), revision + 1);
            assert_eq!(s.activity_progress("must-rollback").unwrap(), 0);
            assert!(keys(&s).is_empty());
            assert_eq!(disposable_bytes(&s.conn).unwrap(), 5);
            assert_eq!(s.storage_usage(&path).unwrap().disposable_bytes, 5);
        }
    }
    #[test]
    fn publication_age_cleanup_is_batched_and_maintenance_finishes_it() {
        let d = tempfile::tempdir().unwrap();
        let mut s = Store::open(&d.path().join("db")).unwrap();
        let time = now();
        let cutoff = time.saturating_sub(CONTENT_AGE) as i64;
        for n in 0..=RETENTION_BATCH {
            content(&s, &format!("expired-{n:03}"), "x", cutoff - 1);
        }
        content(&s, "exact-boundary", "x", cutoff);
        reclaim(&mut s, time, CONTENT_LIMIT).unwrap();
        assert_eq!(
            keys(&s),
            std::iter::once("exact-boundary".to_string())
                .chain((64..129).map(|i| format!("expired-{i:03}")))
                .collect::<Vec<_>>()
        );
        s.maintain_retention_at(time).unwrap();
        assert_eq!(keys(&s), ["exact-boundary", "expired-127", "expired-128"]);
        s.maintain_retention_at(time).unwrap();
        assert_eq!(keys(&s), ["exact-boundary"]);
    }
    fn checks(s: &Store, count: usize, accessed: i64) {
        s.conn
            .execute(
                "INSERT INTO check_state VALUES('P','old',?,1,NULL,?)",
                params![accessed, accessed],
            )
            .unwrap();
        for n in 0..count {
            s.conn
                .execute(
                    "INSERT INTO check_context VALUES('P','old',?,'xxx',0)",
                    [format!("ctx-{n:04}")],
                )
                .unwrap();
        }
    }
    fn check_count(s: &Store) -> i64 {
        s.conn
            .query_row(
                "SELECT COUNT(*) FROM check_context WHERE staged=0",
                [],
                |r| r.get(0),
            )
            .unwrap()
    }
    #[test]
    fn shared_lru_reclaims_checks_without_touching_staging_or_watch_history() {
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("db");
        let mut s = Store::open(&path).unwrap();
        checks(&s, 2, 10);
        content(&s, "older", "xx", 9);
        content(&s, "newer", "xxxx", 11);
        s.conn.execute_batch("INSERT INTO check_context VALUES('P','old','incomplete','x',1);
            INSERT INTO watch_locator(id,repo,number) VALUES('P','a/b',1);
            INSERT INTO watch_source VALUES('P','authored');
            INSERT INTO dedup VALUES('terminal'); INSERT INTO seen VALUES('terminal',0);
            INSERT INTO activity_intent VALUES(1,'pr','unavailable',1);
            INSERT INTO intent_member VALUES(1,'terminal'); INSERT INTO alert_attempt VALUES('attempt',1);").unwrap();
        reclaim(&mut s, 100, 13).unwrap(); // inclusive, no victims
        assert_eq!(check_count(&s), 2);
        reclaim(&mut s, 100, 8).unwrap(); // older content + first check by key
        assert_eq!(keys(&s), ["newer"]);
        assert_eq!(check_count(&s), 1);
        assert_eq!(disposable_bytes(&s.conn).unwrap(), 8);
        assert!(
            !s.conn
                .query_row("SELECT complete FROM check_state", [], |r| r
                    .get::<_, bool>(0))
                .unwrap()
        );
        drop(s);
        let s = Store::open(&path).unwrap(); // startup discards staging only
        assert_eq!(check_count(&s), 1);
        assert_eq!(s.watch_page(0).unwrap().items[0].reasons, ["authored"]);
        for table in [
            "dedup",
            "seen",
            "activity_intent",
            "intent_member",
            "alert_attempt",
        ] {
            assert_eq!(
                s.conn
                    .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r
                        .get::<_, i64>(0))
                    .unwrap(),
                1
            );
        }
    }
    #[test]
    fn lru_check_delete_failure_rolls_back_snapshot_and_new_publication() {
        let d = tempfile::tempdir().unwrap();
        let mut s = Store::open(&d.path().join("db")).unwrap();
        checks(&s, 2, 10);
        content(&s, "old", "x", 9);
        s.conn.execute_batch("CREATE TRIGGER fail_check_lru BEFORE DELETE ON check_context BEGIN SELECT RAISE(ABORT,'fixture'); END;").unwrap();
        let revision = s.revision().unwrap();
        {
            let tx = s.conn.transaction().unwrap();
            tx.execute("INSERT INTO content VALUES('new','xxx',100,100)", [])
                .unwrap();
            crate::store::bump(&tx).unwrap();
            assert!(evict_to_limit(&tx, 100, 3).is_err());
        }
        assert_eq!(s.revision().unwrap(), revision);
        assert_eq!(keys(&s), ["old"]);
        assert_eq!(check_count(&s), 2);
        assert!(
            s.conn
                .query_row("SELECT complete FROM check_state", [], |r| r
                    .get::<_, bool>(0))
                .unwrap()
        );
    }
    #[test]
    fn check_expiry_is_batched_atomic_and_continues_after_restart() {
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("db");
        let mut s = Store::open(&path).unwrap();
        let time = now();
        checks(
            &s,
            RETENTION_BATCH + 1,
            time.saturating_sub(CONTENT_AGE) as i64,
        );
        s.maintain_retention_at(time).unwrap();
        assert_eq!(check_count(&s), 129); // exact boundary retained
        let before = s.revision().unwrap();
        s.conn.execute_batch("CREATE TRIGGER fail_check_cleanup BEFORE DELETE ON check_context BEGIN SELECT RAISE(ABORT,'fixture'); END;").unwrap();
        assert!(s.maintain_retention_at(time + 1).is_err());
        assert_eq!(s.revision().unwrap(), before);
        assert!(
            s.conn
                .query_row("SELECT complete FROM check_state", [], |r| r
                    .get::<_, bool>(0))
                .unwrap()
        );
        assert_eq!(check_count(&s), 129);
        s.conn
            .execute_batch("DROP TRIGGER fail_check_cleanup")
            .unwrap();
        s.maintain_retention_at(time + 1).unwrap();
        // The inclusive-age pass advanced over 128 still-valid candidates.
        // Only the final candidate is ahead of that persisted frontier.
        assert_eq!(check_count(&s), 128);
        drop(s);
        let mut s = Store::open(&path).unwrap();
        s.maintain_retention_at(time + 1).unwrap();
        assert_eq!(check_count(&s), 87);
        s.maintain_retention_at(time + 1).unwrap();
        assert_eq!(check_count(&s), 46);
        s.maintain_retention_at(time + 1).unwrap();
        assert_eq!(check_count(&s), 5);
        s.maintain_retention_at(time + 1).unwrap();
        assert_eq!(check_count(&s), 0);
    }
    #[test]
    fn protected_check_reads_expire_without_reviving_payloads_and_access_preserves_freshness() {
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("db");
        let mut s = Store::open(&path).unwrap();
        s.register_selected(&overview(), true).unwrap();
        s.conn
            .execute("UPDATE watch_locator SET head='old'", [])
            .unwrap();
        let time = now() as i64;
        checks(&s, 1, time - 60);
        s.conn.execute("UPDATE check_context SET json=?",[r#"{"id":"ctx","kind":"CheckRun","name":"CI","status":"COMPLETED","commit":"old","suiteId":"CS_fixture","suiteStatus":"COMPLETED","suiteConclusion":"SUCCESS","suiteUpdatedAt":"2026-10-17T00:00:00Z","runId":42,"attempt":1}"#]).unwrap();
        assert_eq!(s.check_page("a/b", 1, 0).unwrap().items.len(), 1);
        assert_eq!(
            s.activity_check_records("a/b", 1, true, "old")
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            s.conn
                .query_row("SELECT fetched FROM check_state", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            time - 60
        );
        assert!(
            s.conn
                .query_row("SELECT accessed FROM check_state", [], |r| r
                    .get::<_, i64>(0))
                .unwrap()
                >= time
        );
        s.conn
            .execute("UPDATE check_state SET accessed=0", [])
            .unwrap();
        drop(s);
        let s = Store::open_cached(&path).unwrap();
        let page = s.check_page("a/b", 1, 0).unwrap();
        assert!(page.items.is_empty());
        assert!(!page.commits[0].complete);
        assert_eq!(
            page.commits[0].error.as_deref(),
            Some("Cached checks expired")
        );
        assert_eq!(check_count(&s), 1);
        assert!(
            s.activity_check_records("a/b", 1, true, "old")
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            s.conn
                .query_row("SELECT accessed FROM check_state", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
    fn overview() -> Overview {
        Overview {
            repo_id: Some(1),
            id: "P".into(),
            repo: "a/b".into(),
            number: 1,
            title: "T".into(),
            url: "https://github.com/a/b/pull/1".into(),
            body: None,
            body_limited: false,
            head: "old".into(),
            base: "main".into(),
            state: "open".into(),
            draft: false,
            author: None,
            stack: StackState::None,
        }
    }
    fn fault(s: &Store) {
        content(s, "expired", "{}", 1);
        s.conn.execute_batch("CREATE TRIGGER fail_eviction BEFORE DELETE ON content WHEN OLD.key='expired' BEGIN SELECT RAISE(ABORT,'fixture eviction fault'); END;").unwrap();
    }
    #[test]
    fn selected_registration_eviction_failure_rolls_back_membership_and_revision() {
        let d = tempfile::tempdir().unwrap();
        let mut s = Store::open(&d.path().join("db")).unwrap();
        fault(&s);
        let before = s.revision().unwrap();
        assert!(s.register_selected(&overview(), true).is_err());
        assert!(s.watch_page(0).unwrap().items.is_empty());
        assert_eq!(keys(&s), ["expired"]);
        assert_eq!(s.revision().unwrap(), before);
        s.conn.execute_batch("DROP TRIGGER fail_eviction").unwrap();
        s.register_selected(&overview(), true).unwrap();
        assert_eq!(keys(&s), ["list:P"]);
        assert_eq!(s.watch_page(0).unwrap().items[0].reasons, ["selected"]);
    }
    #[test]
    fn head_eviction_failure_rolls_back_epoch_closure_and_check_invalidation() {
        let d = tempfile::tempdir().unwrap();
        let mut s = Store::open(&d.path().join("db")).unwrap();
        s.register_selected(&overview(), true).unwrap();
        let target = s.due_watches(now(), 1, None, false).unwrap().remove(0);
        s.conn
            .execute_batch(
                "INSERT INTO check_state VALUES('P','old',1,1,NULL,1);
            INSERT INTO check_context VALUES('P','old','ctx','{}',1);",
            )
            .unwrap();
        fault(&s);
        let before = s.revision().unwrap();
        let mut pr = PullRequest {
            id: "P".into(),
            repo_id: Some(1),
            repo: "a/b".into(),
            number: 1,
            title: "updated".into(),
            url: "https://github.com/a/b/pull/1".into(),
            state: "closed".into(),
            draft: false,
            author: None,
            updated_at: date(now()),
        };
        let read = HeadRead {
            snapshot: HeadSnapshot {
                pr: pr.clone(),
                head: "new".into(),
                merge: None,
                review: None,
                generation: 0,
                fetched_at: now(),
            },
            pages: vec![],
        };
        assert!(s.accept_head(&read, &target).is_err());
        assert_eq!(s.revision().unwrap(), before);
        assert_eq!(s.watch_page(0).unwrap().items.len(), 1);
        assert_eq!(
            s.conn
                .query_row("SELECT epoch FROM watch_locator WHERE id='P'", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
        assert!(
            s.conn
                .query_row("SELECT complete FROM check_state", [], |r| r
                    .get::<_, bool>(0))
                .unwrap()
        );
        assert_eq!(
            s.conn
                .query_row("SELECT COUNT(*) FROM check_context", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            1
        );
        pr.title = "T".into();
        assert_eq!(
            s.get::<PullRequest>("list:P").unwrap().data.unwrap().title,
            pr.title
        );
        s.conn.execute_batch("DROP TRIGGER fail_eviction").unwrap();
        assert!(s.accept_head(&read, &target).unwrap().is_some());
        assert!(s.watch_page(0).unwrap().items.is_empty());
        assert!(!keys(&s).contains(&"expired".into()));
    }
}

#[cfg(test)]
#[path = "admission_tests.rs"]
pub(crate) mod admission_tests;
