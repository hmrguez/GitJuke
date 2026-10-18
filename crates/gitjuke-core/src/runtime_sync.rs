use crate::{
    Error, Result, github,
    model::*,
    now,
    runtime::{Inner, Job, Runtime, Work},
    store::Store,
    sync::*,
    transport::{ReadOp, Transport},
};
use tokio_util::sync::CancellationToken;

impl Runtime {
    pub(crate) fn job_resource(job: &Job) -> &'static str {
        match job {
            Job::Heads(_) | Job::Contexts(_) | Job::Conversation(_) => "graphql",
            Job::Discover(p) if p.source == "authored" => "graphql",
            Job::Discover(p) if p.source != "teams" => "search",
            Job::Authored(_) | Job::Query(_) => "search",
            _ => "core",
        }
    }
    pub(crate) fn job_eligible(store: &Store, job: &Job, time: u64) -> Result<bool> {
        if let Job::Activity(p) = job
            && store.activity_validator(&p.url())?.2 > time
        {
            return Ok(false);
        }
        Ok(store.budget_eligible("core", time)?
            && store.budget_eligible(Self::job_resource(job), time)?)
    }
    pub fn requested_page(&self, offset: usize) -> Result<Page> {
        let mut i = self.lock()?;
        if i.locked {
            return Err(Error::Safe(i.health.clone()));
        }
        Self::cached_operation(&mut i, |s| s.list_page("requested", offset))
    }
    pub(crate) fn request_discovery(&self, manual: bool) -> Result<()> {
        let result = self.request_discovery_inner(manual);
        self.protect_storage(result)
    }
    fn request_discovery_inner(&self, manual: bool) -> Result<()> {
        let mut i = self.lock()?;
        Self::require_remote_storage(&i)?;
        let id = i
            .store
            .identity()?
            .ok_or_else(|| Error::Safe("Identity validation is required.".into()))?;
        for p in [
            Discovery::new("authored", ""),
            Discovery::new("direct", &format!("user-review-requested:{}", id.login)),
            Discovery::new("teams", ""),
        ] {
            if !manual && i.store.due(&format!("limited:{}", p.source))? != 0 {
                continue;
            }
            if manual {
                i.store.set_due(&format!("limited:{}", p.source), 0)?;
            }
            i.store.ensure_source(&p.source)?;
            i.store.enqueue(
                &format!("discover:{}", p.source),
                &Job::Discover(p),
                manual,
                false,
                0,
                false,
            )?;
        }
        i.store.set_due("discovery", now() + 60)?;
        self.schedule_activity(&mut i, manual)?;
        if manual {
            i.store.set_due("manual-boundary", now())?;
            crate::control_sweep::run(&mut i.store.conn, "manual", serde_json::json!({}))?;
            self.schedule_heads_locked(&mut i, true)?;
        }
        if manual
            && i.reader_open
            && let Some((id, version)) = i.active_query
        {
            Self::queue_query(&mut i, id, version, 1, true)?;
        }
        let selected = i.selected.clone();
        let open = i.reader_open;
        i.store.remember_selection(selected.as_ref(), open)?;
        drop(i);
        self.notify_sync();
        Ok(())
    }
    pub(crate) fn schedule_heads_locked(&self, i: &mut Inner, manual: bool) -> Result<()> {
        // Reserve disjoint due batches before hydration expands individual source reads.
        // Only 128 runnable descriptors are materialized; extra watch work remains due.
        for _ in 0..128 {
            let targets = i.store.due_watches(
                now(),
                i.batch_size,
                if i.reader_visible {
                    i.selected.as_ref()
                } else {
                    None
                },
                manual,
            )?;
            if targets.is_empty() {
                break;
            }
            let foreground = i.reader_visible
                && targets
                    .iter()
                    .any(|t| i.selected.as_ref() == Some(&(t.repo.clone(), t.number)));
            let job = Job::Heads(targets);
            i.store
                .enqueue(&job.key(), &job, manual, foreground, 0, false)?;
        }
        Ok(())
    }
    async fn sync_response(
        &self,
        t: &Transport,
        op: ReadOp,
        c: &CancellationToken,
        epoch: u64,
        id: &Identity,
        graph: bool,
    ) -> Result<github::Response> {
        let r = self.response(t, op, c).await?;
        r.require_ok()?;
        if graph
            && r.body["data"].is_null()
            && r.body["errors"]
                .as_array()
                .is_some_and(|es| es.iter().any(|e| e["type"] == "FORBIDDEN"))
        {
            return Err(Error::Denied);
        }
        if graph {
            if let Some(viewer) = r
                .body
                .get("data")
                .and_then(|d| d.get("viewer"))
                .filter(|v| !v.is_null())
            {
                let actual = Identity {
                    id: num(viewer, "databaseId")?,
                    login: github::string(viewer, "login")?,
                };
                let mut i = self.lock()?;
                if epoch != i.store.generation()? || c.is_cancelled() {
                    return Err(Error::Safe(
                        "Obsolete GraphQL identity result discarded.".into(),
                    ));
                }
                if actual.id != id.id {
                    i.locked = true;
                    i.store.invalidate()?;
                    return Err(Error::Safe(
                        "The authenticated GraphQL identity changed; private content is locked."
                            .into(),
                    ));
                }
                i.store.bind(&actual)?;
            } else {
                return Err(Error::Safe("The GraphQL response did not verify its authenticated viewer; no content was committed.".into()));
            }
        } else {
            let after = self.identity(t, c, epoch).await?;
            if after.id != id.id {
                return Err(Error::Safe("Identity changed during discovery.".into()));
            }
        }
        r.require_ok()?;
        Ok(r)
    }
    fn apply_check_denial(
        &self,
        target: &CheckTarget,
        generation: u64,
        token: &CancellationToken,
    ) -> Result<()> {
        let mut i = self.lock()?;
        if generation != i.store.generation()? || token.is_cancelled() || i.locked {
            return Ok(());
        }
        Self::require_remote_storage(&i)?;
        i.store.deny_checks(&target.watch)?;
        Ok(())
    }
    pub(crate) async fn execute_sync(
        &self,
        w: &Work,
        t: &Transport,
        c: &CancellationToken,
        epoch: u64,
    ) -> Result<()> {
        let identity = self.identity(t, c, epoch).await?;
        match &w.job {
            Job::Discover(plan) => {
                if plan.source != "teams" {
                    let mut i = self.lock()?;
                    if epoch != i.store.generation()? || c.is_cancelled() || i.locked {
                        return Ok(());
                    }
                    if crate::pr_purge::source_restart(&i.store.conn, &plan.source)? {
                        let fresh = Discovery::new(&plan.source, &plan.qualifier);
                        // A fresh first page clears the guard in begin_source.
                        let first = plan.received == 0
                            && plan.cursor.is_none()
                            && plan.ranges.len() == 1
                            && plan.ranges[0].page == 1
                            && plan.ranges[0].received == 0;
                        if !first {
                            i.store.enqueue(
                                &format!("discover:{}", plan.source),
                                &Job::Discover(fresh),
                                w.manual,
                                false,
                                0,
                                true,
                            )?;
                            return Ok(());
                        }
                    }
                }
                if plan.source == "direct"
                    && plan.qualifier != format!("user-review-requested:{}", identity.login)
                {
                    let mut i = self.lock()?;
                    let next = Discovery::new(
                        "direct",
                        &format!("user-review-requested:{}", identity.login),
                    );
                    i.store.enqueue(
                        "discover:direct",
                        &Job::Discover(next),
                        w.manual,
                        false,
                        0,
                        true,
                    )?;
                    return Ok(());
                }
                if let Some(team_id) = plan.source.strip_prefix("team:") {
                    let mut i = self.lock()?;
                    let slug = i
                        .store
                        .teams_due()?
                        .into_iter()
                        .find(|(id, _)| id == team_id)
                        .map(|(_, slug)| slug);
                    let Some(slug) = slug else { return Ok(()) };
                    let qualifier = format!("team-review-requested:{slug}");
                    if plan.qualifier != qualifier {
                        let next = Discovery::new(&plan.source, &qualifier);
                        i.store.enqueue(
                            &format!("discover:{}", plan.source),
                            &Job::Discover(next),
                            w.manual,
                            false,
                            0,
                            true,
                        )?;
                        return Ok(());
                    }
                }
                let op = if plan.source == "authored" {
                    ReadOp::Graph(GraphRead::Authored {
                        cursor: plan.cursor.clone(),
                        size: plan.page_size,
                    })
                } else if plan.source == "teams" {
                    ReadOp::Teams { page: plan.page }
                } else {
                    ReadOp::Search {
                        query: plan.query(),
                        page: plan.ranges.last().unwrap().page,
                        size: plan.page_size,
                    }
                };
                let r = self
                    .sync_response(t, op, c, epoch, &identity, plan.source == "authored")
                    .await?;
                let mut i = self.lock()?;
                if epoch != i.store.generation()? || c.is_cancelled() || i.locked {
                    return Ok(());
                }
                let mut p = plan.clone();
                let complete = if p.source == "teams" {
                    i.store.team_stage(&r.body, p.page == 1)?;
                    let count = r.body.as_array().ok_or_else(schema)?.len() as u64;
                    p.received += count;
                    let next = r
                        .headers
                        .get("link")
                        .is_some_and(|v| v.contains("rel=\"next\""));
                    if next {
                        if count == 0 {
                            return Err(schema());
                        }
                        p.page += 1;
                        false
                    } else {
                        let actual: i64 = i.store.conn.query_row(
                            "SELECT COUNT(*) FROM teams_stage",
                            [],
                            |r| r.get(0),
                        )?;
                        if actual as u64 != p.received {
                            return Err(schema());
                        }
                        i.store.publish_teams()?;
                        for (id, slug) in i.store.teams_due()? {
                            let source = format!("team:{id}");
                            let next =
                                Discovery::new(&source, &format!("team-review-requested:{slug}"));
                            i.store.ensure_source(&source)?;
                            i.store.enqueue(
                                &format!("discover:{source}"),
                                &Job::Discover(next),
                                w.manual,
                                false,
                                0,
                                false,
                            )?;
                        }
                        true
                    }
                } else if p.source == "authored" {
                    let (prs, next, total) = authored(&r)?;
                    if p.cursor.is_none() {
                        i.store.begin_source(&p.source)?;
                        p.total = Some(total);
                    }
                    if p.total != Some(total) {
                        return Err(Error::Safe("Authored membership changed during paging; reconciliation will restart.".into()));
                    }
                    p.received += prs.len() as u64;
                    i.store.stage_source(&p.source, &prs)?;
                    if let Some(cursor) = next {
                        if Some(&cursor) == p.cursor.as_ref() || prs.is_empty() {
                            return Err(schema());
                        }
                        p.cursor = Some(cursor);
                        false
                    } else {
                        if p.received != total || i.store.stage_count(&p.source)? != total {
                            return Err(schema());
                        }
                        i.store.publish_source(&p.source)?;
                        true
                    }
                } else {
                    let page = github::parse_search(&r)?;
                    if page.incomplete {
                        return Err(Error::Safe("GitHub Search reported incomplete results; prior membership was retained.".into()));
                    }
                    if p.received == 0 && p.ranges.len() == 1 && p.ranges[0].page == 1 {
                        i.store.begin_source(&p.source)?;
                    }
                    let range = p.ranges.last_mut().unwrap();
                    if page.total > 1000 {
                        if range.start == range.end {
                            return Err(Error::Safe("GitHub Search cannot enumerate more than 1,000 matching PRs created in the same second. Prior membership is retained; discovery is explicitly limited.".into()));
                        }
                        let mid = range.start + (range.end - range.start) / 2;
                        let end = range.end;
                        range.end = mid;
                        range.page = 1;
                        range.total = None;
                        range.received = 0;
                        p.ranges.push(Range {
                            start: mid + 1,
                            end,
                            page: 1,
                            total: None,
                            received: 0,
                        });
                        false
                    } else {
                        if range.total.is_some_and(|t| t != page.total) {
                            return Err(Error::Safe("Request membership changed during paging; reconciliation will restart.".into()));
                        }
                        range.total = Some(page.total);
                        range.received += page.prs.len() as u64;
                        p.received += page.prs.len() as u64;
                        i.store.stage_source(&p.source, &page.prs)?;
                        if (range.page as u64) * u64::from(p.page_size) < page.total {
                            if page.prs.is_empty() {
                                return Err(schema());
                            }
                            range.page += 1;
                            false
                        } else {
                            if range.received != page.total {
                                return Err(schema());
                            }
                            p.ranges.pop();
                            if p.ranges.is_empty() {
                                if i.store.stage_count(&p.source)? != p.received {
                                    return Err(Error::Safe("Duplicate/missing PRs across request pages require reconciliation.".into()));
                                }
                                i.store.publish_source(&p.source)?;
                                true
                            } else {
                                false
                            }
                        }
                    }
                };
                if !complete {
                    i.store.enqueue(
                        &format!("discover:{}", p.source),
                        &Job::Discover(p),
                        w.manual,
                        false,
                        0,
                        true,
                    )?;
                } else {
                    i.health="Watch discovery reconciled; head/check snapshots follow their own freshness. Activity hydration follows changed threads; native alerts follow your permission and category choices.".into();
                    self.schedule_heads_locked(&mut i, w.manual)?;
                }
            }
            Job::Heads(targets) => {
                let targets = {
                    let i = self.lock()?;
                    targets
                        .iter()
                        .filter(|t| i.store.is_active(&t.id).unwrap_or(false))
                        .cloned()
                        .collect::<Vec<_>>()
                };
                if targets.is_empty() {
                    return Ok(());
                }
                let context_size = self.lock()?.context_size;
                let r = self
                    .sync_response(
                        t,
                        ReadOp::Graph(GraphRead::Heads {
                            ids: targets.iter().map(|t| t.id.clone()).collect(),
                            size: context_size,
                        }),
                        c,
                        epoch,
                        &identity,
                        true,
                    )
                    .await?;
                let data = r.body.get("data").ok_or_else(schema)?;
                let nodes = data["nodes"].as_array().ok_or_else(schema)?;
                if nodes.len() != targets.len() {
                    return Err(schema());
                }
                let global_error = r.body["errors"]
                    .as_array()
                    .is_some_and(|es| es.iter().any(|e| e["path"][0] != "nodes"));
                if global_error {
                    return Err(schema());
                }
                let mut i = self.lock()?;
                if epoch != i.store.generation()? || c.is_cancelled() || i.locked {
                    return Ok(());
                }
                for (index, (target, node)) in targets.iter().zip(nodes).enumerate() {
                    let errors = r.body["errors"]
                        .as_array()
                        .map(|es| {
                            es.iter()
                                .filter(|e| {
                                    e["path"][0] == "nodes"
                                        && e["path"][1].as_u64() == Some(index as u64)
                                })
                                .collect::<Vec<_>>()
                        })
                        .unwrap_or_default();
                    if !node.is_null() && !errors.is_empty() {
                        if errors
                            .iter()
                            .any(|e| e["type"] == "FORBIDDEN" || e["type"] == "NOT_FOUND")
                        {
                            i.store.deny_checks(target)?;
                        } else {
                            i.store.postpone_watch(
                                &target.id,
                                now() + 30,
                                "Partial GraphQL snapshot; prior content retained",
                            )?;
                        }
                        continue;
                    }
                    if node.is_null() {
                        i.store.purge_watch(target)?;
                        return Ok(());
                    }
                    match head(node, target) {
                        Ok(read) => {
                            if let Some(current) = i.store.accept_head(&read, target)? {
                                // Keep an old overview labeled by its own SHA until explicitly read again.
                                for page in read.pages {
                                    let target = CheckTarget {
                                        page_size: i.context_size,
                                        watch: current.clone(),
                                        head: read.snapshot.head.clone(),
                                        commit: page.commit.clone(),
                                        cursor: page.next.clone().unwrap_or_default(),
                                        total: page.total,
                                    };
                                    if i.store.check_pending(&target)? {
                                        continue;
                                    }
                                    if i.store.stage_contexts(&target, &page, true)?
                                        && page.next.is_some()
                                    {
                                        let key =
                                            format!("checks:{}:{}", target.watch.id, target.commit);
                                        i.store.enqueue(
                                            &key,
                                            &Job::Contexts(target),
                                            w.manual,
                                            false,
                                            0,
                                            true,
                                        )?;
                                    }
                                }
                            }
                        }
                        Err(err) => {
                            i.store
                                .postpone_watch(&target.id, now() + 30, &err.to_string())?;
                        }
                    }
                }
                i.health="Head/check snapshots updated. Delayed freshness remains visible; eligible native alerts follow your permission and category choices.".into();
                // Do not rescan watches until the current check traversals have completed.
            }
            Job::Contexts(target) => {
                {
                    let i = self.lock()?;
                    if !i.store.valid_check(target)? {
                        return Ok(());
                    }
                }
                let response = self
                    .sync_response(
                        t,
                        ReadOp::Graph(GraphRead::Contexts {
                            repo: target.watch.repo.clone(),
                            commit: target.commit.clone(),
                            cursor: target.cursor.clone(),
                            size: target.page_size,
                        }),
                        c,
                        epoch,
                        &identity,
                        true,
                    )
                    .await;
                let r = match response {
                    Err(Error::Denied) => return self.apply_check_denial(target, epoch, c),
                    other => other?,
                };
                if r.body["errors"].as_array().is_some_and(|errors| {
                    errors.iter().any(|error| {
                        (error["type"] == "FORBIDDEN" || error["type"] == "NOT_FOUND")
                            && error["path"][0] == "repository"
                    })
                }) {
                    return self.apply_check_denial(target, epoch, c);
                }
                let data = graph_data(&r)?;
                let page = context_page(&data["repository"]["object"])?;
                if page.commit != target.commit
                    || page.total != target.total
                    || page.next.as_ref() == Some(&target.cursor)
                {
                    return Err(Error::Safe(
                        "Check traversal changed; it must be reconciled from the first page."
                            .into(),
                    ));
                }
                let mut i = self.lock()?;
                if epoch != i.store.generation()? || c.is_cancelled() || i.locked {
                    return Ok(());
                }
                if i.store.stage_contexts(target, &page, false)?
                    && let Some(cursor) = page.next
                {
                    let mut next = target.clone();
                    next.cursor = cursor;
                    i.store.enqueue(
                        &format!("checks:{}:{}", next.watch.id, next.commit),
                        &Job::Contexts(next),
                        w.manual,
                        false,
                        0,
                        true,
                    )?;
                }
            }
            _ => unreachable!(),
        }
        Ok(())
    }
}
