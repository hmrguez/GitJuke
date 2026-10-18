use crate::{
    Error, Result,
    activity::*,
    now,
    runtime::{Inner, Job, Runtime, Work},
    transport::{ReadOp, Transport},
};
use rusqlite::params;
use tokio_util::sync::CancellationToken;
impl Runtime {
    pub fn refresh_activity(&self, repo: &str, number: u64) -> Result<()> {
        let result = self.refresh_activity_inner(repo, number);
        self.protect_storage(result)
    }
    fn refresh_activity_inner(&self, repo: &str, number: u64) -> Result<()> {
        if !crate::github::valid_repo(repo) || number == 0 {
            return Err(Error::Safe("Invalid activity locator.".into()));
        }
        let mut i = self.lock()?;
        Self::require_remote_storage(&i)?;
        if i.locked {
            return Err(Error::Safe(i.health.clone()));
        }
        i.store.conn.execute("INSERT INTO activity_target VALUES(?,?,0,0) ON CONFLICT(repo,number) DO UPDATE SET done=0,due=0",params![repo,number as i64])?;
        let job = ActivityJob::Hydrate {
            repo: repo.into(),
            number,
            source: 0,
            page: 1,
            size: 50,
        };
        i.store.enqueue(
            &job.key(),
            &Job::Activity(job.clone()),
            true,
            true,
            0,
            false,
        )?;
        drop(i);
        self.notify_sync();
        Ok(())
    }
    pub fn activity(&self, offset: usize) -> Result<ActivityPage> {
        let mut i = self.lock()?;
        if i.locked {
            return Err(Error::Safe(i.health.clone()));
        }
        i.store.activity_page(offset)
    }
    pub fn mark_activity_seen(&self, ids: &[String]) -> Result<()> {
        let mut i = self.lock()?;
        if i.locked {
            return Err(Error::Safe(i.health.clone()));
        }
        Self::store_operation(&mut i, |s| s.activity_seen(ids))?;
        let revision = i.store.revision()?;
        drop(i);
        (self.on_revision)(revision);
        Ok(())
    }
    pub(crate) fn schedule_activity(&self, i: &mut Inner, manual: bool) -> Result<()> {
        i.store.activity_targets()?;
        if i.store.due("activity:notifications")? <= now() {
            let since = i.store.activity_progress("activity:notifications")?;
            let job = ActivityJob::Notifications {
                page: 1,
                size: 50,
                since,
                before: now(),
            };
            i.store.enqueue(
                &job.key(),
                &Job::Activity(job.clone()),
                manual,
                false,
                0,
                false,
            )?;
        }
        for (repo, number) in i.store.activity_due(now())? {
            let job = ActivityJob::Hydrate {
                repo,
                number,
                source: 0,
                page: 1,
                size: 50,
            };
            i.store.enqueue(
                &job.key(),
                &Job::Activity(job.clone()),
                manual,
                false,
                0,
                false,
            )?;
        }
        i.store.activity_finish_recovery(manual)?;
        Ok(())
    }
    pub(crate) async fn execute_activity(
        &self,
        w: &Work,
        t: &Transport,
        c: &CancellationToken,
        epoch: u64,
        job: &ActivityJob,
    ) -> Result<()> {
        let id = self.identity(t, c, epoch).await?;
        let url = job.url();
        let (etag, modified, _) = { self.lock()?.store.activity_validator(&url)? };
        let response = self
            .response(
                t,
                ReadOp::Activity(ActivityRead {
                    url: url.clone(),
                    etag,
                    modified,
                }),
                c,
            )
            .await?;
        let after = self.identity(t, c, epoch).await?;
        if after.id != id.id {
            return Err(Error::Safe(
                "Identity changed during activity hydration.".into(),
            ));
        }
        let mut i = self.lock()?;
        if c.is_cancelled() || i.locked || i.store.generation()? != epoch {
            return Ok(());
        }
        if response.status != 304
            && let Err(err) = response.require_ok()
        {
            if matches!(err, Error::Denied)
                && let ActivityJob::Hydrate { repo, number, .. } = job
            {
                i.store.activity_purge(repo, *number)?;
            }
            return Err(err);
        }
        #[derive(serde::Serialize, serde::Deserialize)]
        struct Cached {
            body: serde_json::Value,
            next: bool,
        }
        let key = format!("api:{url}");
        let cached = if response.status == 304 {
            match i.store.get::<Cached>(&key)?.data {
                Some(data) => data,
                None => {
                    // Validators cannot stand in for evicted payloads.
                    i.store.conn.execute(
                        "UPDATE api_validator SET etag=NULL,modified=NULL WHERE url=?",
                        [&url],
                    )?;
                    return Err(Error::Safe("Conditional activity payload was evicted; queued an unconditional read under the existing deadline.".into()));
                }
            }
        } else {
            let cached = Cached {
                body: response.body,
                next: response
                    .headers
                    .get("link")
                    .is_some_and(|s| s.contains("rel=\"next\"")),
            };
            i.store.put(&key, &cached, epoch)?;
            i.store.conn.execute("INSERT INTO api_validator(url,etag,modified) VALUES(?,?,?) ON CONFLICT(url) DO UPDATE SET etag=excluded.etag,modified=excluded.modified",params![url,response.headers.get("etag"),response.headers.get("last-modified")])?;
            cached
        };
        match job {
            ActivityJob::Notifications {
                page,
                size,
                since,
                before,
            } => {
                let targets = notification_targets(&cached.body)?;
                let count = cached
                    .body
                    .as_array()
                    .ok_or_else(|| Error::Safe("Invalid notifications page.".into()))?
                    .len();
                if count > *size as usize || *size > 50 {
                    return Err(Error::Safe("Notification page exceeded its requested source bound; progress was not published.".into()));
                }
                if cached.next && count == 0 {
                    return Err(Error::Safe("Notification pagination is incomplete.".into()));
                }
                i.store
                    .activity_notifications(&targets, (!cached.next).then_some(*before))?;
                if cached.next {
                    let next = ActivityJob::Notifications {
                        page: page + 1,
                        size: *size,
                        since: *since,
                        before: *before,
                    };
                    i.store.enqueue(
                        &next.key(),
                        &Job::Activity(next.clone()),
                        w.manual,
                        false,
                        0,
                        true,
                    )?;
                }
            }
            ActivityJob::Hydrate {
                repo,
                number,
                source,
                page,
                size,
            } => {
                if *source == 0 {
                    let overview = crate::github::parse_overview(
                        &crate::github::Response {
                            status: 200,
                            headers: Default::default(),
                            body: cached.body.clone(),
                        },
                        repo,
                        *number,
                    )?;
                    if overview.repo != *repo || overview.number != *number {
                        return Err(Error::Safe(
                            "Activity PR identity did not match its locator.".into(),
                        ));
                    }
                    let authored = cached.body["user"]["id"].as_u64() == Some(id.id);
                    i.store.put(
                        &format!("activity-owner:{repo}:{number}"),
                        &(authored, overview.head),
                        epoch,
                    )?;
                }
                let (authored, source_head) = i
                    .store
                    .get::<(bool, String)>(&format!("activity-owner:{repo}:{number}"))?
                    .data
                    .ok_or_else(|| {
                        Error::Safe(
                            "Activity author context was evicted; restart hydration.".into(),
                        )
                    })?;
                let teams = {
                    let mut q = i.store.conn.prepare("SELECT id,slug FROM teams")?;
                    q.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
                        .collect::<std::result::Result<Vec<_>, _>>()?
                };
                let items = records(repo, *number, *source, &cached.body, &id, &teams, authored)?;
                i.store
                    .activity_stage(repo, *number, &items, *source == 0)?;
                let next = if cached.next {
                    if cached.body.as_array().is_none_or(|v| v.is_empty()) {
                        return Err(Error::Safe("Activity pagination did not progress.".into()));
                    }
                    Some(ActivityJob::Hydrate {
                        repo: repo.clone(),
                        number: *number,
                        source: *source,
                        page: page + 1,
                        size: *size,
                    })
                } else if *source < 4 {
                    Some(ActivityJob::Hydrate {
                        repo: repo.clone(),
                        number: *number,
                        source: source + 1,
                        page: 1,
                        size: *size,
                    })
                } else {
                    None
                };
                if let Some(next) = next {
                    i.store.enqueue(
                        &next.key(),
                        &Job::Activity(next.clone()),
                        w.manual,
                        false,
                        0,
                        true,
                    )?;
                } else {
                    let checks =
                        i.store
                            .activity_check_records(repo, *number, authored, &source_head)?;
                    i.store.activity_stage(repo, *number, &checks, false)?;
                    i.store.activity_publish(repo, *number, w.manual, &id)?;
                    i.store.conn.execute("INSERT INTO resource_state VALUES(?,?,1,NULL) ON CONFLICT(key) DO UPDATE SET fetched=excluded.fetched,complete=1,error=NULL",params![job.key(),now() as i64])?;
                }
            }
        }
        if matches!(job, ActivityJob::Notifications { .. }) {
            i.store.conn.execute("INSERT INTO resource_state VALUES('activity:notifications',?,1,NULL) ON CONFLICT(key) DO UPDATE SET fetched=excluded.fetched,complete=1,error=NULL",[now() as i64])?;
        }
        Ok(())
    }
}
