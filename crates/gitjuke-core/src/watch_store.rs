//! Transactional membership reasons, bounded due work and check traversals.
use crate::{Error, Result, model::*, now, store::Store, sync::*};
use rusqlite::{OptionalExtension, params};
use serde_json::Value;

use crate::pending_selection::PendingRow;
type CheckLocator = (
    String,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
);

impl Store {
    pub fn enqueue(
        &mut self,
        key: &str,
        job: &impl serde::Serialize,
        manual: bool,
        foreground: bool,
        at: u64,
        replace: bool,
    ) -> Result<()> {
        let tx = self.conn.transaction()?;
        if replace {
            tx.execute("DELETE FROM pending WHERE key=?", [key])?;
        }
        let value = serde_json::to_value(job)?;
        let resource = if value.get("Heads").is_some()
            || value.get("Contexts").is_some()
            || value["Discover"]["source"] == "authored"
        {
            "graphql"
        } else if value.get("Query").is_some()
            || value.get("Authored").is_some()
            || value.get("Discover").is_some() && value["Discover"]["source"] != "teams"
        {
            "search"
        } else {
            "core"
        };
        tx.execute("INSERT INTO pending(key,job,manual,foreground,at,resource) VALUES(?,?,?,?,?,?) ON CONFLICT(key) DO UPDATE SET manual=MAX(manual,excluded.manual),foreground=MAX(foreground,excluded.foreground)",params![key,serde_json::to_string(job)?,manual,foreground,at as i64,resource])?;
        tx.commit()?;
        Ok(())
    }
    pub fn pending(&self, time: u64) -> Result<Vec<PendingRow>> {
        crate::pending_selection::select(self, time)
    }
    pub fn ack(&self, seq: i64) -> Result<()> {
        self.conn
            .execute("DELETE FROM pending WHERE seq=?", [seq])?;
        Ok(())
    }
    pub fn retry_task(&self, seq: i64, at: u64) -> Result<()> {
        self.conn.execute(
            "UPDATE pending SET at=? WHERE seq=?",
            params![at as i64, seq],
        )?;
        Ok(())
    }
    pub fn ensure_source(&self, source: &str) -> Result<()> {
        self.conn.execute("INSERT OR IGNORE INTO resource_state(key,complete,error) VALUES(?,0,'Discovery pending')",[source])?;
        Ok(())
    }
    pub fn begin_source(&mut self, source: &str) -> Result<()> {
        self.publication_ready()?;
        if crate::pr_purge::any_pending(&self.conn)? {
            return Err(Error::Safe(
                "Downloaded content is being cleared; refresh will retry.".into(),
            ));
        }
        crate::source_cleanup::initialize(&mut self.conn, source)
    }
    pub fn stage_source(&mut self, source: &str, prs: &[PullRequest]) -> Result<()> {
        let Some(request) = self.publication_request("source_page", &(source, prs))? else {
            return Ok(());
        };
        if crate::pr_purge::source_restart(&self.conn, source)? {
            return Err(Error::Safe(
                "Discovery initialization is unfinished; retry from the beginning.".into(),
            ));
        }
        crate::publication::discard_stage(
            &mut self.conn,
            "source_stage",
            &serde_json::json!({"source":source,"prs":prs}),
        )?;
        self.apply_publication(
            "source_page",
            serde_json::json!({"request":request,"time":now(),"source":source,"prs":prs}),
        )
    }
    pub fn stage_count(&self, source: &str) -> Result<u64> {
        crate::publication_completion::source_total(&self.conn, source)
    }
    pub fn publish_source(&mut self, source: &str) -> Result<()> {
        let Some(request) = self.publication_request("source", &source)? else {
            return Ok(());
        };
        if crate::pr_purge::any_pending(&self.conn)?
            || crate::pr_purge::source_restart(&self.conn, source)?
        {
            return Err(Error::Safe(
                "Access changed during discovery; retry the list from the beginning.".into(),
            ));
        }
        self.apply_publication(
            "source",
            serde_json::json!({"request":request,"time":now(),"source":source}),
        )
    }
    pub fn remember_selection(
        &mut self,
        selection: Option<&(String, u64)>,
        open: bool,
    ) -> Result<()> {
        let Some(request) = self.publication_request("selection", &(selection, open))? else {
            return Ok(());
        };
        let (repo, number) = selection.map(|(r, n)| (r.as_str(), *n)).unwrap_or(("", 0));
        self.apply_publication(
            "selection",
            serde_json::json!({"request":request,"time":now(),"repo":repo,"number":number,"open":open}),
        )
    }
    pub fn selection(&self) -> Result<Option<(String, u64)>> {
        let v: Option<String> = self
            .conn
            .query_row(
                "SELECT value FROM progress WHERE key='selection'",
                [],
                |r| r.get(0),
            )
            .optional()?;
        v.map(|v| serde_json::from_str(&v).map_err(Error::from))
            .transpose()
    }
    pub fn register_selected(&mut self, pr: &Overview, open: bool) -> Result<()> {
        let Some(request) = self.publication_request("selected", &(pr, open))? else {
            return Ok(());
        };
        if crate::pr_purge::pending(&self.conn, &pr.repo, pr.number)? {
            return Err(Error::Safe(
                "Resource unavailable; downloaded content is being cleared.".into(),
            ));
        }
        let p = PullRequest {
            id: pr.id.clone(),
            repo_id: pr.repo_id,
            repo: pr.repo.clone(),
            number: pr.number,
            title: pr.title.clone(),
            url: pr.url.clone(),
            state: pr.state.clone(),
            draft: pr.draft,
            author: pr.author.clone(),
            updated_at: date(now()),
        };
        let mut loc = self.publication_locator(&pr.id)?.unwrap_or_else(|| {
            serde_json::json!([
                pr.id, pr.repo, pr.number, pr.repo_id, pr.state, null, null, null, 0, 0, null,
                null, 0
            ])
        });
        loc[1] = serde_json::json!(pr.repo);
        loc[2] = serde_json::json!(pr.number);
        if pr.repo_id.is_some() {
            loc[3] = serde_json::json!(pr.repo_id);
        }
        loc[4] = serde_json::json!(pr.state);
        self.apply_publication(
            "selected",
            serde_json::json!({"request":request,"time":now(),"pr":p,"locator":loc,"open":open}),
        )
    }
    fn publication_locator(&self, id: &str) -> Result<Option<Value>> {
        let row:Option<String>=self.conn.query_row("SELECT json_array(id,repo,number,repo_id,state,review,head,merge_sha,epoch,due,last_success,error,purged_generation) FROM watch_locator WHERE id=?",[id],|r|r.get(0)).optional()?;
        row.map(|s| serde_json::from_str(&s).map_err(Error::from))
            .transpose()
    }
    pub fn active_counts(&self) -> Result<(usize, usize)> {
        Ok(self.conn.query_row("SELECT COUNT(*),COALESCE(SUM(error IS NOT NULL OR EXISTS(SELECT 1 FROM pr_purge p WHERE p.repo=watch_locator.repo COLLATE NOCASE AND p.number=watch_locator.number) OR (due<? AND (last_success IS NULL OR last_success<?))),0) FROM watch_locator WHERE id IN (SELECT id FROM watch_source)",params![now() as i64,now().saturating_sub(10) as i64],|r|Ok((r.get::<_,i64>(0)? as usize,r.get::<_,i64>(1)? as usize)))?)
    }
    pub fn due_watches(
        &self,
        time: u64,
        limit: usize,
        selected: Option<&(String, u64)>,
        manual: bool,
    ) -> Result<Vec<WatchTarget>> {
        crate::watch_scheduling::select(self, time, limit, selected, manual)
    }
    pub fn accept_head(
        &mut self,
        read: &HeadRead,
        target: &WatchTarget,
    ) -> Result<Option<WatchTarget>> {
        let request = self.publication_request(
            "head",
            &(
                &read.snapshot.pr,
                &read.snapshot.head,
                &read.snapshot.merge,
                &read.snapshot.review,
                target,
            ),
        )?;
        if request.is_none() {
            return Ok(self.publication_locator(&target.id)?.and_then(|loc| {
                loc[8].as_u64().map(|generation| WatchTarget {
                    generation,
                    ..target.clone()
                })
            }));
        }
        let request = request.unwrap();
        if read.snapshot.pr.id != target.id {
            return Err(Error::Safe(
                "Head response identity did not match its target.".into(),
            ));
        }
        let Some(mut loc) = self.publication_locator(&target.id)? else {
            return Ok(None);
        };
        if loc[1] != serde_json::json!(target.repo)
            || loc[2] != serde_json::json!(target.number)
            || loc[8].as_u64() != Some(target.generation)
            || crate::check_purge::pending(&self.conn, &target.id)?
            || crate::pr_purge::pending(&self.conn, &target.repo, target.number)?
        {
            return Ok(None);
        }
        let changed = loc[6] != serde_json::json!(read.snapshot.head)
            || loc[7] != serde_json::json!(read.snapshot.merge);
        let epoch = target.generation + u64::from(changed);
        let time = now();
        loc[3] = serde_json::json!(read.snapshot.pr.repo_id);
        loc[4] = serde_json::json!(read.snapshot.pr.state);
        loc[5] = serde_json::json!(read.snapshot.review);
        loc[6] = serde_json::json!(read.snapshot.head);
        loc[7] = serde_json::json!(read.snapshot.merge);
        loc[8] = serde_json::json!(epoch);
        loc[9] = serde_json::json!(time + 10);
        loc[10] = serde_json::json!(time);
        loc[11] = Value::Null;
        self.apply_publication(
            "head",
            serde_json::json!({"request":request,"time":time,"pr":read.snapshot.pr,"locator":loc,"changed":changed}),
        )?;
        Ok(Some(WatchTarget {
            generation: epoch,
            ..target.clone()
        }))
    }
    pub fn check_pending(&self, target: &CheckTarget) -> Result<bool> {
        Ok(self.conn.query_row("SELECT EXISTS(SELECT 1 FROM pending WHERE key=? AND json_extract(job,'$.Contexts.watch.generation')=? AND json_extract(job,'$.Contexts.head')=?)",params![format!("checks:{}:{}",target.watch.id,target.commit),target.watch.generation as i64,target.head],|r|r.get(0))?)
    }
    pub fn is_active(&self, id: &str) -> Result<bool> {
        Ok(self.conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM watch_source WHERE id=?)",
            [id],
            |r| r.get(0),
        )?)
    }
    pub fn valid_check(&self, t: &CheckTarget) -> Result<bool> {
        Ok(self.conn.query_row("SELECT COUNT(*) FROM watch_locator WHERE id=? AND epoch=? AND head=? AND (head=? OR merge_sha=?) AND NOT EXISTS(SELECT 1 FROM check_purge WHERE pr_id=watch_locator.id) AND NOT EXISTS(SELECT 1 FROM pr_purge p WHERE p.repo=watch_locator.repo COLLATE NOCASE AND p.number=watch_locator.number)",params![t.watch.id,t.watch.generation as i64,t.head,t.commit,t.commit],|r|r.get::<_,i64>(0))?>0)
    }
    pub fn stage_contexts(
        &mut self,
        target: &CheckTarget,
        page: &ContextPage,
        first: bool,
    ) -> Result<bool> {
        let Some(request) = self.publication_request(
            "checks",
            &(target, &page.contexts, &page.next, page.total, first),
        )?
        else {
            return self.valid_check(target);
        };
        if !self.valid_check(target)? {
            return Ok(false);
        }
        if page.next.is_none() {
            let ids = page
                .contexts
                .iter()
                .map(|c| c.id.clone())
                .collect::<Vec<_>>();
            let count = crate::publication_completion::staged_total(
                &self.conn,
                &target.watch.id,
                &target.commit,
                first,
                &ids,
            )?;
            if count as u64 != page.total {
                return Err(Error::Safe("Check contexts changed during pagination; previous successful results are retained.".into()));
            }
        }
        crate::publication::discard_stage(
            &mut self.conn,
            "check_context",
            &serde_json::json!({"id":target.watch.id,"commit":target.commit,"first":first,"contexts":page.contexts}),
        )?;
        self.apply_publication("checks",serde_json::json!({"request":request,"time":now(),"id":target.watch.id,"commit":target.commit,"first":first,"complete":page.next.is_none(),"total":page.total,"contexts":page.contexts}))?;
        Ok(true)
    }
    pub fn postpone_watch(&self, id: &str, at: u64, error: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE watch_locator SET due=?,error=? WHERE id=?",
            params![at as i64, error, id],
        )?;
        Ok(())
    }
    pub fn deny_checks(&mut self, target: &WatchTarget) -> Result<bool> {
        if crate::pr_purge::pending(&self.conn, &target.repo, target.number)? {
            return Ok(false);
        }
        crate::check_purge::deny(&mut self.conn, target)
    }
    pub fn purge_watch(&mut self, target: &WatchTarget) -> Result<()> {
        crate::pr_purge::purge(&mut self.conn, &target.repo, target.number, Some(target))?;
        Ok(())
    }
    pub fn watch_page(&self, offset: usize) -> Result<WatchPage> {
        crate::watch_reader::page(self, offset)
    }
    pub fn check_page(&self, repo: &str, number: u64, offset: usize) -> Result<CheckPage> {
        let row: Option<CheckLocator> = self
            .conn
            .query_row(
                "SELECT id,head,merge_sha,error,review FROM watch_locator WHERE repo=? AND number=?",
                params![repo, number as i64],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?,r.get(3)?,r.get(4)?)),
            )
            .optional()?;
        let mut result = CheckPage {
            error: None,
            review: None,
            pr: None,
            items: vec![],
            next_offset: None,
            head: None,
            commits: vec![],
            revision: self.revision()?,
        };
        if crate::clear_content::pending(&self.conn)? {
            result.error = Some(crate::clear_content::MESSAGE.into());
            return Ok(result);
        }
        if crate::pr_purge::pending(&self.conn, repo, number)? {
            result.error = Some(crate::pr_purge::MESSAGE.into());
            return Ok(result);
        }
        let Some((id, head, merge, error, review)) = row else {
            return Ok(result);
        };
        let summary: Option<String> = self
            .conn
            .query_row(
                "SELECT json FROM content WHERE key=? AND accessed>=?",
                params![
                    format!("list:{id}"),
                    crate::now().saturating_sub(crate::retention::CONTENT_AGE) as i64
                ],
                |r| r.get(0),
            )
            .optional()?;
        result.pr = summary.map(|v| serde_json::from_str(&v)).transpose()?;
        result.head = head.clone();
        result.error = error;
        result.review = review;
        if crate::check_purge::pending(&self.conn, &id)? {
            result.error = Some(crate::check_purge::MESSAGE.into());
            return Ok(result);
        }
        let cutoff = now().saturating_sub(crate::retention::CONTENT_AGE) as i64;
        let mut q=self.conn.prepare("SELECT c.json FROM check_context c JOIN check_state s USING(pr_id,commit_sha) WHERE c.pr_id=? AND (c.commit_sha=? OR c.commit_sha=?) AND c.staged=0 AND s.accessed>=? ORDER BY c.commit_sha,c.id LIMIT 101 OFFSET ?")?;
        let mut bytes = 0;
        for row in q.query_map(params![id, head, merge, cutoff, offset as i64], |r| {
            r.get::<_, String>(0)
        })? {
            let text = row?;
            if result.items.len() == 100 || bytes + text.len() > 900 * 1024 {
                result.next_offset = Some(offset + result.items.len());
                break;
            }
            bytes += text.len();
            result.items.push(serde_json::from_str(&text)?);
        }
        let mut q=self.conn.prepare("SELECT commit_sha,fetched,complete AND accessed>=?1,CASE WHEN accessed<?1 THEN 'Cached checks expired' ELSE error END FROM check_state WHERE pr_id=?2 AND (commit_sha=?3 OR commit_sha=?4)")?;
        result.commits = q
            .query_map(params![cutoff, id, head, merge], |r| {
                Ok(ResourceHealth {
                    resource: r.get(0)?,
                    fetched_at: r.get::<_, Option<i64>>(1)?.map(|n| n as u64),
                    complete: r.get(2)?,
                    error: r.get(3)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        if crate::invalidation::pending(&self.conn)? {
            for commit in &mut result.commits {
                commit.complete = false;
                commit.error = Some("Content invalidated".into());
            }
        }
        if self.cache_writes {
            // Expired snapshots cannot be revived by touching their metadata.
            self.conn.execute("UPDATE check_state SET accessed=? WHERE pr_id=? AND (commit_sha=? OR commit_sha=?) AND accessed>=?", params![now() as i64,id,head,merge,cutoff])?;
        }
        Ok(result)
    }
    pub fn health(&self) -> Result<Vec<ResourceHealth>> {
        let mut q=self.conn.prepare("SELECT key,fetched,complete AND NOT EXISTS(SELECT 1 FROM source_restart WHERE source=resource_state.key) AND NOT EXISTS(SELECT 1 FROM content_clear) AND NOT EXISTS(SELECT 1 FROM pending WHERE pending.key='discover:'||resource_state.key),CASE WHEN EXISTS(SELECT 1 FROM content_clear) THEN 'Downloaded content cleared' WHEN EXISTS(SELECT 1 FROM source_restart WHERE source=resource_state.key) THEN 'Discovery must restart from the beginning' ELSE error END FROM resource_state WHERE key IN ('authored','direct','teams') OR (key LIKE 'team:%' AND substr(key,6) IN(SELECT id FROM teams)) ORDER BY key LIMIT 100")?;
        Ok(q.query_map([], |r| {
            Ok(ResourceHealth {
                resource: r.get(0)?,
                fetched_at: r.get::<_, Option<i64>>(1)?.map(|n| n as u64),
                complete: r.get(2)?,
                error: r.get(3)?,
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?)
    }
    pub fn team_stage(&mut self, body: &Value, first: bool) -> Result<()> {
        let Some(request) = self.publication_request("teams_page", &(body, first))? else {
            return Ok(());
        };
        let rows = body.as_array().ok_or_else(schema)?;
        if rows.len() > 100 {
            return Err(schema());
        }
        for t in rows {
            crate::github::string(t, "node_id")?;
            let org = crate::github::string(&t["organization"], "login")?;
            let slug = crate::github::string(t, "slug")?;
            if !crate::github::valid_repo(&format!("{org}/{slug}")) {
                return Err(schema());
            }
        }
        self.apply_publication(
            "teams_page",
            serde_json::json!({"request":request,"time":now(),"rows":rows,"first":first}),
        )
    }
    pub fn publish_teams(&mut self) -> Result<()> {
        let Some(request) = self.publication_request("teams", &())? else {
            return Ok(());
        };
        self.apply_publication("teams", serde_json::json!({"request":request,"time":now()}))
    }
    pub fn teams_due(&self) -> Result<Vec<(String, String)>> {
        let mut q = self.conn.prepare("SELECT id,slug FROM teams ORDER BY id")?;
        Ok(q.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<std::result::Result<Vec<_>, _>>()?)
    }
    pub fn budget(&self, resource: &str) -> Result<Option<Budget>> {
        let text: Option<String> = self
            .conn
            .query_row(
                "SELECT value FROM progress WHERE key=?",
                [format!("budget:{resource}")],
                |r| r.get(0),
            )
            .optional()?;
        text.map(|v| serde_json::from_str(&v).map_err(Error::from))
            .transpose()
    }
    pub fn save_budget(&self, b: &Budget) -> Result<()> {
        self.conn.execute(
            "INSERT INTO progress VALUES(?,?) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            params![format!("budget:{}", b.resource), serde_json::to_string(b)?],
        )?;
        Ok(())
    }
}

impl Store {
    pub fn budget_eligible(&self, resource: &str, time: u64) -> Result<bool> {
        Ok(self.due(&format!("backoff:{resource}"))? <= time
            && self
                .budget(resource)?
                .is_none_or(|b| b.blocked_until <= time))
    }
    pub fn record_failure(&self, resource: &str, time: u64) -> Result<()> {
        let count = self
            .due(&format!("failures:{resource}"))?
            .saturating_add(1)
            .min(8);
        self.set_due(&format!("failures:{resource}"), count)?;
        self.set_due(
            &format!("backoff:{resource}"),
            time + 30 * 2u64.pow(count as u32 - 1).min(120),
        )?;
        Ok(())
    }
    pub fn record_response(
        &self,
        resource: &str,
        r: &crate::github::Response,
        payload: usize,
        time: u64,
    ) -> Result<()> {
        let mut budget = self.budget(resource)?.unwrap_or(Budget {
            resource: resource.into(),
            remaining: u64::MAX,
            reset_at: 0,
            blocked_until: 0,
            requests: 0,
            payload_bytes: 0,
            cost: 0,
        });
        budget.requests += 1;
        budget.payload_bytes = budget.payload_bytes.saturating_add(payload as u64);
        if let Some(remaining) = r
            .headers
            .get("x-ratelimit-remaining")
            .and_then(|v| v.parse().ok())
        {
            budget.remaining = remaining;
        }
        if let Some(reset) = r
            .headers
            .get("x-ratelimit-reset")
            .and_then(|v| v.parse().ok())
        {
            budget.reset_at = reset;
        }
        if let Some(rate) = r.body.get("data").and_then(|d| d.get("rateLimit")) {
            budget.remaining = num(rate, "remaining")?;
            budget.cost += num(rate, "cost")?;
            // Keep headers' exact reset epoch where present; GraphQL resetAt is checked too.
            if let Some(reset) = rate["resetAt"].as_str() {
                budget.reset_at = parse_timestamp(reset)?;
            }
        }
        let deadline = r.next_deadline(time);
        // Reserve capacity for ordinary gh use instead of deliberately spending to zero.
        if budget.remaining <= 10 && budget.reset_at > time {
            budget.blocked_until = budget.reset_at;
        }
        if deadline > time {
            budget.blocked_until = budget.blocked_until.max(deadline);
        }
        if r.status == 429
            || r.status == 403 && deadline > time
            || r.body["errors"]
                .as_array()
                .is_some_and(|a| a.iter().any(|e| e["type"] == "RATE_LIMITED"))
        {
            self.record_failure(resource, time)?;
            budget.blocked_until = budget
                .blocked_until
                .max(self.due(&format!("backoff:{resource}"))?);
            // Secondary-limit deadlines apply across API resources and identity probes.
            if r.status == 429
                || r.headers.contains_key("retry-after")
                || r.body["message"]
                    .as_str()
                    .is_some_and(|s| s.contains("secondary"))
            {
                self.set_due("remote", budget.blocked_until)?;
            }
        } else if (200..300).contains(&r.status) && r.body.get("errors").is_none() {
            self.set_due(&format!("failures:{resource}"), 0)?;
            self.set_due(&format!("backoff:{resource}"), 0)?;
        }
        self.save_budget(&budget)
    }
    pub fn list_page(&mut self, view: &str, offset: usize) -> Result<Page> {
        let predicate = match view {
            "authored" => "source='authored'",
            "requested" => "(source='direct' OR source LIKE 'team:%')",
            _ => return Err(Error::Safe("Unknown PR list.".into())),
        };
        if crate::clear_content::pending(&self.conn)? {
            return Ok(Page {
                items: vec![],
                next_offset: None,
                fetched_at: None,
                complete: false,
                error: Some(crate::clear_content::MESSAGE.into()),
                revision: self.revision()?,
            });
        }
        let sql = format!(
            "SELECT c.key,c.json FROM content c JOIN watch_source s ON c.key='list:'||s.id WHERE {predicate} AND c.accessed>=? AND NOT EXISTS(SELECT 1 FROM watch_locator l JOIN pr_purge p ON p.repo=l.repo COLLATE NOCASE AND p.number=l.number WHERE c.key='list:'||l.id) GROUP BY c.key ORDER BY c.key LIMIT 101 OFFSET ?"
        );
        let mut q = self.conn.prepare(&sql)?;
        let mut items = vec![];
        let mut bytes = 0;
        let mut more = false;
        for row in q.query_map(
            params![now().saturating_sub(30 * 86400) as i64, offset as i64],
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
        )? {
            let (key, text) = row?;
            if text.len() > 900 * 1024 {
                return Err(Error::Safe("List record exceeds IPC byte budget.".into()));
            }
            if items.len() == 100 || bytes + text.len() > 900 * 1024 {
                more = true;
                break;
            }
            bytes += text.len();
            items.push(serde_json::from_str(&text)?);
            if self.cache_writes {
                self.conn.execute(
                    "UPDATE content SET accessed=? WHERE key=?",
                    params![now() as i64, key],
                )?;
            }
        }
        let keys = if view == "authored" {
            "key='authored'"
        } else {
            "(key='direct' OR key='teams' OR key LIKE 'team:%')"
        };
        let (fetched,complete,error):(Option<i64>,bool,Option<String>)=self.conn.query_row(&format!("SELECT MIN(fetched),COALESCE(MIN(complete),0),MAX(error) FROM resource_state WHERE {keys}"),[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?)))?;
        let expected: i64 = self.conn.query_row(
            &format!("SELECT COUNT(DISTINCT id) FROM watch_source WHERE {predicate}"),
            [],
            |r| r.get(0),
        )?;
        let available:i64=self.conn.query_row(&format!("SELECT COUNT(DISTINCT id) FROM watch_source WHERE {predicate} AND 'list:'||id IN(SELECT key FROM content WHERE accessed>=? AND NOT EXISTS(SELECT 1 FROM watch_locator l JOIN pr_purge p ON p.repo=l.repo COLLATE NOCASE AND p.number=l.number WHERE content.key='list:'||l.id))"),[now().saturating_sub(30 * 86400) as i64],|r|r.get(0))?;
        let sources_ready = if view == "authored" {
            true
        } else {
            self.conn.query_row("SELECT EXISTS(SELECT 1 FROM resource_state WHERE key='direct') AND EXISTS(SELECT 1 FROM resource_state WHERE key='teams') AND NOT EXISTS(SELECT 1 FROM teams WHERE 'team:'||id NOT IN (SELECT key FROM resource_state WHERE complete=1))",[],|r|r.get::<_,bool>(0))?
        };
        Ok(Page {
            next_offset: more.then_some(offset + items.len()),
            items,
            fetched_at: fetched.map(|v| v as u64),
            complete: complete && expected == available && sources_ready  && !self.conn.query_row("SELECT EXISTS(SELECT 1 FROM source_restart WHERE source=? OR (?=1 AND (source='teams' OR source LIKE 'team:%')))",params![if view=="authored" {"authored"} else {"direct"},view!="authored"],|r|r.get::<_,bool>(0))?,
            error,
            revision: self.revision()?,
        })
    }
}
pub fn parse_timestamp(v: &str) -> Result<u64> {
    let value = std::ffi::CString::new(v).map_err(|_| schema())?;
    let format = c"%Y-%m-%dT%H:%M:%SZ";
    let mut t: libc::tm = unsafe { std::mem::zeroed() };
    let end = unsafe { libc::strptime(value.as_ptr(), format.as_ptr(), &mut t) };
    if end.is_null() || unsafe { *end } != 0 {
        return Err(schema());
    }
    let timestamp = unsafe { libc::timegm(&mut t) };
    if timestamp < 0 {
        return Err(schema());
    }
    Ok(timestamp as u64)
}
