use crate::{
    Error, Result,
    conversation::*,
    model::Snapshot,
    runtime::{Inner, Job, Runtime},
    transport::{ReadOp, Transport},
};
use tokio_util::sync::CancellationToken;
impl Runtime {
    pub(crate) fn cancel_conversation_locked(i: &mut Inner) {
        i.conversation_seq = i.conversation_seq.saturating_add(1);
        if i.conversation_running {
            i.current.cancel();
        }
    }
    pub fn cancel_conversation(&self) -> Result<()> {
        Self::cancel_conversation_locked(&mut *self.lock()?);
        self.notify_sync();
        Ok(())
    }
    pub fn request_conversation(&self, mut job: ConversationJob) -> Result<()> {
        job.validate()?;
        let mut i = self.lock()?;
        if i.locked {
            return Err(Error::Safe(i.health.clone()));
        }
        Self::require_remote_storage(&i)?;
        if job.source == "destination" {
            let id = format!(
                "github.com:{}:{}:{}",
                job.repo,
                job.number,
                job.thread.as_deref().unwrap()
            );
            let retained: bool = i.store.conn.query_row("SELECT EXISTS(SELECT 1 FROM activity WHERE id=?1 AND repo=?2 AND number=?3 AND kind IN ('comment','review','mention') AND occurred>=?4)", rusqlite::params![id, job.repo, job.number as i64, crate::now().saturating_sub(90*86400) as i64], |r| r.get(0))?;
            if !retained {
                return Err(Error::Denied);
            }
        }
        if !i.reader_open || i.selected.as_ref() != Some(&(job.repo.clone(), job.number)) {
            return Err(Error::Safe(
                "Select this PR before reading Conversation.".into(),
            ));
        }
        if i.store.conversation_denied(&job.repo, job.number)? {
            Self::store_operation(&mut i, |s| s.clean_conversation(&job.repo, job.number))?;
        }
        Self::cancel_conversation_locked(&mut i);
        job.seq = i.conversation_seq;
        i.queue.discard_conversation();
        Self::store_operation(&mut i, |s| {
            s.conn.execute("DELETE FROM pending WHERE seq IN (SELECT seq FROM pending WHERE key GLOB 'conversation:selected:*' ORDER BY seq LIMIT 32)",[])?;
            Ok(())
        })?;
        Self::store_operation(&mut i, |s| {
            s.enqueue(
                &format!("conversation:selected:{}", job.seq),
                &Job::Conversation(job.clone()),
                true,
                true,
                0,
                true,
            )
        })?;
        Self::store_operation(&mut i, |s| {
            s.error(
                &job.key(),
                "Queued — waiting for the serialized GitHub reader and server deadlines.",
            )
        })?;
        drop(i);
        self.notify_sync();
        Ok(())
    }
    pub fn conversation(
        &self,
        job: &ConversationJob,
    ) -> Result<Snapshot<Option<ConversationPage>>> {
        job.validate()?;
        let mut i = self.lock()?;
        if i.locked {
            return Err(Error::Safe(i.health.clone()));
        }
        Self::cached_operation(&mut i, |s| {
            if s.conversation_denied(&job.repo, job.number)? {
                return Ok(Snapshot {
                    data: None,
                    fetched_at: None,
                    complete: false,
                    error: Some(
                        "Conversation access unavailable. Restore access and retry.".into(),
                    ),
                    revision: s.revision()?,
                });
            }
            s.get(&job.key())
        })
    }
    pub fn conversation_url(&self, job: &ConversationJob, id: &str) -> Result<String> {
        let page = self.conversation(job)?.data.ok_or(Error::Denied)?;
        let url = page
            .messages
            .iter()
            .find(|m| m.id == id)
            .and_then(|m| m.url.clone())
            .or_else(|| {
                page.threads
                    .iter()
                    .find(|t| t.id == id)
                    .and_then(|t| t.url.clone())
            })
            .ok_or(Error::Denied)?;
        source_url(&url, &job.repo, job.number).ok_or(Error::Denied)
    }
    pub fn activity_rendered(&self, id: &str, text: &str) -> Result<()> {
        if id.len() > 1024 || text.len() > 8192 {
            return Err(Error::Denied);
        }
        let mut i = self.lock()?;
        if i.locked {
            return Err(Error::Denied);
        }
        let record = i
            .store
            .get::<crate::activity::ActivityRecord>(&format!("activity-payload:{id}"))?
            .data
            .ok_or(Error::Denied)?;
        if record.text.as_deref() != Some(text)
            || (record.kind != "check"
                && i.store.conversation_denied(&record.repo, record.number)?)
        {
            return Err(Error::Denied);
        }
        Self::store_operation(&mut i, |s| s.activity_seen(&[id.to_owned()]))?;
        let revision = i.store.revision()?;
        drop(i);
        (self.on_revision)(revision);
        Ok(())
    }
    /// Called only after an explicitly opened source has committed to the DOM.
    pub fn conversation_seen(
        &self,
        job: &ConversationJob,
        id: &str,
        digest: &str,
        generation: u64,
    ) -> Result<()> {
        let mut i = self.lock()?;
        if i.locked
            || !i.reader_open
            || i.selected.as_ref() != Some(&(job.repo.clone(), job.number))
            || i.store.generation()? != generation
            || i.store.conversation_denied(&job.repo, job.number)?
        {
            return Err(Error::Denied);
        }
        job.validate()?;
        let overview = i
            .store
            .get::<crate::model::Overview>(&format!("pr:{}:{}", job.repo, job.number))?
            .data;
        if overview.is_some_and(|overview| overview.head != job.head) {
            return Err(Error::Denied);
        }
        let page = i
            .store
            .get::<ConversationPage>(&job.key())?
            .data
            .ok_or(Error::Denied)?;
        if page.generation != generation
            || !page.messages.iter().any(|m| {
                m.id == id
                    && m.state == "complete"
                    && m.body.is_some()
                    && m.digest.as_deref() == Some(digest)
            })
        {
            return Err(Error::Denied);
        }
        Self::store_operation(&mut i, |s| {
            s.activity_seen(&[format!("github.com:{}:{}:{}", job.repo, job.number, id)])
        })?;
        let revision = i.store.revision()?;
        drop(i);
        (self.on_revision)(revision);
        Ok(())
    }
    fn conversation_valid(
        i: &Inner,
        j: &ConversationJob,
        g: u64,
        c: &CancellationToken,
    ) -> Result<bool> {
        Ok(!c.is_cancelled()
            && !i.locked
            && i.reader_open
            && i.conversation_seq == j.seq
            && i.store.generation()? == g
            && i.selected.as_ref() == Some(&(j.repo.clone(), j.number)))
    }
    pub(crate) async fn execute_conversation(
        &self,
        t: &Transport,
        c: &CancellationToken,
        g: u64,
        j: &ConversationJob,
    ) -> Result<()> {
        if !Self::conversation_valid(&*self.lock()?, j, g, c)? {
            return Ok(());
        }
        j.validate()?;
        let id = self.identity(t, c, g).await?;
        let before = self
            .response(
                t,
                ReadOp::Overview {
                    repo: j.repo.clone(),
                    number: j.number,
                },
                c,
            )
            .await?;
        if matches!(before.require_ok(), Err(gitjuke_github::Error::Denied)) {
            let mut i = self.lock()?;
            if Self::conversation_valid(&i, j, g, c)? {
                i.store.activity_purge(&j.repo, j.number)?;
            }
            return Err(Error::Denied);
        }
        let rev = crate::changes::revision(&before, &j.repo, j.number)?;
        if rev.0 != j.head {
            return Err(Error::Safe(
                "Conversation is from an old revision. Update the PR.".into(),
            ));
        }
        let mut first = if j.source == "destination" {
            1
        } else if j.source == "threads" {
            20
        } else {
            50
        };
        let mut page = loop {
            let response = self
                .response(t, ReadOp::Conversation(j.query(first)), c)
                .await;
            if response
                .as_ref()
                .err()
                .is_some_and(|e| e.to_string().contains("output limit"))
                && first > 1
            {
                first = (first / 2).max(1);
                continue;
            }
            let response = response?;
            let denied = matches!(response.require_ok(), Err(gitjuke_github::Error::Denied))
                || response.body["errors"].as_array().is_some_and(|a| {
                    a.iter()
                        .any(|v| matches!(v["type"].as_str(), Some("FORBIDDEN" | "NOT_FOUND")))
                });
            if denied {
                let _ = self.identity(t, c, g).await?;
                let mut i = self.lock()?;
                if Self::conversation_valid(&i, j, g, c)? {
                    i.store.deny_conversation(&j.repo, j.number)?;
                }
                return Err(Error::Denied);
            }
            response.require_ok()?;
            let mut p = parse(&response.body, j, first)?;
            if j.source == "destination"
                && response.body["data"]["node"]["__typename"] == "PullRequestReviewComment"
                && p.messages[0].state == "complete"
            {
                // fullDatabaseId is a BigInt scalar serialized as a decimal string.
                let value = &response.body["data"]["node"]["fullDatabaseId"];
                let database_id = value
                    .as_str()
                    .and_then(|s| s.parse::<u64>().ok())
                    .or_else(|| value.as_u64())
                    .filter(|n| *n > 0);
                if let Some(database_id) = database_id {
                    let coordinates = self
                        .response(
                            t,
                            ReadOp::Activity(crate::activity::ActivityRead {
                                url: format!("repos/{}/pulls/comments/{database_id}", j.repo),
                                etag: None,
                                modified: None,
                            }),
                            c,
                        )
                        .await?;
                    if matches!(coordinates.require_ok(), Err(gitjuke_github::Error::Denied)) {
                        let mut i = self.lock()?;
                        if Self::conversation_valid(&i, j, g, c)? {
                            i.store.deny_conversation(&j.repo, j.number)?;
                        }
                        return Err(Error::Denied);
                    }
                    coordinates.require_ok()?;
                    p.destination_anchor =
                        Some(destination_anchor(&coordinates.body, j, &p.messages[0])?);
                }
            }
            if serde_json::to_vec(&p)?.len() > PAGE_BYTES
                || p.messages
                    .iter()
                    .filter_map(|m| m.body.as_ref())
                    .map(String::len)
                    .sum::<usize>()
                    > 2 * 1024 * 1024
            {
                if first == 1 {
                    return Err(Error::Safe(
                        "Conversation metadata exceeds the page limit. Open GitHub.".into(),
                    ));
                }
                first = (first / 2).max(1);
                continue;
            }
            break p;
        };
        let after = self
            .response(
                t,
                ReadOp::Overview {
                    repo: j.repo.clone(),
                    number: j.number,
                },
                c,
            )
            .await?;
        if matches!(after.require_ok(), Err(gitjuke_github::Error::Denied)) {
            let mut i = self.lock()?;
            if Self::conversation_valid(&i, j, g, c)? {
                i.store.activity_purge(&j.repo, j.number)?;
            }
            return Err(Error::Denied);
        }
        if crate::changes::revision(&after, &j.repo, j.number)? != rev {
            return Err(Error::Safe(
                "PR changed during Conversation read; no mixed revision was published.".into(),
            ));
        }
        let after_id = self.identity(t, c, g).await?;
        if after_id.id != id.id {
            return Err(Error::Denied);
        }
        let mut i = self.lock()?;
        page.generation = g;
        if Self::conversation_valid(&i, j, g, c)? {
            i.store.put(&j.key(), &page, g)?;
            i.store.restore_conversation(&j.repo, j.number)?;
        }
        Ok(())
    }
}
