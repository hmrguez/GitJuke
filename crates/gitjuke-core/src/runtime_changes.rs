use crate::{
    Error, Result,
    changes::*,
    github,
    model::Snapshot,
    runtime::{Inner, Job, Runtime},
    transport::{ReadOp, Transport},
};
use tokio_util::sync::CancellationToken;

impl Runtime {
    pub(crate) fn cancel_changes_locked(i: &mut Inner) {
        i.changes_seq = i.changes_seq.saturating_add(1);
        if i.changes_running {
            i.current.cancel();
        }
        // Persisted obsolete descriptors are acknowledged without reads by execute_changes.
    }
    pub fn set_changes_mode(&self, paged: bool) -> Result<()> {
        let mut i = self.lock()?;
        let mut settings = i.store.settings()?;
        settings.paged_changes = paged;
        Self::store_operation(&mut i, |s| s.save_settings(&settings))?;
        let revision = i.store.revision()?;
        drop(i);
        (self.on_revision)(revision);
        Ok(())
    }
    pub fn cancel_changes(&self) -> Result<()> {
        Self::cancel_changes_locked(&mut *self.lock()?);
        self.notify_sync();
        Ok(())
    }
    pub fn request_changes(&self, mut job: ChangesJob) -> Result<()> {
        job.validate()?;
        let mut i = self.lock()?;
        if i.locked {
            return Err(Error::Safe(i.health.clone()));
        }
        Self::require_remote_storage(&i)?;
        if !i.reader_open || i.selected.as_ref() != Some(&(job.repo.clone(), job.number)) {
            return Err(Error::Safe("Select this PR before loading Changes.".into()));
        }
        if i.store.changes_denied(&job.repo, job.number)? {
            Self::store_operation(&mut i, |s| s.clean_changes(&job.repo, job.number))?;
        }
        Self::cancel_changes_locked(&mut i);
        job.seq = i.changes_seq;
        i.queue.discard_changes();
        Self::store_operation(&mut i, |s| {
            s.conn.execute("DELETE FROM pending WHERE seq IN (SELECT seq FROM pending WHERE key GLOB 'changes:selected:*' ORDER BY seq LIMIT 32)",[])?;
            Ok(())
        })?;
        // Unique tickets prevent a cancelled worker from acknowledging its replacement.
        Self::store_operation(&mut i, |s| {
            s.enqueue(
                &format!("changes:selected:{}", job.seq),
                &Job::Changes(job.clone()),
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
    pub fn changes_files(&self, job: &ChangesJob) -> Result<Snapshot<Option<Files>>> {
        job.validate()?;
        let mut i = self.lock()?;
        if i.locked {
            return Err(Error::Safe(i.health.clone()));
        }
        Self::cached_operation(&mut i, |s| {
            if s.changes_denied(&job.repo, job.number)? {
                return Ok(Snapshot {
                    data: None,
                    fetched_at: None,
                    complete: false,
                    error: Some("Changes access unavailable. Retry after restoring access.".into()),
                    revision: s.revision()?,
                });
            }
            s.get(&job.key())
        })
    }
    pub fn changes_patch(&self, job: &ChangesJob) -> Result<Snapshot<Option<Patch>>> {
        job.validate()?;
        let mut i = self.lock()?;
        if i.locked {
            return Err(Error::Safe(i.health.clone()));
        }
        Self::cached_operation(&mut i, |s| {
            if s.changes_denied(&job.repo, job.number)? {
                return Ok(Snapshot {
                    data: None,
                    fetched_at: None,
                    complete: false,
                    error: Some("Changes access unavailable. Retry after restoring access.".into()),
                    revision: s.revision()?,
                });
            }
            s.get(&job.key())
        })
    }
    pub fn changes_chunk(
        &self,
        job: &ChangesJob,
        digest: &str,
        chunk: usize,
    ) -> Result<Snapshot<Option<String>>> {
        job.validate()?;
        let mut i = self.lock()?;
        if i.locked {
            return Err(Error::Safe(i.health.clone()));
        }
        Self::cached_operation(&mut i, |s| {
            if s.changes_denied(&job.repo, job.number)? {
                return Err(Error::Denied);
            }
            let manifest = s.get::<Patch>(&job.key())?;
            let p = manifest
                .data
                .ok_or_else(|| Error::Safe("Patch was evicted; reload it.".into()))?;
            if p.state != "complete" || p.digest.as_deref() != Some(digest) || chunk >= p.chunks {
                return Err(Error::Safe("Obsolete patch transfer.".into()));
            }
            s.get(&format!("{}/chunk/{digest}/{chunk}", job.key()))
        })
    }
    pub fn changes_url(&self, job: &ChangesJob) -> Result<String> {
        job.validate()?;
        // Access to the PR is revalidated against the permitted cache at click time.
        let pr = self
            .overview(&job.repo, job.number)?
            .data
            .ok_or(Error::Denied)?;
        let mut url = format!("https://github.com/{}/pull/{}/files", job.repo, job.number);
        if pr.url != format!("https://github.com/{}/pull/{}", job.repo, job.number) {
            return Err(Error::Denied);
        }
        if job.index.is_some() {
            let p = self.changes_patch(job)?.data;
            if let Some(p) = p
                && p.head == job.head
                && p.base == job.base.clone().unwrap_or_default()
                && p.file.path == job.path.clone().unwrap_or_default()
                && let Some(exact) = p.file.url
            {
                url = exact;
            }
        }
        Ok(url)
    }
    fn changes_valid(
        i: &Inner,
        job: &ChangesJob,
        generation: u64,
        c: &CancellationToken,
    ) -> Result<bool> {
        Ok(!c.is_cancelled()
            && !i.locked
            && i.reader_open
            && i.changes_seq == job.seq
            && i.store.generation()? == generation
            && i.selected.as_ref() == Some(&(job.repo.clone(), job.number)))
    }
    pub(crate) async fn execute_changes(
        &self,
        t: &Transport,
        c: &CancellationToken,
        generation: u64,
        job: &ChangesJob,
    ) -> Result<()> {
        if !Self::changes_valid(&*self.lock()?, job, generation, c)? {
            return Ok(());
        }
        job.validate()?;
        let id = self.identity(t, c, generation).await?;
        let before = self
            .response(
                t,
                ReadOp::Overview {
                    repo: job.repo.clone(),
                    number: job.number,
                },
                c,
            )
            .await?;
        if matches!(before.require_ok(), Err(Error::Denied)) {
            self.lock()?.store.activity_purge(&job.repo, job.number)?;
            return Err(Error::Denied);
        }
        let rev = revision(&before, &job.repo, job.number)?;
        if rev.0 != job.head || job.base.as_ref().is_some_and(|b| b != &rev.1) {
            return Err(Error::Safe(
                "New revision available. Update Changes before loading this file.".into(),
            ));
        }
        let base = &rev.1;
        let total = rev.2;
        let (page, size) = if let Some(index) = job.index {
            (index + 1, 1)
        } else {
            (job.page, FILE_PAGE)
        };
        let response = self.changes_response(t, c, job, page, size).await?;
        // Identity and both ends of the mutable comparison must survive the complete read.
        let after = self
            .response(
                t,
                ReadOp::Overview {
                    repo: job.repo.clone(),
                    number: job.number,
                },
                c,
            )
            .await?;
        if matches!(after.require_ok(), Err(Error::Denied)) {
            self.lock()?.store.activity_purge(&job.repo, job.number)?;
            return Err(Error::Denied);
        }
        if revision(&after, &job.repo, job.number)? != rev {
            return Err(Error::Safe(
                "The PR changed during this read. Update Changes; no mixed revision was published."
                    .into(),
            ));
        }
        let after_id = self.identity(t, c, generation).await?;
        if id.id != after_id.id {
            return Err(Error::Safe("Identity changed during Changes read.".into()));
        }
        let mut i = self.lock()?;
        if !Self::changes_valid(&i, job, generation, c)? {
            return Ok(());
        }
        if let Err(Error::Denied) = response.require_ok() {
            i.store.deny_changes(&job.repo, job.number)?;
            return Err(Error::Denied);
        }
        response.require_ok()?;
        let mut pending = Vec::new();
        if let Some(index) = job.index {
            let items = response
                .body
                .as_array()
                .ok_or_else(|| Error::Safe("Missing selected file.".into()))?;
            if items.len() != 1 {
                return Err(Error::Safe(
                    "Selected file is no longer present; update Changes.".into(),
                ));
            }
            let f = file(&items[0], index, &job.repo, &job.head, base)?;
            if Some(&f.path) != job.path.as_ref() {
                return Err(Error::Safe(
                    "File order changed during navigation. Update Changes.".into(),
                ));
            }
            Self::store_changes_patch(&mut pending, job, &items[0], f, base)?;
        } else {
            let data = files(&response, job, base, total, FILE_PAGE)?;
            let items = response.body.as_array().expect("files validated the array");
            // Retain eligible supplied patches before making their metadata visible.
            // The automatic tier still labels large content instead of opting into it.
            for (value, file) in items.iter().zip(&data.items) {
                if value.get("patch").is_none() {
                    continue;
                }
                let patch_job = ChangesJob {
                    base: Some(base.clone()),
                    index: Some(file.index),
                    path: Some(file.path.clone()),
                    large: false,
                    ..job.clone()
                };
                Self::store_changes_patch(&mut pending, &patch_job, value, file.clone(), base)?;
                while pending.len() >= 12 {
                    i.store.put_changes_batch(&pending[..12], generation)?;
                    pending.drain(..12);
                }
            }
            i.store.put_changes_batch(&pending, generation)?;
            pending.clear();
            i.store.put(&job.key(), &data, generation)?;
        }
        for rows in pending.chunks(12) {
            i.store.put_changes_batch(rows, generation)?;
        }
        i.store.restore_changes(&job.repo, job.number)?;
        Ok(())
    }
    fn store_changes_patch(
        rows: &mut Vec<(String, String)>,
        job: &ChangesJob,
        value: &serde_json::Value,
        file: File,
        base: &str,
    ) -> Result<()> {
        let (manifest, chunks) = patch(value, file, &job.head, base, job.large);
        // Keep chunks before their final digest-bound manifest, even across commits.
        for (n, chunk) in chunks.iter().enumerate() {
            rows.push((
                format!(
                    "{}/chunk/{}/{n}",
                    job.key(),
                    manifest.digest.as_deref().unwrap_or_default()
                ),
                serde_json::to_string(chunk)?,
            ));
        }
        rows.push((job.key(), serde_json::to_string(&manifest)?));
        Ok(())
    }
    async fn changes_response(
        &self,
        t: &Transport,
        c: &CancellationToken,
        job: &ChangesJob,
        page: u32,
        size: u16,
    ) -> Result<github::Response> {
        let r = self
            .response(
                t,
                ReadOp::Files {
                    repo: job.repo.clone(),
                    number: job.number,
                    page,
                    size,
                },
                c,
            )
            .await;
        // Split the same logical metadata window when a multi-file transport overflows.
        if r.as_ref()
            .err()
            .is_some_and(|e| e.to_string().contains("output limit"))
            && size > 1
        {
            let start = (page - 1) * u32::from(size);
            let mut all = Vec::new();
            let mut retained_bytes = 0;
            for index in start..start + u32::from(size) {
                let one = self
                    .response(
                        t,
                        ReadOp::Files {
                            repo: job.repo.clone(),
                            number: job.number,
                            page: index + 1,
                            size: 1,
                        },
                        c,
                    )
                    .await?;
                if !(200..300).contains(&one.status) {
                    return Ok(one);
                }
                let a = one
                    .body
                    .as_array()
                    .ok_or_else(|| Error::Safe("Incomplete files page.".into()))?;
                if a.len() > 1 {
                    return Err(Error::Safe("Invalid files page.".into()));
                }
                if a.is_empty() {
                    break;
                }
                // Keep the fallback bounded too. Omitted patches remain cold and
                // can be requested individually; never cache omission as "missing".
                let mut v = a[0].clone();
                let bytes = serde_json::to_vec(&v)?.len();
                if retained_bytes + bytes > crate::transport::STDOUT_LIMIT
                    && let Some(object) = v.as_object_mut()
                {
                    object.remove("patch");
                }
                let stored_bytes = serde_json::to_vec(&v)?.len();
                if retained_bytes + stored_bytes > crate::transport::STDOUT_LIMIT {
                    return Err(Error::Safe(
                        "Files metadata exceeds the transport limit. Open GitHub Changes.".into(),
                    ));
                }
                retained_bytes += stored_bytes;
                all.push(v);
            }
            return Ok(github::Response {
                status: 200,
                headers: Default::default(),
                body: serde_json::Value::Array(all),
            });
        }
        r
    }
}
