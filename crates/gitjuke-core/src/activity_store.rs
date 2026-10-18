use crate::{
    Result,
    activity::*,
    model::Identity,
    now,
    retention::{ACTIVITY_AGE, CONTENT_AGE},
    store::{Store, bump},
};
use rusqlite::{OptionalExtension, params};

impl Store {
    pub fn activity_recovery(&mut self, time: u64) -> Result<()> {
        crate::recovery_cleanup::recover(&mut self.conn, time)
    }

    pub(crate) fn activity_offline(&mut self) -> Result<()> {
        // An interrupted recovery already owns a boundary; ordinary offline time
        // gets its boundary when recovery starts, so activity during the outage
        // joins catch-up. Keep these states distinct across further failures.
        let tx = self.conn.transaction()?;
        tx.execute("UPDATE activity_cycle SET mode=CASE WHEN mode IN ('normal','offline') THEN 'offline' ELSE 'recovery_offline' END,notifications_done=0 WHERE id=1", [])?;
        bump(&tx)?;
        tx.commit()?;
        Ok(())
    }
    pub(crate) fn activity_targets(&mut self) -> Result<()> {
        crate::activity_enrollment::enroll(&mut self.conn)?;
        Ok(())
    }
    pub(crate) fn activity_due(&self, time: u64) -> Result<Vec<(String, u64)>> {
        let mut q=self.conn.prepare("SELECT repo,number FROM activity_target t WHERE due<=? AND NOT EXISTS(SELECT 1 FROM pending p WHERE p.key='activity:pr:'||t.repo||':'||t.number) AND (done=0 OR EXISTS(SELECT 1 FROM watch_locator l JOIN watch_source s ON s.id=l.id WHERE l.repo=t.repo AND l.number=t.number AND s.source!='selected')) ORDER BY due,repo,number LIMIT 128")?;
        Ok(q.query_map([time as i64], |r| {
            Ok((r.get(0)?, r.get::<_, i64>(1)? as u64))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?)
    }
    pub(crate) fn activity_notifications(
        &mut self,
        targets: &[(String, u64)],
        complete: Option<u64>,
    ) -> Result<()> {
        self.publication_ready()?;
        if targets.len() > 50 {
            return Err(crate::Error::Safe(
                "Notification target page exceeded its source bound.".into(),
            ));
        }
        crate::store::storage_healthy(
            std::path::Path::new(self.conn.path().unwrap())
                .parent()
                .unwrap(),
        )?;
        let tx = self.conn.transaction()?;
        let before = tx.total_changes();
        for (repo, number) in targets {
            tx.execute("INSERT INTO activity_target VALUES(?,?,0,0) ON CONFLICT(repo,number) DO UPDATE SET due=0,done=0",params![repo,*number as i64])?;
        }
        if let Some(complete) = complete {
            tx.execute("INSERT INTO progress VALUES('activity:notifications',?) ON CONFLICT(key) DO UPDATE SET value=excluded.value",[complete.to_string()])?;
            tx.execute(
                "UPDATE activity_cycle SET notifications_done=1 WHERE id=1",
                [],
            )?;
        }
        bump(&tx)?;
        if tx.total_changes() - before > 128 {
            return Err(crate::Error::Safe(
                "Activity notification mutation allowance exceeded.".into(),
            ));
        }
        #[cfg(test)]
        crate::publication::record(&tx, before);
        tx.commit()?;
        Ok(())
    }
    pub(crate) fn activity_validator(
        &self,
        url: &str,
    ) -> Result<(Option<String>, Option<String>, u64)> {
        Ok(self
            .conn
            .query_row(
                "SELECT etag,modified,next_at FROM api_validator WHERE url=?",
                [url],
                |r| Ok((r.get(0)?, r.get(1)?, r.get::<_, i64>(2)? as u64)),
            )
            .optional()?
            .unwrap_or_default())
    }
    pub(crate) fn activity_progress(&self, key: &str) -> Result<u64> {
        Ok(self
            .conn
            .query_row(
                "SELECT CAST(value AS INTEGER) FROM progress WHERE key=?",
                [key],
                |r| r.get::<_, i64>(0),
            )
            .optional()?
            .unwrap_or(0) as u64)
    }
    pub(crate) fn activity_stage(
        &mut self,
        repo: &str,
        number: u64,
        records: &[ActivityRecord],
        first: bool,
    ) -> Result<()> {
        self.publication_ready()?;
        if crate::pr_purge::pending(&self.conn, repo, number)? {
            return Err(crate::Error::Safe(
                "Resource unavailable; downloaded content is being cleared.".into(),
            ));
        }
        if first {
            crate::publication::discard_stage(
                &mut self.conn,
                "activity_stage",
                "repo=json_extract(?1,'$.repo') AND number=json_extract(?1,'$.number')",
                &serde_json::json!({"repo":repo,"number":number}),
            )?;
        }
        for records in records.chunks(100) {
            crate::content_budget::publish_bounded_observed(
                &mut self.conn,
                now(),
                |tx| {
                    for item in records {
                        let duplicate: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM activity_stage WHERE repo=? AND number=? AND id=?)",
                params![repo, number as i64, item.id],
                |r| r.get(0),
            )?;
                        if duplicate {
                            return Err(crate::Error::Safe("Individual source changed during pagination; traversal will restart without publishing incomplete progress.".into()));
                        }
                        tx.execute(
                            "INSERT INTO activity_stage VALUES(?,?,?,?)",
                            params![repo, number as i64, item.id, serde_json::to_string(item)?],
                        )?;
                    }
                    Ok(())
                },
                || {
                    #[cfg(test)]
                    crate::activity_crash::point("stage_before_commit");
                },
            )?;
        }
        #[cfg(test)]
        crate::activity_crash::point("stage_after_commit");
        Ok(())
    }
    pub(crate) fn activity_publish(
        &mut self,
        repo: &str,
        number: u64,
        manual: bool,
        _identity: &Identity,
    ) -> Result<()> {
        let Some(request) = self.publication_request("activity", &(repo, number, manual))? else {
            return Ok(());
        };
        if crate::pr_purge::pending(&self.conn, repo, number)? {
            return Err(crate::Error::Safe(
                "Resource unavailable; downloaded content is being cleared.".into(),
            ));
        }
        let baseline_at = self.activity_progress("activity:baseline")?;
        let baseline = baseline_at != 0;
        let (mode, boundary): (String, u64) = self.conn.query_row(
            "SELECT mode,boundary FROM activity_cycle WHERE id=1",
            [],
            |r| Ok((r.get(0)?, r.get::<_, i64>(1)? as u64)),
        )?;
        let paused = self.settings()?.paused;
        let time = now();
        let cutoff = time.saturating_sub(ACTIVITY_AGE);
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM(SELECT 1 FROM activity_stage WHERE repo=? AND number=? LIMIT 17)",
            params![repo, number as i64],
            |r| r.get(0),
        )?;
        if count > 16 {
            let intent = next_intent(&self.conn)?;
            return self.apply_publication("activity",serde_json::json!({"request":request,"repo":repo,"number":number,"time":time,"baseline":baseline_at,"historical":!baseline,"mode":mode,"boundary":boundary,"alerts":baseline && !manual && !paused,"intent":intent}));
        }
        crate::content_budget::publish_bounded_observed(
            &mut self.conn,
            now(),
            |tx| {
                let mut normal_intent = None;
                {
                    let mut query = tx.prepare(
                        "SELECT json FROM activity_stage WHERE repo=? AND number=? ORDER BY id",
                    )?;
                    let rows =
                        query.query_map(params![repo, number as i64], |r| r.get::<_, String>(0))?;
                    for row in rows {
                        let mut record: ActivityRecord = serde_json::from_str(&row?)?;
                        // Source ID remains the dedup key when body/state/notification reason changes.
                        let fresh =
                            tx.execute("INSERT OR IGNORE INTO dedup VALUES(?)", [&record.id])? != 0;
                        #[cfg(test)]
                        crate::activity_crash::point("publish_after_dedup");
                        record.seen = !baseline || record.occurred <= baseline_at;
                        if fresh {
                            tx.execute(
                                "INSERT OR IGNORE INTO seen VALUES(?,?)",
                                params![record.id, record.seen],
                            )?;
                        }
                        // Old source pages still establish deduplication, but cannot
                        // resurrect expired entries/payloads or create delivery work.
                        if record.occurred < cutoff {
                            continue;
                        }
                        if fresh {
                            tx.execute(
                                "INSERT INTO activity VALUES(?,?,?,?,?,?,?,?)",
                                params![
                                    record.id,
                                    repo,
                                    number as i64,
                                    record.kind,
                                    record.occurred as i64,
                                    record.eligible,
                                    record.reason,
                                    now() as i64
                                ],
                            )?;
                            #[cfg(test)]
                            crate::activity_crash::point("publish_after_activity");
                            if baseline && !record.seen && record.eligible && !manual && !paused {
                                if mode == "recovery" && record.occurred <= boundary {
                                    tx.execute(
                                        "INSERT OR IGNORE INTO progress VALUES(?,?)",
                                        params![format!("catchup:{}", record.id), record.id],
                                    )?;
                                } else {
                                    let intent = match normal_intent {
                                        Some(intent) => intent,
                                        None => {
                                            let intent = new_intent(tx, "pr")?;
                                            #[cfg(test)]
                                            crate::activity_crash::point(
                                                "publish_after_intent_created",
                                            );
                                            normal_intent = Some(intent);
                                            intent
                                        }
                                    };
                                    tx.execute(
                                        "INSERT INTO intent_member VALUES(?,?)",
                                        params![intent, record.id],
                                    )?;
                                }
                            }
                        }
                        #[cfg(test)]
                        crate::activity_crash::point("publish_after_intent");
                        // Existing seen/dedup state is never changed by a refresh or suppression.
                        let key = format!("activity-payload:{}", record.id);
                        tx.execute("INSERT INTO content SELECT ?,?,?,? WHERE EXISTS(SELECT 1 FROM activity WHERE id=?) ON CONFLICT(key) DO UPDATE SET json=excluded.json,fetched=excluded.fetched,accessed=excluded.accessed",params![key,serde_json::to_string(&record)?,time as i64,time as i64,record.id])?;
                    }
                }
                #[cfg(test)]
                crate::activity_crash::point("publish_after_payloads");
                tx.execute(
                    "DELETE FROM activity_stage WHERE repo=? AND number=?",
                    params![repo, number as i64],
                )?;
                tx.execute(
                    "UPDATE activity_target SET due=?,done=1 WHERE repo=? AND number=?",
                    params![(now() + 10) as i64, repo, number as i64],
                )?;
                tx.execute(
            "INSERT INTO progress VALUES(?,?) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            params![format!("activity:pr:{repo}:{number}"), now().to_string()],
        )?;
                #[cfg(test)]
                crate::activity_crash::point("publish_after_progress");
                bump(tx)?;
                Ok(())
            },
            || {
                #[cfg(test)]
                crate::activity_crash::point("publish_before_commit");
            },
        )?;
        #[cfg(test)]
        crate::activity_crash::point("publish_after_commit");
        Ok(())
    }
    pub(crate) fn activity_finish_recovery(&mut self, manual: bool) -> Result<()> {
        if crate::publication::pending(&self.conn)? {
            let own: bool = self.conn.query_row(
                "SELECT EXISTS(SELECT 1 FROM main.publication WHERE kind='activity_summary')",
                [],
                |r| r.get(0),
            )?;
            if own {
                self.publication_ready()?;
            }
            return Ok(());
        }
        if manual
            || self.settings()?.paused
            || (crate::invalidation::pending(&self.conn)?
                || crate::clear_content::pending(&self.conn)?
                || crate::control_sweep::pending(&self.conn)?
                || crate::check_purge::any_pending(&self.conn)?
                || crate::pr_purge::any_pending(&self.conn)?)
        {
            return Ok(());
        }
        // Enrollment spans scheduling passes; unenrolled watches are unfinished work.
        if crate::activity_enrollment::missing(&self.conn)? {
            return Ok(());
        }
        let ready:bool=self.conn.query_row("SELECT EXISTS(SELECT 1 FROM activity_cycle WHERE id=1 AND mode='recovery' AND notifications_done=1) AND NOT EXISTS(SELECT 1 FROM source_restart WHERE source IN ('authored','direct','teams') OR (substr(source,1,5)='team:' AND substr(source,6) IN (SELECT id FROM teams))) AND NOT EXISTS(SELECT 1 FROM activity_target WHERE done=0) AND NOT EXISTS(SELECT 1 FROM pending WHERE key LIKE 'discover:%' OR key LIKE 'checks:%') AND NOT EXISTS(SELECT 1 FROM watch_locator l JOIN watch_source s ON s.id=l.id WHERE s.source='authored' AND (l.last_success IS NULL OR l.error IS NOT NULL OR NOT EXISTS(SELECT 1 FROM check_state c WHERE c.pr_id=l.id AND c.commit_sha=l.head AND c.complete=1 AND c.error IS NULL AND c.accessed>=?))) AND (SELECT COUNT(*) FROM resource_state WHERE key IN ('authored','direct','teams') AND complete=1)=3 AND NOT EXISTS(SELECT 1 FROM resource_state WHERE (key IN ('authored','direct','teams') OR key LIKE 'team:%') AND (complete=0 OR error IS NOT NULL))",[now().saturating_sub(crate::retention::CONTENT_AGE) as i64],|r|r.get(0))?;
        if !ready {
            return Ok(());
        }
        let time = now();
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM(SELECT 1 FROM progress WHERE key LIKE 'catchup:%' LIMIT 55)",
            [],
            |r| r.get(0),
        )?;
        if count > 54 {
            let intent = next_intent(&self.conn)?;
            return self.apply_publication(
                "activity_summary",
                serde_json::json!({"time":time,"intent":intent}),
            );
        }
        crate::store::storage_healthy(
            std::path::Path::new(self.conn.path().unwrap())
                .parent()
                .unwrap(),
        )?;
        let tx = self.conn.transaction()?;
        let before = tx.total_changes();
        // SQLite streams membership into the single summary. Do not materialize an
        // unbounded recovery history in Rust just to count or group its members.
        let has_members: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM activity a JOIN progress p ON p.key='catchup:'||a.id JOIN seen s ON s.source_id=a.id WHERE s.seen=0 AND a.eligible=1 AND a.occurred>=?)", [time.saturating_sub(ACTIVITY_AGE) as i64], |r| r.get(0))?;
        if has_members {
            let intent = new_intent(&tx, "summary")?;
            #[cfg(test)]
            crate::activity_crash::point("summary_after_intent");
            tx.execute("INSERT INTO intent_member SELECT ?,a.id FROM activity a JOIN progress p ON p.key='catchup:'||a.id JOIN seen s ON s.source_id=a.id WHERE s.seen=0 AND a.eligible=1 AND a.occurred>=?", params![intent, time.saturating_sub(ACTIVITY_AGE) as i64])?;
        }
        #[cfg(test)]
        crate::activity_crash::point("summary_after_membership");
        tx.execute("DELETE FROM progress WHERE key LIKE 'catchup:%'", [])?;
        #[cfg(test)]
        crate::activity_crash::point("summary_after_progress");
        tx.execute(
            "INSERT INTO progress VALUES('activity:baseline',?) ON CONFLICT(key) DO NOTHING",
            [time.to_string()],
        )?;
        tx.execute("UPDATE activity_cycle SET mode='normal' WHERE id=1", [])?;
        bump(&tx)?;
        #[cfg(test)]
        crate::activity_crash::point("summary_before_commit");
        if tx.total_changes() - before > 128 {
            return Err(crate::Error::Safe(
                "Recovery completion mutation allowance exceeded.".into(),
            ));
        }
        #[cfg(test)]
        crate::publication::record(&tx, before);
        tx.commit()?;
        #[cfg(test)]
        crate::activity_crash::point("summary_after_commit");
        Ok(())
    }
    pub fn activity_page(&mut self, offset: usize) -> Result<ActivityPage> {
        // Apply the content overlay at the exact key in each union branch.
        // Joining the general protected view materializes unrelated payloads.

        let mut q=self.conn.prepare("SELECT a.id,a.repo,a.number,a.kind,a.occurred,a.eligible,a.reason,s.seen,(SELECT c.json FROM (SELECT b.json,b.accessed FROM main.content b WHERE b.key='activity-payload:'||a.id AND (NOT EXISTS(SELECT 1 FROM main.publication WHERE phase!='committed') OR NOT EXISTS(SELECT 1 FROM main.publication_row r WHERE r.tbl='content' AND r.k=json_array(b.key))) UNION ALL SELECT json_extract(r.old_json,'$[1]'),json_extract(r.old_json,'$[3]') FROM main.publication_row r WHERE r.tbl='content' AND r.k=json_array('activity-payload:'||a.id) AND r.old_json IS NOT NULL AND EXISTS(SELECT 1 FROM main.publication WHERE phase!='committed')) c WHERE c.accessed>=? AND NOT EXISTS(SELECT 1 FROM content_clear) AND NOT EXISTS(SELECT 1 FROM pr_purge p WHERE p.repo=a.repo COLLATE NOCASE AND p.number=a.number) AND NOT EXISTS(SELECT 1 FROM check_purge p JOIN watch_locator l ON l.id=p.pr_id WHERE a.kind='check' AND l.repo=a.repo COLLATE NOCASE AND l.number=a.number) LIMIT 1) FROM activity a JOIN seen s ON s.source_id=a.id  WHERE a.occurred>=? ORDER BY a.occurred DESC,a.id DESC LIMIT 101 OFFSET ?")?;
        let mut rows = q
            .query_map(
                params![
                    now().saturating_sub(CONTENT_AGE) as i64,
                    now().saturating_sub(ACTIVITY_AGE) as i64,
                    offset as i64
                ],
                |r| {
                    let payload: Option<String> = r.get(8)?;
                    let payload =
                        payload.and_then(|s| serde_json::from_str::<ActivityRecord>(&s).ok());
                    Ok(ActivityRecord {
                        id: r.get(0)?,
                        repo: r.get(1)?,
                        number: r.get::<_, i64>(2)? as u64,
                        kind: r.get(3)?,
                        occurred: r.get::<_, i64>(4)? as u64,
                        eligible: r.get(5)?,
                        reason: r.get(6)?,
                        seen: r.get(7)?,
                        actor: payload.as_ref().and_then(|p| p.actor.clone()),
                        url: payload.as_ref().map(|p| p.url.clone()).unwrap_or_default(),
                        head: payload.as_ref().and_then(|p| p.head.clone()),
                        text: payload.and_then(|p| p.text),
                    })
                },
            )?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let mut bytes = 0;
        let mut length = 0;
        for row in rows.iter().take(100) {
            let size = serde_json::to_vec(row)?.len();
            if bytes + size > 900 * 1024 {
                break;
            }
            bytes += size;
            length += 1;
        }
        let more = rows.len() > length;
        rows.truncate(length);
        let recovery: Option<String> = self
            .conn
            .query_row(
                "SELECT mode FROM activity_cycle WHERE id=1 AND mode!='normal'",
                [],
                |r| r.get(0),
            )
            .optional()?;
        let error:Option<String>=self.conn.query_row("SELECT error FROM resource_state WHERE key LIKE 'activity:%' AND error IS NOT NULL ORDER BY key LIMIT 1",[],|r|r.get(0)).optional()?.flatten();
        Ok(ActivityPage {
            items: rows,
            next_offset: more.then_some(offset + length),
            revision: self.revision()?,
            baseline: self.activity_progress("activity:baseline")? != 0,
            recovery,
            pending: self.conn.query_row(
                "SELECT COUNT(*) FROM activity_target WHERE done=0",
                [],
                |r| r.get::<_, i64>(0),
            )? as u64,
            error,
            next_poll_at: self.due("activity:notifications")?,
        })
    }
    pub fn activity_seen(&mut self, ids: &[String]) -> Result<()> {
        self.publication_ready()?;
        if ids.len() > 100 {
            return Err(crate::Error::Safe(
                "Seen updates require a snapshot of at most 100 loaded items.".into(),
            ));
        }
        let tx = self.conn.transaction()?;
        for id in ids {
            tx.execute("UPDATE seen SET seen=1 WHERE source_id=? AND EXISTS(SELECT 1 FROM activity WHERE id=? AND occurred>=?)",params![id,id,now().saturating_sub(ACTIVITY_AGE) as i64])?;
        }
        bump(&tx)?;
        tx.commit()?;
        Ok(())
    }
    pub(crate) fn activity_purge(&mut self, repo: &str, number: u64) -> Result<()> {
        crate::pr_purge::purge(&mut self.conn, repo, number, None)?;
        Ok(())
    }
}
fn new_intent(tx: &rusqlite::Transaction, kind: &str) -> Result<i64> {
    // Native delivery is unavailable in this slice. Eligibility is durable; no backlog can replay on future enabling.
    tx.execute(
        "INSERT INTO activity_intent(kind,state,created) VALUES(?,'unavailable',?)",
        params![kind, now() as i64],
    )?;
    Ok(tx.last_insert_rowid())
}

impl Store {
    pub(crate) fn activity_check_records(
        &self,
        repo: &str,
        number: u64,
        authored: bool,
        source_head: &str,
    ) -> Result<Vec<ActivityRecord>> {
        if crate::publication::pending(&self.conn)? {
            return Ok(vec![]);
        }

        if crate::pr_purge::pending(&self.conn, repo, number)? {
            return Ok(vec![]);
        }
        if !authored
            || (crate::invalidation::pending(&self.conn)?
                || crate::clear_content::pending(&self.conn)?)
        {
            return Ok(vec![]);
        }
        let current:Option<(String,String)>=self.conn.query_row("SELECT l.id,l.head FROM watch_locator l WHERE l.repo=? AND l.number=? AND l.head=?",params![repo,number as i64,source_head],|r|Ok((r.get(0)?,r.get(1)?))).optional()?;
        let Some((pr, head)) = current else {
            return Ok(vec![]);
        };
        if crate::check_purge::pending(&self.conn, &pr)? {
            return Ok(vec![]);
        }
        let mut q=self.conn.prepare("SELECT c.json FROM check_context c JOIN check_state s ON s.pr_id=c.pr_id AND s.commit_sha=c.commit_sha JOIN watch_locator l ON l.id=c.pr_id WHERE c.pr_id=? AND c.staged=0 AND s.complete=1 AND s.error IS NULL AND s.accessed>=?2 AND (c.commit_sha=l.head OR c.commit_sha=l.merge_sha) ORDER BY c.id")?;
        let contexts = q
            .query_map(
                params![
                    pr,
                    now().saturating_sub(crate::retention::CONTENT_AGE) as i64
                ],
                |r| r.get::<_, String>(0),
            )?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let mut records = std::collections::BTreeMap::new();
        for context in contexts {
            let c: crate::sync::CheckContext = serde_json::from_str(&context)?;
            // Workflow attempts have authoritative run/attempt identities. External suite retry
            // qualification remains explicitly unavailable; never guess an attempt from a timestamp.
            let (key, occurred, text) = if let (Some(run), Some(attempt), Some(time)) =
                (c.run_id, c.attempt, c.suite_updated_at.as_deref())
            {
                if c.suite_status.as_deref() != Some("COMPLETED") || c.suite_conclusion.is_none() {
                    continue;
                }
                (
                    format!("workflow:{run}:{attempt}:{}", c.commit),
                    crate::activity::timestamp(time)?,
                    format!(
                        "Workflow {run} · attempt {attempt} · {}",
                        c.suite_conclusion.as_deref().unwrap_or_default()
                    ),
                )
            } else if c.kind == "StatusContext"
                && ["SUCCESS", "FAILURE", "ERROR"].contains(&c.status.as_str())
            {
                let Some(time) = c.source_created_at.as_deref() else {
                    continue;
                };
                (
                    format!("status:{}:{}:{}", c.id, c.commit, time),
                    crate::activity::timestamp(time)?,
                    format!("{} · {}", c.name, c.status),
                )
            } else {
                continue;
            };
            let id = format!("github.com:{repo}:{number}:{key}");
            records.entry(id.clone()).or_insert(ActivityRecord {
                id,
                repo: repo.into(),
                number,
                kind: "check".into(),
                occurred,
                actor: None,
                url: format!("https://github.com/{repo}/pull/{number}/checks"),
                text: Some(text),
                eligible: true,
                reason: "eligible".into(),
                seen: false,
                head: Some(head.clone()),
            });
        }
        Ok(records.into_values().collect())
    }
}

fn next_intent(c: &rusqlite::Connection) -> Result<i64> {
    Ok(c.query_row("SELECT max(COALESCE((SELECT MAX(id) FROM activity_intent),0),COALESCE((SELECT seq FROM sqlite_sequence WHERE name='activity_intent'),0))+1",[],|r|r.get(0))?)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn denial_purge_matches_literal_repo_and_pr_boundaries_and_rolls_back() {
        let (_d, mut s, _) = setup();
        let removed = [
            "api:repos/A_/B_/pulls/1",
            "api:repos/a_/b_/pulls/1?window=1",
            "api:repos/a_/b_/pulls/1/reviews?per_page=50&page=2",
            "api:repos/a_/b_/pulls/1/comments?per_page=50&page=1",
            "api:repos/a_/b_/issues/1/comments?per_page=50&page=9",
            "api:repos/a_/b_/issues/1/timeline?per_page=50&page=3",
            "activity-payload:denied",
        ];
        let retained = [
            "api:repos/a_/b_/pulls/10",
            "api:repos/a_/b_/issues/100/comments?per_page=50&page=1",
            "api:repos/ax/b_/pulls/1/reviews",
            "api:repos/a_/bx/issues/1/comments",
            "api:repos/a_/b_x/pulls/1",
            "api:repos/a_/b_/pulls/1suffix",
            "api:repos/a_/b_/issues/1suffix",
            "api:notifications?all=true",
            "activity-payload:other",
        ];
        for key in removed.iter().chain(retained.iter()) {
            s.conn
                .execute("INSERT INTO content VALUES(?,'{}',1,1)", [key])
                .unwrap();
        }
        s.conn.execute_batch("INSERT INTO activity VALUES('denied','A_/B_',1,'comment',1,1,'eligible',1),('other','a_/b_',10,'comment',1,1,'eligible',1);
            INSERT INTO seen VALUES('denied',0); INSERT INTO dedup VALUES('denied');
            INSERT INTO progress VALUES('activity:pr:a_/b_:1','17');
            INSERT INTO activity_target VALUES('a_/b_',1,7,1);
            INSERT INTO activity_stage VALUES('a_/b_',1,'partial','{}');
            INSERT INTO activity_intent VALUES(1,'pr','attempted',1);
            INSERT INTO intent_member VALUES(1,'denied');
            CREATE TRIGGER fail_purge BEFORE UPDATE ON activity_target WHEN OLD.repo='a_/b_' BEGIN SELECT RAISE(ABORT,'fixture purge fault'); END;").unwrap();
        let revision = s.revision().unwrap();
        assert!(s.activity_purge("a_/b_", 1).is_err());
        assert_eq!(s.revision().unwrap(), revision + 1);
        assert_eq!(
            s.conn
                .query_row("SELECT COUNT(*) FROM content", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            (removed.len() + retained.len()) as i64
        );
        assert_eq!(
            s.conn
                .query_row("SELECT COUNT(*) FROM activity_stage", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            1
        );
        s.conn.execute_batch("DROP TRIGGER fail_purge").unwrap();
        s.activity_purge("a_/b_", 1).unwrap();
        for key in removed {
            assert_eq!(
                s.conn
                    .query_row("SELECT COUNT(*) FROM content WHERE key=?", [key], |r| r
                        .get::<_, i64>(0))
                    .unwrap(),
                0,
                "{key}"
            );
        }
        for key in retained {
            assert_eq!(
                s.conn
                    .query_row("SELECT COUNT(*) FROM content WHERE key=?", [key], |r| r
                        .get::<_, i64>(0))
                    .unwrap(),
                1,
                "{key}"
            );
        }
        assert_eq!(
            s.conn
                .query_row("SELECT COUNT(*) FROM activity_stage", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
        assert_eq!(
            s.conn
                .query_row("SELECT seen FROM seen WHERE source_id='denied'", [], |r| {
                    r.get::<_, i64>(0)
                })
                .unwrap(),
            0
        );
        assert_eq!(s.activity_progress("activity:pr:a_/b_:1").unwrap(), 17);
        assert_eq!(
            s.conn
                .query_row("SELECT state FROM activity_intent", [], |r| r
                    .get::<_, String>(0))
                .unwrap(),
            "attempted"
        );
        assert_eq!(
            s.conn
                .query_row("SELECT COUNT(*) FROM intent_member", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            1
        );
        assert_eq!(
            s.conn
                .query_row("SELECT COUNT(*) FROM dedup", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            1
        );
    }
    fn setup() -> (tempfile::TempDir, Store, Identity) {
        let d = tempfile::tempdir().unwrap();
        let mut s = Store::open(&d.path().join("db")).unwrap();
        let id = Identity {
            id: 42,
            login: "reader".into(),
        };
        s.bind(&id).unwrap();
        s.activity_recovery(now()).unwrap();
        s.conn
            .execute("INSERT INTO activity_target VALUES('a/b',1,0,0)", [])
            .unwrap();
        (d, s, id)
    }
    fn record(id: &str, time: u64) -> ActivityRecord {
        ActivityRecord {
            id: id.into(),
            repo: "a/b".into(),
            number: 1,
            kind: "comment".into(),
            occurred: time,
            actor: Some("other".into()),
            url: "https://github.com/a/b/pull/1#issuecomment-1".into(),
            text: Some("source".into()),
            eligible: true,
            reason: "eligible".into(),
            seen: false,
            head: None,
        }
    }
    fn ingest(s: &mut Store, id: &Identity, records: &[ActivityRecord], manual: bool) {
        s.activity_stage("a/b", 1, records, true).unwrap();
        s.activity_publish("a/b", 1, manual, id).unwrap();
    }
    fn ready(s: &mut Store) {
        s.conn
            .execute("UPDATE activity_cycle SET notifications_done=1", [])
            .unwrap();
        for source in ["authored", "direct", "teams"] {
            s.conn
                .execute("INSERT INTO resource_state VALUES(?,1,1,NULL)", [source])
                .unwrap();
        }
        s.activity_finish_recovery(false).unwrap();
    }
    #[test]
    fn baseline_suppression_snapshot_seen_clear_and_replay() {
        let (_d, mut s, id) = setup();
        ingest(&mut s, &id, &[record("old", now() - 10)], false);
        ready(&mut s);
        assert!(s.activity_page(0).unwrap().baseline);
        s.conn
            .execute(
                "UPDATE progress SET value=? WHERE key='activity:baseline'",
                [(now() - 100).to_string()],
            )
            .unwrap();
        assert!(s.activity_page(0).unwrap().items[0].seen);
        let mut suppressed = record("own", now());
        suppressed.eligible = false;
        suppressed.reason = "own action".into();
        ingest(&mut s, &id, &[record("new", now()), suppressed], false);
        assert_eq!(
            s.conn
                .query_row("SELECT COUNT(*) FROM activity_intent", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            1
        );
        s.activity_seen(&["new".into()]).unwrap();
        ingest(&mut s, &id, &[record("later", now())], false);
        assert!(
            s.activity_page(0)
                .unwrap()
                .items
                .iter()
                .find(|r| r.id == "new")
                .unwrap()
                .seen
        );
        assert!(
            !s.activity_page(0)
                .unwrap()
                .items
                .iter()
                .find(|r| r.id == "own")
                .unwrap()
                .seen
        );
        s.clear().unwrap();
        assert!(
            s.activity_page(0)
                .unwrap()
                .items
                .iter()
                .all(|r| r.text.is_none())
        );
        let intents: i64 = s
            .conn
            .query_row("SELECT COUNT(*) FROM activity_intent", [], |r| r.get(0))
            .unwrap();
        ingest(
            &mut s,
            &id,
            &[record("new", now()), record("later", now())],
            false,
        );
        assert_eq!(
            intents,
            s.conn
                .query_row("SELECT COUNT(*) FROM activity_intent", [], |r| r
                    .get::<_, i64>(0))
                .unwrap()
        );
    }
    #[test]
    fn recovery_boundary_manual_duplicates_seen_exclusion_and_restart() {
        let (d, mut s, id) = setup();
        ready(&mut s);
        // No baseline until all known targets reconcile.
        assert!(!s.activity_page(0).unwrap().baseline);
        ingest(&mut s, &id, &[], false);
        s.activity_finish_recovery(false).unwrap();
        s.conn
            .execute(
                "UPDATE progress SET value=? WHERE key='activity:baseline'",
                [(now() - 100).to_string()],
            )
            .unwrap();
        s.activity_recovery(now() - 5).unwrap();
        let boundary = s
            .conn
            .query_row("SELECT boundary FROM activity_cycle", [], |r| {
                r.get::<_, i64>(0)
            })
            .unwrap();
        ingest(&mut s, &id, &[record("manual", now() - 20)], true);
        ingest(
            &mut s,
            &id,
            &[
                record("manual", now() - 20),
                record("missed", now() - 10),
                record("seen", now() - 10),
                record("future", now()),
            ],
            false,
        );
        s.activity_seen(&["seen".into()]).unwrap();
        drop(s);
        let mut s = Store::open(&d.path().join("db")).unwrap();
        s.activity_recovery(now()).unwrap();
        assert_eq!(
            boundary,
            s.conn
                .query_row("SELECT boundary FROM activity_cycle", [], |r| r
                    .get::<_, i64>(0))
                .unwrap()
        );
        ingest(&mut s, &id, &[], false);
        s.conn
            .execute("UPDATE activity_cycle SET notifications_done=1", [])
            .unwrap();
        s.activity_finish_recovery(false).unwrap();
        let count:i64=s.conn.query_row("SELECT COUNT(*) FROM intent_member m JOIN activity_intent i ON i.id=m.intent WHERE i.kind='summary' AND m.activity='missed'",[],|r|r.get(0)).unwrap();
        assert_eq!(count, 1);
        assert_eq!(s.conn.query_row("SELECT COUNT(*) FROM intent_member m JOIN activity_intent i ON i.id=m.intent WHERE i.kind='summary'",[],|r|r.get::<_,i64>(0)).unwrap(),1);
        assert!(
            !s.activity_page(0)
                .unwrap()
                .items
                .iter()
                .find(|r| r.id == "missed")
                .unwrap()
                .seen
        );
    }
    #[test]
    fn normal_outage_captures_boundary_on_recovery_not_on_first_failure() {
        let (_d, mut s, id) = setup();
        ingest(&mut s, &id, &[], false);
        ready(&mut s);
        s.activity_offline().unwrap();
        s.activity_offline().unwrap();
        assert_eq!(
            s.conn
                .query_row("SELECT mode FROM activity_cycle", [], |r| r
                    .get::<_, String>(0))
                .unwrap(),
            "offline"
        );
        let restored_at = now() + 10;
        s.activity_recovery(restored_at).unwrap();
        assert_eq!(
            s.conn
                .query_row("SELECT boundary FROM activity_cycle", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            restored_at as i64
        );
    }
    #[test]
    fn offline_and_restart_preserve_incomplete_boundary_and_terminal_intents() {
        let (d, mut s, id) = setup();
        ingest(&mut s, &id, &[], false);
        ready(&mut s);
        let boundary = now() - 10;
        s.activity_recovery(boundary).unwrap();
        // Pending delivery is future functionality; existing unavailable/attempted
        // entries must never be made pending by an interruption or a restart.
        for state in ["unavailable", "attempted"] {
            s.conn
                .execute(
                    "INSERT INTO activity_intent(kind,state,created) VALUES('pr',?,1)",
                    [state],
                )
                .unwrap();
        }
        s.activity_offline().unwrap();
        s.activity_offline().unwrap();
        drop(s);
        let mut s = Store::open(&d.path().join("db")).unwrap();
        s.activity_recovery(now()).unwrap();
        assert_eq!(
            s.conn
                .query_row("SELECT boundary FROM activity_cycle", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            boundary as i64
        );
        assert_eq!(s.conn.query_row("SELECT COUNT(*) FROM activity_intent WHERE state IN ('unavailable','attempted')", [], |r| r.get::<_, i64>(0)).unwrap(), 2);
        assert_eq!(
            s.conn
                .query_row(
                    "SELECT COUNT(*) FROM activity_intent WHERE state='pending'",
                    [],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
            0
        );
    }
    #[test]
    fn summary_filters_expired_and_seen_members_without_loading_ids() {
        let (_d, mut s, id) = setup();
        ingest(&mut s, &id, &[], false);
        ready(&mut s);
        s.conn
            .execute(
                "UPDATE progress SET value=? WHERE key='activity:baseline'",
                [(now() - 100).to_string()],
            )
            .unwrap();
        s.activity_recovery(now()).unwrap();
        let records = (0..1001)
            .map(|n| record(&format!("missed-{n}"), now() - 10))
            .collect::<Vec<_>>();
        ingest(&mut s, &id, &records, false);
        s.activity_seen(&["missed-0".into()]).unwrap();
        s.conn
            .execute("UPDATE activity SET occurred=1 WHERE id='missed-1'", [])
            .unwrap();
        s.conn
            .execute("UPDATE activity_cycle SET notifications_done=1", [])
            .unwrap();
        s.activity_finish_recovery(false).unwrap();
        assert_eq!(
            s.conn
                .query_row(
                    "SELECT COUNT(*) FROM activity_intent WHERE kind='summary'",
                    [],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
            1
        );
        assert_eq!(s.conn.query_row("SELECT COUNT(*) FROM intent_member m JOIN activity_intent i ON i.id=m.intent WHERE i.kind='summary'", [], |r| r.get::<_, i64>(0)).unwrap(), 999);
        s.activity_finish_recovery(false).unwrap();
        assert_eq!(
            s.conn
                .query_row(
                    "SELECT COUNT(*) FROM activity_intent WHERE kind='summary'",
                    [],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
            1
        );
    }
    #[test]
    fn retention_filters_payloads_without_writes_and_old_pages_cannot_resurrect_them() {
        let (_d, mut s, id) = setup();
        ingest(&mut s, &id, &[], false);
        ready(&mut s);
        let time = now();
        s.conn
            .execute(
                "UPDATE progress SET value=? WHERE key='activity:baseline'",
                [(time - 100).to_string()],
            )
            .unwrap();
        ingest(
            &mut s,
            &id,
            &[record("retained", time - 10), record("expired", time - 10)],
            false,
        );
        s.conn
            .execute(
                "UPDATE activity SET occurred=? WHERE id='expired'",
                [time.saturating_sub(ACTIVITY_AGE + 1) as i64],
            )
            .unwrap();
        s.conn
            .execute(
                "UPDATE content SET accessed=? WHERE key='activity-payload:retained'",
                [time.saturating_sub(CONTENT_AGE + 1) as i64],
            )
            .unwrap();
        // No cleanup write is needed to enforce either lifetime in a protected cache.
        s.conn.pragma_update(None, "query_only", true).unwrap();
        let page = s.activity_page(0).unwrap();
        assert_eq!(page.items.len(), 1);
        assert_eq!(page.items[0].id, "retained");
        assert!(!page.items[0].seen);
        assert!(page.items[0].text.is_none());
        assert!(page.items[0].actor.is_none());
        assert!(page.items[0].url.is_empty());
        s.conn.pragma_update(None, "query_only", false).unwrap();
        s.maintain_retention_at(time).unwrap();
        let before: i64 = s
            .conn
            .query_row("SELECT COUNT(*) FROM activity_intent", [], |r| r.get(0))
            .unwrap();
        // A changed source timestamp cannot evade the retained source-ID barrier.
        ingest(
            &mut s,
            &id,
            &[
                record("expired", time),
                record("never-seen-old", time.saturating_sub(ACTIVITY_AGE + 1)),
            ],
            false,
        );
        assert_eq!(s.activity_page(0).unwrap().items.len(), 1);
        assert_eq!(s.conn.query_row("SELECT COUNT(*) FROM content WHERE key IN ('activity-payload:expired','activity-payload:never-seen-old')", [], |r| r.get::<_, i64>(0)).unwrap(), 0);
        assert_eq!(
            s.conn
                .query_row("SELECT COUNT(*) FROM dedup", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            3
        );
        assert_eq!(
            s.conn
                .query_row("SELECT COUNT(*) FROM activity_intent", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            before
        );
        assert!(
            !s.conn
                .query_row("SELECT seen FROM seen WHERE source_id='expired'", [], |r| r
                    .get::<_, bool>(0))
                .unwrap()
        );
    }
    #[test]
    fn publication_rolls_back_all_ledgers_and_progress_on_write_failure() {
        let (_d, mut s, id) = setup();
        s.activity_stage("a/b", 1, &[record("atomic", now())], true)
            .unwrap();
        s.conn.execute_batch("CREATE TRIGGER fail_progress BEFORE INSERT ON progress BEGIN SELECT RAISE(ABORT,'fixture write fault'); END;").unwrap();
        assert!(s.activity_publish("a/b", 1, false, &id).is_err());
        for table in ["activity", "dedup", "seen", "activity_intent"] {
            assert_eq!(
                s.conn
                    .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r
                        .get::<_, i64>(0))
                    .unwrap(),
                0
            );
        }
        assert_eq!(
            s.conn
                .query_row("SELECT done FROM activity_target", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
        s.conn.execute_batch("DROP TRIGGER fail_progress").unwrap();
        s.activity_publish("a/b", 1, false, &id).unwrap();
        assert_eq!(s.activity_page(0).unwrap().items.len(), 1);
    }
}
