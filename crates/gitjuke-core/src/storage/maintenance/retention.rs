//! Bounded logical reclamation. Freed SQLite pages remain available for reuse;
//! this does not shrink the database or impose a bound on durable ledgers.
use crate::{
    Result, now,
    store::{Store, bump},
};
use rusqlite::{Transaction, params};

pub(crate) const RETENTION_BATCH: usize = 128;
pub(crate) const ACTIVITY_AGE: u64 = 90 * 86400;
pub(crate) const CONTENT_AGE: u64 = 30 * 86400;
pub(crate) const ACTIVITY_CANDIDATES: &str =
    "SELECT id FROM activity WHERE occurred<? ORDER BY occurred,id LIMIT ?";
pub(crate) const EXPIRED_CONTENT: &str = "DELETE FROM content WHERE key IN(SELECT key FROM content INDEXED BY content_lru WHERE accessed<? ORDER BY accessed,key LIMIT ?)";

fn expire_activity(tx: &Transaction<'_>, time: u64, allowance: &mut usize) -> Result<usize> {
    // The existing occurrence index bounds the candidate set. Keep only one
    // batch of identities in memory and delete its dependent payloads atomically.
    let ids = {
        let mut q = tx.prepare(ACTIVITY_CANDIDATES)?;
        q.query_map(
            params![
                time.saturating_sub(ACTIVITY_AGE) as i64,
                (*allowance / 4) as i64
            ],
            |r| r.get::<_, String>(0),
        )?
        .collect::<std::result::Result<Vec<_>, _>>()?
    };
    let before = tx.total_changes();
    for id in &ids {
        tx.execute(
            "DELETE FROM content WHERE key=?",
            [format!("activity-payload:{id}")],
        )?;
        tx.execute(
            "DELETE FROM progress WHERE key=?",
            [format!("catchup:{id}")],
        )?;
        tx.execute("DELETE FROM activity WHERE id=?", [id])?;
    }
    *allowance -= (tx.total_changes() - before) as usize;
    // seen, dedup, intent membership and attempt outcomes are identity-only
    // replay barriers. Expiration must never rewrite or delete them.
    Ok(ids.len())
}

impl Store {
    pub(crate) fn maintain_retention(&mut self) -> Result<()> {
        self.maintain_retention_at(now())
    }
    pub(crate) fn maintain_retention_at(&mut self, time: u64) -> Result<()> {
        self.publication_ready()?;
        crate::control_sweep::resume(&mut self.conn)?;
        crate::store::storage_healthy(
            std::path::Path::new(self.conn.path().unwrap())
                .parent()
                .unwrap(),
        )?;
        let tx = self.conn.transaction()?;
        let before = tx.total_changes();
        // Reserve revision before selecting any victims. Each Activity unit needs
        // at most four mutations; never split its payload/progress/minimal row.
        let mut allowance = RETENTION_BATCH - 1;
        let mut visible = expire_activity(&tx, time, &mut allowance)?;
        // Expiration is an index range, separate from orphan detection. LIMIT
        // now bounds candidates examined instead of filtering a combined OR.
        visible += tx.execute(
            EXPIRED_CONTENT,
            params![
                time.saturating_sub(CONTENT_AGE) as i64,
                (allowance / 2) as i64
            ],
        )?;
        allowance = RETENTION_BATCH - 1 - (tx.total_changes() - before) as usize;
        use crate::retention_window::{Class, sweep};
        for (class, cutoff) in [
            (Class::Orphans, time),
            (Class::Checks, time.saturating_sub(CONTENT_AGE)),
            (Class::Validators, time),
            (Class::Resources, time),
            (Class::States, time.saturating_sub(CONTENT_AGE)),
        ] {
            visible += sweep(&tx, class, cutoff, &mut allowance)?;
        }
        if visible != 0 {
            bump(&tx)?;
        }
        // Enforce the combined bound even if a later writer changes its cost.
        if tx.total_changes() - before > RETENTION_BATCH as u64 {
            return Err(crate::Error::StorageUnavailable(
                "Retention mutation allowance exceeded.".into(),
            ));
        }
        #[cfg(test)]
        crate::control_sweep::point("retention_before");
        #[cfg(test)]
        crate::publication::record(&tx, before);
        tx.commit()?;
        #[cfg(test)]
        crate::control_sweep::point("retention_after");
        Ok(())
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) fn seed(s: &mut Store, count: usize, time: u64) {
        let tx = s.conn.transaction().unwrap();
        tx.execute(
            "INSERT INTO activity_intent VALUES(1,'pr','attempted',1)",
            [],
        )
        .unwrap();
        tx.execute(
            "INSERT INTO activity_intent VALUES(2,'summary','unavailable',1)",
            [],
        )
        .unwrap();
        tx.execute("INSERT INTO alert_attempt VALUES('attempt',1)", [])
            .unwrap();
        tx.execute("INSERT INTO progress VALUES('activity:baseline','1')", [])
            .unwrap();
        for n in 0..count {
            let id = format!("old-{n:04}");
            tx.execute(
                "INSERT INTO activity VALUES(?,'a/b',1,'comment',?,1,'eligible',1)",
                params![id, time.saturating_sub(ACTIVITY_AGE + 1) as i64],
            )
            .unwrap();
            tx.execute("INSERT INTO seen VALUES(?,?)", params![id, (n % 2) as i64])
                .unwrap();
            tx.execute("INSERT INTO dedup VALUES(?)", [&id]).unwrap();
            // Refreshed recently: Activity expiry must remove its payload even
            // when ordinary content-age cleanup would leave it in place.
            tx.execute(
                "INSERT INTO content VALUES(?,'{}',?,?)",
                params![format!("activity-payload:{id}"), time as i64, time as i64],
            )
            .unwrap();
            tx.execute(
                "INSERT INTO progress VALUES(?,?)",
                params![format!("catchup:{id}"), id],
            )
            .unwrap();
            tx.execute("INSERT INTO intent_member VALUES(1,?)", [&id])
                .unwrap();
            tx.execute("INSERT INTO intent_member VALUES(2,?)", [&id])
                .unwrap();
        }
        tx.commit().unwrap();
    }
    fn count(s: &Store, table: &str) -> usize {
        s.conn
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| {
                r.get::<_, i64>(0)
            })
            .unwrap() as usize
    }

    #[test]
    fn disposable_metadata_reclaims_in_batches_across_restart_without_replay() {
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("db");
        let mut s = Store::open(&path).unwrap();
        let time = now();
        seed(&mut s, 1, time);
        s.conn
            .execute(
                "INSERT INTO due VALUES('activity:notifications',?)",
                [(time + 120) as i64],
            )
            .unwrap();
        for n in 0..RETENTION_BATCH * 2 + 1 {
            let url = format!("notifications?old-window={n:04}");
            s.conn
                .execute(
                    "INSERT INTO api_validator VALUES(?,'etag','modified',0)",
                    [&url],
                )
                .unwrap();
            s.conn
                .execute(
                    "INSERT INTO resource_state VALUES(?,1,1,NULL)",
                    [format!("api:{url}")],
                )
                .unwrap();
            s.conn
                .execute(
                    "INSERT INTO check_state VALUES('orphan',?,1,1,NULL,1)",
                    [format!("sha-{n:04}")],
                )
                .unwrap();
        }
        // Only API cache health is disposable here, never source reconciliation.
        s.conn
            .execute("INSERT INTO resource_state VALUES('authored',1,1,NULL)", [])
            .unwrap();
        let revision = s.revision().unwrap();
        s.maintain_retention_at(time).unwrap();
        for table in ["api_validator", "check_state"] {
            assert_eq!(
                count(&s, table),
                if table == "api_validator" { 135 } else { 257 }
            );
        }
        assert_eq!(count(&s, "resource_state"), 258);
        assert_eq!(s.revision().unwrap(), revision + 1);
        drop(s);
        let mut s = Store::open(&path).unwrap();
        s.maintain_retention_at(time).unwrap();
        assert_eq!(count(&s, "api_validator"), 9);
        assert_eq!(count(&s, "check_state"), 257);
        for _ in 0..8 {
            s.maintain_retention_at(time).unwrap();
        }
        assert_eq!(count(&s, "api_validator"), 0);
        assert_eq!(count(&s, "check_state"), 0);
        assert_eq!(count(&s, "resource_state"), 1);
        assert_eq!(s.due("activity:notifications").unwrap(), time + 120);
        for (table, expected) in [
            ("seen", 1),
            ("dedup", 1),
            ("intent_member", 2),
            ("activity_intent", 2),
            ("alert_attempt", 1),
            ("progress", 1),
        ] {
            assert_eq!(count(&s, table), expected);
        }
        assert_eq!(s.activity_progress("activity:baseline").unwrap(), 1);
        let revision = s.revision().unwrap();
        s.maintain_retention_at(time).unwrap();
        assert_eq!(s.revision().unwrap(), revision);
    }

    #[test]
    fn metadata_keeps_deadlines_payload_owners_and_current_or_unfinished_checks() {
        let d = tempfile::tempdir().unwrap();
        let mut s = Store::open(&d.path().join("db")).unwrap();
        let time = now();
        let edge = time.saturating_sub(CONTENT_AGE) as i64;
        for (url, deadline) in [("cached", 0), ("waiting", time + 1), ("due", time)] {
            s.conn
                .execute(
                    "INSERT INTO api_validator VALUES(?,'etag',NULL,?)",
                    params![url, deadline as i64],
                )
                .unwrap();
            s.conn
                .execute(
                    "INSERT INTO resource_state VALUES(?,1,1,NULL)",
                    [format!("api:{url}")],
                )
                .unwrap();
        }
        s.conn
            .execute(
                "INSERT INTO content VALUES('api:cached','{}',?,?)",
                params![edge, edge],
            )
            .unwrap();
        for (sha, accessed) in [
            ("edge", edge),
            ("head", 1),
            ("merge", 1),
            ("queued", 1),
            ("staged", 1),
            ("published", edge),
            ("obsolete", 1),
        ] {
            s.conn
                .execute(
                    "INSERT INTO check_state VALUES('P',?,1,1,NULL,?)",
                    params![sha, accessed],
                )
                .unwrap();
        }
        s.conn.execute_batch("INSERT INTO watch_locator(id,repo,number,head,merge_sha) VALUES('P','a/b',1,'head','merge');
            INSERT INTO watch_source VALUES('P','authored');
            INSERT INTO check_context VALUES('P','staged','c','{}',1);
            INSERT INTO check_context VALUES('P','published','c','{}',0);
            INSERT INTO pending(key,job,manual,foreground,at) VALUES('checks:P:queued','{}',0,0,0);").unwrap();
        s.maintain_retention_at(time).unwrap();
        assert_eq!(
            s.activity_validator("cached").unwrap().0.as_deref(),
            Some("etag")
        );
        assert_eq!(s.activity_validator("waiting").unwrap().2, time + 1);
        assert_eq!(s.activity_validator("due").unwrap(), (None, None, 0));
        assert_eq!(count(&s, "api_validator"), 2);
        assert_eq!(count(&s, "resource_state"), 2);
        assert_eq!(count(&s, "check_state"), 6);
        // Empty current snapshots retain health; no cleanup invents completion.
        s.cache_writes = false;
        let page = s.check_page("a/b", 1, 0).unwrap();
        assert_eq!(page.commits.len(), 2);
        assert!(page.commits.iter().all(|c| !c.complete));
        s.maintain_retention_at(time + 1).unwrap();
        assert_eq!(count(&s, "api_validator"), 0);
        // Staging, queued and current head/merge survive; aged published rows
        // and their now-empty obsolete state are removed together.
        assert_eq!(count(&s, "check_state"), 4);
        assert_eq!(count(&s, "check_context"), 1);
        assert_eq!(count(&s, "watch_source"), 1);
    }

    #[test]
    fn metadata_failure_rolls_back_payload_cleanup_deadlines_and_revision() {
        for table in ["api_validator", "resource_state", "check_state"] {
            let d = tempfile::tempdir().unwrap();
            let mut s = Store::open(&d.path().join("db")).unwrap();
            let time = now();
            seed(&mut s, 1, time);
            s.conn
                .execute_batch(
                    "INSERT INTO api_validator VALUES('old','etag',NULL,0);
                INSERT INTO resource_state VALUES('api:old',1,1,NULL);
                INSERT INTO check_state VALUES('orphan','sha',1,1,NULL,1);
                INSERT INTO due VALUES('activity:notifications',123);",
                )
                .unwrap();
            s.conn.execute_batch(&format!("CREATE TRIGGER fail_metadata BEFORE DELETE ON {table} BEGIN SELECT RAISE(ABORT,'fixture metadata cleanup failure'); END;")).unwrap();
            let revision = s.revision().unwrap();
            assert!(s.maintain_retention_at(time).is_err());
            for table in [
                "activity",
                "content",
                "api_validator",
                "resource_state",
                "check_state",
            ] {
                assert_eq!(count(&s, table), 1);
            }
            assert_eq!(count(&s, "progress"), 2);
            assert_eq!(s.revision().unwrap(), revision);
            assert_eq!(s.due("activity:notifications").unwrap(), 123);
            s.conn.execute_batch("DROP TRIGGER fail_metadata").unwrap();
            s.maintain_retention_at(time).unwrap();
            for table in [
                "activity",
                "content",
                "api_validator",
                "resource_state",
                "check_state",
            ] {
                assert_eq!(count(&s, table), 0);
            }
            assert_eq!(count(&s, "seen"), 1);
            assert_eq!(count(&s, "dedup"), 1);
            assert_eq!(count(&s, "intent_member"), 2);
        }
    }

    #[test]
    fn bounded_batches_preserve_replay_barriers_and_resume_after_reopen() {
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("db");
        let mut s = Store::open(&path).unwrap();
        let time = now();
        let total = RETENTION_BATCH * 2 + 1;
        seed(&mut s, total, time);
        let before = s.revision().unwrap();
        s.maintain_retention_at(time).unwrap();
        assert_eq!(count(&s, "activity"), total - 31);
        assert_eq!(count(&s, "content"), total - 31);
        assert_eq!(count(&s, "progress"), total - 31 + 1);
        assert_eq!(s.revision().unwrap(), before + 1);
        drop(s);
        let mut s = Store::open(&path).unwrap();
        s.maintain_retention_at(time).unwrap();
        assert_eq!(count(&s, "activity"), total - 62);
        for _ in 0..7 {
            s.maintain_retention_at(time).unwrap();
        }
        assert_eq!(count(&s, "activity"), 0);
        assert_eq!(count(&s, "content"), 0);
        assert_eq!(count(&s, "progress"), 1); // baseline survives
        assert_eq!(count(&s, "seen"), total);
        assert_eq!(count(&s, "dedup"), total);
        assert_eq!(count(&s, "intent_member"), total * 2);
        assert_eq!(count(&s, "alert_attempt"), 1);
        assert_eq!(
            s.conn
                .query_row("SELECT COUNT(*) FROM seen WHERE seen=1", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            (total / 2) as i64
        );
        assert_eq!(s.conn.query_row("SELECT COUNT(*) FROM activity_intent WHERE state IN ('attempted','unavailable')", [], |r| r.get::<_, i64>(0)).unwrap(), 2);
        let settled = s.revision().unwrap();
        s.maintain_retention_at(time).unwrap();
        assert_eq!(
            s.revision().unwrap(),
            settled,
            "empty maintenance makes no revision"
        );
    }

    #[test]
    fn cleanup_fault_rolls_back_payload_membership_and_revision_together() {
        let d = tempfile::tempdir().unwrap();
        let mut s = Store::open(&d.path().join("db")).unwrap();
        let time = now();
        seed(&mut s, 2, time);
        let before = s.revision().unwrap();
        s.conn.execute_batch("CREATE TRIGGER fail_retention BEFORE DELETE ON activity BEGIN SELECT RAISE(ABORT,'fixture retention fault'); END;").unwrap();
        assert!(s.maintain_retention_at(time).is_err());
        assert_eq!(s.revision().unwrap(), before);
        for table in ["activity", "content", "seen", "dedup"] {
            assert_eq!(count(&s, table), 2);
        }
        assert_eq!(count(&s, "progress"), 3);
        assert_eq!(count(&s, "intent_member"), 4);
        s.conn.execute_batch("DROP TRIGGER fail_retention").unwrap();
        s.maintain_retention_at(time).unwrap();
        assert_eq!(count(&s, "activity"), 0);
    }

    #[test]
    fn exact_age_boundaries_keep_minimal_activity_and_recent_payloads() {
        let d = tempfile::tempdir().unwrap();
        let mut s = Store::open(&d.path().join("db")).unwrap();
        let time = now();
        s.conn
            .execute(
                "INSERT INTO activity VALUES('edge','a/b',1,'comment',?,1,'eligible',1)",
                [time.saturating_sub(ACTIVITY_AGE) as i64],
            )
            .unwrap();
        s.conn
            .execute("INSERT INTO seen VALUES('edge',0)", [])
            .unwrap();
        s.conn
            .execute("INSERT INTO dedup VALUES('edge')", [])
            .unwrap();
        s.conn
            .execute(
                "INSERT INTO content VALUES('activity-payload:edge','{}',1,?)",
                [time.saturating_sub(CONTENT_AGE) as i64],
            )
            .unwrap();
        s.maintain_retention_at(time).unwrap();
        assert_eq!(count(&s, "activity"), 1);
        assert_eq!(count(&s, "content"), 1);
        s.maintain_retention_at(time + 1).unwrap();
        assert_eq!(count(&s, "activity"), 0);
        assert_eq!(count(&s, "content"), 0);
        assert_eq!(count(&s, "seen"), 1);
        assert_eq!(count(&s, "dedup"), 1);
    }
    #[test]
    fn content_expiry_and_orphan_cleanup_share_revision_reserved_budget() {
        let d = tempfile::tempdir().unwrap();
        let mut s = Store::open(&d.path().join("db")).unwrap();
        let time = now();
        for n in 0..=RETENTION_BATCH {
            s.conn
                .execute(
                    "INSERT INTO content VALUES(?,'{}',1,?)",
                    params![
                        format!("cache-{n}"),
                        time.saturating_sub(CONTENT_AGE + 1) as i64
                    ],
                )
                .unwrap();
        }
        s.maintain_retention_at(time).unwrap();
        assert_eq!(count(&s, "content"), 66);
        s.conn
            .execute(
                "INSERT INTO content VALUES('activity-payload:orphan','{}',?,?)",
                params![time as i64, time as i64],
            )
            .unwrap();
        s.maintain_retention_at(time).unwrap();
        assert_eq!(count(&s, "content"), 4);
        s.maintain_retention_at(time).unwrap();
        assert_eq!(count(&s, "content"), 0);
    }
}
