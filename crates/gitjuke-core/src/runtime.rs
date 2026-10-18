use crate::sync::{CheckTarget, Discovery, WatchTarget};
use crate::{
    Error, Result,
    github::{self, Response},
    model::*,
    now,
    store::{Store, storage_healthy},
    transport::{self, ReadOp, Transport},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::VecDeque,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Job {
    Activity(crate::activity::ActivityJob),
    Discover(Discovery),
    Heads(Vec<WatchTarget>),
    Contexts(CheckTarget),
    Bootstrap,
    Authored(u32),
    Overview(String, u64),
    Stacks(String, u64, String),
    StackDetail(String, u64, String, u64),
}
impl Job {
    pub(crate) fn key(&self) -> String {
        match self {
            Self::Activity(p) => p.key(),
            Self::Discover(p) => p.source.clone(),
            Self::Heads(targets) => format!(
                "heads:{}",
                targets.first().map(|t| t.id.as_str()).unwrap_or_default()
            ),
            Self::Contexts(t) => format!("checks:{}:{}", t.watch.id, t.commit),
            Self::Bootstrap => "identity".into(),
            Self::Authored(_) => "authored".into(),
            Self::Overview(r, n) | Self::Stacks(r, n, _) | Self::StackDetail(r, n, _, _) => {
                format!("pr:{r}:{n}")
            }
        }
    }
    fn coalesce_key(&self) -> String {
        format!("{self:?}")
    }
}
#[derive(Clone)]
pub(crate) struct Work {
    pub(crate) job: Job,
    pub(crate) manual: bool,
    pub(crate) foreground: bool,
    pub(crate) ticket: Option<i64>,
}
#[derive(Default)]
pub struct Queue {
    jobs: VecDeque<Work>,
    foreground_run: usize,
}
impl Queue {
    fn push(&mut self, w: Work) -> bool {
        if self
            .jobs
            .iter()
            .any(|j| j.job.coalesce_key() == w.job.coalesce_key())
        {
            return true;
        }
        if self.jobs.len() >= 128 {
            return false;
        }
        self.jobs.push_back(w);
        true
    }
    fn pop(&mut self) -> Option<Work> {
        let bg = self.jobs.iter().position(|w| !w.foreground);
        let fg = self.jobs.iter().position(|w| w.foreground);
        let index = if self.foreground_run >= 3 {
            bg.or(fg)
        } else {
            fg.or(bg)
        }?;
        let w = self.jobs.remove(index)?;
        if w.foreground {
            self.foreground_run += 1
        } else {
            self.foreground_run = 0
        }
        Some(w)
    }
}
pub(crate) struct Inner {
    pub(crate) store: Store,
    pub(crate) transport: Option<Transport>,
    pub(crate) queue: Queue,
    pub(crate) locked: bool,
    pub(crate) health: String,
    pub(crate) version: Option<String>,
    pub(crate) busy: bool,
    pub(crate) current: CancellationToken,
    pub(crate) selected: Option<(String, u64)>,
    pub(crate) reader_open: bool,
    pub(crate) reader_visible: bool,
    pub(crate) batch_size: usize,
    pub(crate) context_size: u16,
    pub(crate) path: PathBuf,
    last_tick: u64,
    storage_stopped: bool,
    storage_check_at: u64,
    startup_deferred: bool,
    storage_check: StorageCheck,
}
type StorageCheck = Arc<dyn Fn(&std::path::Path) -> Result<()> + Send + Sync>;

#[derive(Clone)]
pub struct Runtime {
    pub(crate) inner: Arc<Mutex<Inner>>,
    notify: Arc<Notify>,
    stop: CancellationToken,
    finished: CancellationToken,
    pub(crate) on_revision: Arc<dyn Fn(u64) + Send + Sync>,
}
impl Runtime {
    pub fn start(path: PathBuf, on_revision: impl Fn(u64) + Send + Sync + 'static) -> Result<Self> {
        Self::start_checked(path, on_revision, Arc::new(storage_healthy))
    }
    fn start_checked(
        path: PathBuf,
        on_revision: impl Fn(u64) + Send + Sync + 'static,
        storage_check: StorageCheck,
    ) -> Result<Self> {
        let directory = path
            .parent()
            .ok_or_else(|| Error::Safe("Missing application data directory.".into()))?;
        let prepared = (|| -> Result<Store> {
            storage_check(directory)?;
            let mut store = Store::open(&path)?;
            store.activity_recovery(now())?;
            Ok(store)
        })();
        let (store, startup_deferred) = match prepared {
            Ok(store) => (store, false),
            Err(err) if Self::storage_error(&err) && path.exists() => {
                (Store::open_cached(&path)?, true)
            }
            Err(err) => return Err(err),
        };
        let settings = store.settings()?;
        let selected = store.selection()?;
        let transport =
            Transport::discover(settings.gh_path.as_deref(), settings.config_dir.as_deref());
        let health=transport.as_ref().err().map(ToString::to_string).unwrap_or_else(||if settings.paused{"Paused. Use Refresh to validate the configured identity and read cached content.".into()}else{"Validating stored gh identity…".into()});
        let mut queue = Queue::default();
        if !settings.paused && transport.is_ok() {
            queue.push(Work {
                ticket: None,
                job: Job::Bootstrap,
                manual: false,
                foreground: false,
            });
        }
        let runtime = Self {
            inner: Arc::new(Mutex::new(Inner {
                store,
                transport: transport.ok(),
                queue,
                locked: true,
                health,
                version: None,
                busy: false,
                current: CancellationToken::new(),
                selected,
                reader_open: true,
                reader_visible: true,
                batch_size: 10,
                context_size: 25,
                path,
                last_tick: now(),
                storage_stopped: startup_deferred,
                startup_deferred,
                storage_check,
                storage_check_at: now() + 60,
            })),
            notify: Arc::new(Notify::new()),
            stop: CancellationToken::new(),
            finished: CancellationToken::new(),
            on_revision: Arc::new(on_revision),
        };
        let worker = runtime.clone();
        tokio::spawn(async move {
            worker.run().await;
        });
        Ok(runtime)
    }
    pub(crate) fn lock(&self) -> Result<std::sync::MutexGuard<'_, Inner>> {
        self.inner
            .lock()
            .map_err(|_| Error::Safe("Runtime stopped after an internal failure.".into()))
    }
    pub(crate) fn notify_sync(&self) {
        self.notify.notify_one();
    }
    pub fn status(&self) -> Result<Status> {
        let i = self.lock()?;
        Ok(Status {
            settings: i.store.settings()?,
            identity: i.store.identity()?,
            locked: i.locked,
            health: if i.storage_stopped {
                "Storage protection: remote reads and alerts are stopped until a free-space, committed-write and checkpoint probe succeeds. Cached reading remains available after identity validation; local changes require a successful write.".into()
            } else {
                i.health.clone()
            },
            storage_stopped: i.storage_stopped,
            storage: i.store.storage_usage(&i.path)?,
            gh_version: i.version.clone(),
            gh_path: i.transport.as_ref().map(|t| t.path.display().to_string()),
            config_dir: i
                .transport
                .as_ref()
                .map(|t| t.config_dir.display().to_string())
                .unwrap_or_default(),
            ignored_overrides: transport::ignored_overrides(),
            revision: i.store.revision()?,
            queued: i.queue.jobs.len(),
            busy: i.busy,
            next_remote_at: i.store.due("remote")?,
            discovery_more: i.store.teams_due()?.len() + 3 > 100,
            watch_count: i.store.active_counts()?.0,
            delayed_watches: i.store.active_counts()?.1,
            discovery: i.store.health()?,
            budgets: ["core", "search", "graphql"]
                .into_iter()
                .filter_map(|r| i.store.budget(r).transpose())
                .collect::<Result<Vec<_>>>()?,
            selected: i.selected.clone(),
        })
    }
    pub fn page(&self, offset: usize) -> Result<Page> {
        let mut i = self.lock()?;
        if i.locked {
            return Err(Error::Safe(i.health.clone()));
        }
        Self::cached_operation(&mut i, |s| s.list_page("authored", offset))
    }
    pub fn watches(&self, offset: usize) -> Result<WatchPage> {
        let i = self.lock()?;
        if i.locked {
            return Err(Error::Safe(i.health.clone()));
        }
        i.store.watch_page(offset)
    }
    pub fn checks(&self, repo: &str, number: u64, offset: usize) -> Result<CheckPage> {
        let mut i = self.lock()?;
        if i.locked {
            return Err(Error::Safe(i.health.clone()));
        }
        Self::cached_operation(&mut i, |s| s.check_page(repo, number, offset))
    }
    pub fn overview(&self, repo: &str, number: u64) -> Result<Snapshot<Option<Overview>>> {
        let mut i = self.lock()?;
        if i.locked {
            return Err(Error::Safe(i.health.clone()));
        }
        Self::cached_operation(&mut i, |s| s.get(&format!("pr:{repo}:{number}")))
    }
    pub fn refresh(&self) -> Result<()> {
        let mut i = self.lock()?;
        if i.storage_stopped {
            return Err(Error::StorageUnavailable("Storage protection prevents remote reads. Cached reading remains available after identity validation.".into()));
        }
        let job = if i.locked {
            Job::Bootstrap
        } else {
            drop(i);
            return self.request_discovery(true);
        };
        // Explicit reads are allowed while paused, but never resume monitoring or create alerts.
        if !i.queue.push(Work {
            ticket: None,
            job,
            manual: true,
            foreground: true,
        }) {
            return Err(Error::Safe(
                "Read queue is full; work remains due. Try again after a read completes.".into(),
            ));
        }
        drop(i);
        self.notify.notify_one();
        Ok(())
    }
    pub fn select(&self, repo: String, number: u64) -> Result<()> {
        if !github::valid_repo(&repo) || number == 0 {
            return Err(Error::Safe("Invalid pull request identity.".into()));
        }
        let mut i = self.lock()?;
        if i.locked {
            return Err(Error::Safe(i.health.clone()));
        }
        i.selected = Some((repo.clone(), number));
        // During protection, cached navigation is in-memory only. Do not persist
        // a selection or queue a remote read when writes are unavailable.
        if i.storage_stopped {
            i.reader_open = true;
            return Ok(());
        }
        let selection = i.selected.clone();
        Self::store_operation(&mut i, |s| s.remember_selection(selection.as_ref(), true))?;
        i.reader_open = true;
        if !i.queue.push(Work {
            ticket: None,
            job: Job::Overview(repo, number),
            manual: true,
            foreground: true,
        }) {
            return Err(Error::Safe(
                "The read queue is full; retry after a job completes.".into(),
            ));
        }
        drop(i);
        self.notify.notify_one();
        Ok(())
    }
    pub fn reader_visibility(&self, visible: bool) -> Result<()> {
        self.lock()?.reader_visible = visible;
        Ok(())
    }
    pub fn reader_closed(&self) -> Result<()> {
        let mut i = self.lock()?;
        i.reader_open = false;
        i.reader_visible = false;
        let selected = i.selected.clone();
        Self::store_operation(&mut i, |s| s.remember_selection(selected.as_ref(), false))?;
        Ok(())
    }
    pub fn reader_opened(&self) -> Result<()> {
        let mut i = self.lock()?;
        i.reader_open = true;
        i.reader_visible = true;
        let selected = i.selected.clone();
        Self::store_operation(&mut i, |s| s.remember_selection(selected.as_ref(), true))?;
        drop(i);
        self.notify.notify_one();
        Ok(())
    }
    pub fn set_pause(&self, paused: bool) -> Result<()> {
        let mut i = self.lock()?;
        let mut settings = i.store.settings()?;
        settings.paused = paused;
        Self::store_operation(&mut i, |s| s.save_settings(&settings))?;
        if paused {
            i.current.cancel();
            i.queue.jobs.retain(|w| w.manual);
            i.health = "Paused — cached reading and explicit refresh remain available.".into();
        } else {
            Self::store_operation(&mut i, |s| s.activity_recovery(now()))?;
            i.queue.jobs.retain(|w| !matches!(w.job, Job::Activity(_)));
            i.health = "Monitoring resumed; reconciliation pending.".into();
            if i.locked {
                i.queue.push(Work {
                    ticket: None,
                    job: Job::Bootstrap,
                    manual: false,
                    foreground: false,
                });
            } else {
                Self::store_operation(&mut i, |s| s.set_due("discovery", 0))?;
            }
        }
        let revision = i.store.revision()?;
        drop(i);
        (self.on_revision)(revision);
        self.notify.notify_one();
        Ok(())
    }
    pub fn set_theme(&self, theme: Theme) -> Result<()> {
        if !theme.valid() {
            return Err(Error::Safe(
                "Theme requires four valid hex colors and a light/dark preset.".into(),
            ));
        }
        let mut i = self.lock()?;
        let mut s = i.store.settings()?;
        s.theme = theme;
        Self::store_operation(&mut i, |store| store.save_settings(&s))?;
        let revision = i.store.revision()?;
        drop(i);
        (self.on_revision)(revision);
        Ok(())
    }
    pub fn clear(&self) -> Result<()> {
        let mut i = self.lock()?;
        i.current.cancel();
        i.queue.jobs.clear();
        let result = Self::store_operation(&mut i, |s| s.clear());
        // A failed later batch can follow a committed logical clear. Publish
        // that revision too, so an open reader discards its previous payloads.
        let revision = i.store.revision();
        drop(i);
        if let Ok(revision) = revision {
            (self.on_revision)(revision);
        }
        result
    }
    pub fn configure(&self, path: String, config: String) -> Result<()> {
        let transport = Transport::discover(Some(&path), Some(&config))?;
        let mut i = self.lock()?;
        i.current.cancel();
        Self::store_operation(&mut i, |s| s.invalidate())?;
        i.queue.jobs.clear();
        i.locked = true;
        let mut settings = i.store.settings()?;
        settings.gh_path = Some(path);
        settings.config_dir = Some(config);
        Self::store_operation(&mut i, |s| s.save_settings(&settings))?;
        i.transport = Some(transport);
        i.health = "Configuration changed; validating the original identity…".into();
        i.queue.push(Work {
            ticket: None,
            job: Job::Bootstrap,
            manual: true,
            foreground: true,
        });
        drop(i);
        self.notify.notify_one();
        Ok(())
    }
    pub async fn shutdown(&self) {
        self.stop.cancel();
        if let Ok(i) = self.lock() {
            i.current.cancel();
        }
        self.notify.notify_one();
        self.finished.cancelled().await;
    }
    async fn run(&self) {
        loop {
            if self.stop.is_cancelled() {
                break;
            }
            let work = self.next();
            match work {
                Ok(Some((work, transport, token, generation))) => {
                    let mut result = self.execute(&work, &transport, &token, generation).await;
                    if let Ok(mut i) = self.lock() {
                        i.busy = false;
                        if result.is_ok() {
                            if let Some(seq) = work.ticket {
                                result = i.store.ack(seq);
                            }
                            if work.manual
                                && matches!(work.job, Job::Heads(_) | Job::Contexts(_))
                                && result.is_ok()
                            {
                                result = self.schedule_heads_locked(&mut i, true);
                            }
                        }
                        if let Err(err) = result {
                            i.health = err.to_string();
                            if Self::storage_error(&err) {
                                Self::stop_storage(&mut i, &err);
                                let revision = i.store.revision().unwrap_or(0);
                                drop(i);
                                (self.on_revision)(revision);
                                continue;
                            }
                            let recovery_result = (|| -> Result<()> {
                                if !token.is_cancelled() {
                                    i.store.error(&work.job.key(), &err.to_string())?;
                                    let at = now() + 60;
                                    let attempted_context_size = i.context_size;
                                    if err.to_string().contains("output limit")
                                        && matches!(work.job, Job::Heads(_))
                                    {
                                        if let Job::Heads(targets) = &work.job {
                                            if targets.len() > 1 {
                                                i.batch_size = (targets.len() / 2).max(1);
                                            } else if i.context_size > 1 {
                                                i.context_size = (i.context_size / 2).max(1);
                                            }
                                        }
                                        // Resize every reserved head batch after learning a payload bound.
                                        // Membership/due state stays on disk and is re-expanded at the smaller size.
                                        crate::control_sweep::run(
                                            &mut i.store.conn,
                                            "heads",
                                            serde_json::json!({}),
                                        )?;
                                        i.queue.jobs.retain(|w| !matches!(w.job, Job::Heads(_)));
                                    } else if err.to_string().contains("output limit")
                                        && matches!(&work.job,Job::Contexts(t) if t.page_size>1)
                                    {
                                        if let Job::Contexts(t) = &work.job {
                                            let mut next = t.clone();
                                            next.page_size = (next.page_size / 2).max(1);
                                            i.store.enqueue(
                                                &work.job.key(),
                                                &Job::Contexts(next),
                                                work.manual,
                                                false,
                                                0,
                                                true,
                                            )?;
                                        }
                                    } else if let Job::Activity(job) = &work.job {
                                        let mut next = job.clone();
                                        let oversized = err.to_string().contains("output limit")
                                            || err.to_string().contains("IPC limit");
                                        let mut limited = false;
                                        match &mut next {
                                            crate::activity::ActivityJob::Hydrate {
                                                repo,
                                                number,
                                                source,
                                                page,
                                                size,
                                            } => {
                                                crate::control_sweep::run(
                                                    &mut i.store.conn,
                                                    "activity",
                                                    serde_json::json!({"repo":repo,"number":number}),
                                                )?;
                                                if oversized {
                                                    if *source == 0 || *size == 1 {
                                                        limited = true;
                                                    } else {
                                                        *size = (*size / 2).max(1);
                                                    }
                                                }
                                                *source = 0;
                                                *page = 1;
                                            }
                                            crate::activity::ActivityJob::Notifications {
                                                size,
                                                ..
                                            } => {
                                                if oversized {
                                                    if *size == 1 {
                                                        limited = true;
                                                    } else {
                                                        *size = (*size / 2).max(1);
                                                    }
                                                }
                                            }
                                        }
                                        if let Some(seq) = work.ticket {
                                            i.store.ack(seq)?;
                                        }
                                        let retry =
                                            if limited { (i64::MAX / 2) as u64 } else { at };
                                        i.store.enqueue(
                                            &next.key(),
                                            &Job::Activity(next.clone()),
                                            work.manual,
                                            false,
                                            retry,
                                            true,
                                        )?;
                                    } else if let Job::Contexts(target) = &work.job {
                                        if let Some(seq) = work.ticket {
                                            i.store.ack(seq)?;
                                        }
                                        i.store.conn.execute("UPDATE check_state SET error=? WHERE pr_id=? AND commit_sha=?",rusqlite::params![err.to_string(),target.watch.id,target.commit])?;
                                        let retry = if err.to_string().contains("output limit") {
                                            (i64::MAX / 2) as u64
                                        } else {
                                            now()
                                        };
                                        i.store.postpone_watch(
                                            &target.watch.id,
                                            retry,
                                            &err.to_string(),
                                        )?;
                                    } else if let Job::Discover(p) = &work.job {
                                        let mut restart = Discovery::new(&p.source, &p.qualifier);
                                        if err.to_string().contains("output limit") {
                                            if p.page_size > 1 && p.source != "teams" {
                                                restart.page_size = (p.page_size / 2).max(1);
                                            } else {
                                                i.store
                                                    .set_due(&format!("limited:{}", p.source), 1)?;
                                                if let Some(seq) = work.ticket {
                                                    i.store.ack(seq)?;
                                                }
                                                return Ok(());
                                            }
                                        }
                                        i.store.enqueue(
                                            &format!("discover:{}", p.source),
                                            &Job::Discover(restart),
                                            work.manual,
                                            false,
                                            at,
                                            true,
                                        )?;
                                    } else if let Some(seq) = work.ticket {
                                        i.store.retry_task(seq, at)?;
                                    }
                                    if let Job::Heads(targets) = &work.job {
                                        for target in targets {
                                            if matches!(err, Error::Denied) {
                                                i.store.purge_watch(target)?;
                                            } else {
                                                let retry =
                                                    if err.to_string().contains("output limit") {
                                                        if targets.len() == 1
                                                            && attempted_context_size == 1
                                                        {
                                                            (i64::MAX / 2) as u64
                                                        } else {
                                                            now() + 1
                                                        }
                                                    } else {
                                                        at
                                                    };
                                                i.store.postpone_watch(
                                                    &target.id,
                                                    retry,
                                                    &err.to_string(),
                                                )?;
                                            }
                                        }
                                    }
                                    if matches!(err, Error::Io(_))
                                        || (err.to_string().contains("gh exceeded")
                                            && !err.to_string().contains("output limit"))
                                        || err.to_string().contains("response headers unavailable")
                                        || err.to_string().contains("HTTP 5")
                                    {
                                        i.store
                                            .record_failure(Self::job_resource(&work.job), now())?;
                                        if Self::job_resource(&work.job) != "core" {
                                            i.store.record_failure("core", now())?;
                                        }
                                        i.store.activity_offline()?;
                                    }
                                }
                                Ok(())
                            })();
                            if let Err(err) = recovery_result {
                                if Self::storage_error(&err) {
                                    Self::stop_storage(&mut i, &err);
                                } else {
                                    i.health = err.to_string();
                                }
                            }
                        }
                        if !i.storage_stopped
                            && work.manual
                            && matches!(work.job, Job::Heads(_))
                            && let Err(err) = self.schedule_heads_locked(&mut i, true)
                        {
                            if Self::storage_error(&err) {
                                Self::stop_storage(&mut i, &err);
                            } else {
                                i.health = err.to_string();
                            }
                        }
                        let revision = i.store.revision().unwrap_or(0);
                        drop(i);
                        (self.on_revision)(revision);
                    }
                }
                Err(err) => {
                    if let Ok(mut i) = self.lock() {
                        i.health = err.to_string();
                        if Self::storage_error(&err) {
                            Self::stop_storage(&mut i, &err);
                        }
                    }
                    tokio::select! {_=self.stop.cancelled()=>break,_=tokio::time::sleep(Duration::from_secs(1))=>{}}
                }
                Ok(None) => {
                    tokio::select! {_=self.stop.cancelled()=>break,_=self.notify.notified()=>{},_=tokio::time::sleep(Duration::from_secs(1))=>{}}
                }
            }
        }
        if let Ok(i) = self.lock() {
            let _ = i.store.checkpoint();
        }
        self.finished.cancel();
    }
    pub(crate) fn protect_storage<T>(&self, result: Result<T>) -> Result<T> {
        if let Err(err) = &result
            && Self::storage_error(err)
            && let Ok(mut i) = self.lock()
        {
            Self::stop_storage(&mut i, err);
        }
        result
    }
    pub(crate) fn cached_operation<T>(
        i: &mut Inner,
        operation: impl Fn(&mut Store) -> Result<T>,
    ) -> Result<T> {
        match operation(&mut i.store) {
            Err(err) if Self::storage_error(&err) => {
                Self::stop_storage(i, &err);
                // Retry only the cache read with access-time writes disabled.
                // Actual read errors still propagate; no fabricated empty result.
                operation(&mut i.store)
            }
            result => result,
        }
    }
    pub(crate) fn store_operation<T>(
        i: &mut Inner,
        operation: impl FnOnce(&mut Store) -> Result<T>,
    ) -> Result<T> {
        let result = operation(&mut i.store);
        if let Err(err) = &result
            && Self::storage_error(err)
        {
            Self::stop_storage(i, err);
        }
        result
    }
    pub(crate) fn require_remote_storage(i: &Inner) -> Result<()> {
        if i.storage_stopped || crate::control_sweep::pending(&i.store.conn)? {
            return Err(Error::Safe(
                "Storage protection prevents remote reads; repair is checked each minute.".into(),
            ));
        }
        Ok(())
    }
    fn storage_error(err: &Error) -> bool {
        matches!(err, Error::Storage(_) | Error::StorageUnavailable(_))
    }
    fn stop_storage(i: &mut Inner, err: &Error) {
        i.storage_stopped = true;
        i.store.cache_writes = false;
        i.storage_check_at = now() + 60;
        i.current.cancel();
        i.health = format!(
            "{err} Remote reads are stopped; cached reading remains available; local changes require a successful write. Storage will be checked again in one minute."
        );
    }
    fn check_storage(i: &mut Inner) -> Result<bool> {
        if i.storage_stopped && now() < i.storage_check_at {
            return Ok(false);
        }
        // Check free space before scheduling (which itself writes). Maintenance is
        // serialized with ingestion, without an unbounded VACUUM or startup rewrite.
        (i.storage_check)(i.path.parent().unwrap())?;
        if i.storage_stopped || now() >= i.storage_check_at {
            let repair_selection = crate::publication::pending(&i.store.conn)?;
            let repair_heads: bool = i.store.conn.query_row(
                "SELECT EXISTS(SELECT 1 FROM control_sweep WHERE kind='heads')",
                [],
                |r| r.get(0),
            )?;
            if i.startup_deferred {
                // Only replace the read-only connection after startup cleanup,
                // recovery writes and a real committed/checkpointed probe succeed.
                let mut writable = Store::open(&i.path)?;
                writable.activity_recovery(now())?;
                writable.set_due("discovery", 0)?;
                writable.maintain_retention()?;
                writable.storage_probe()?;
                i.store = writable;
                i.startup_deferred = false;
            } else {
                crate::publication::resume(&mut i.store.conn)?;
                crate::pr_purge::resume(&mut i.store.conn)?;
                crate::check_purge::resume(&mut i.store.conn)?;
                crate::clear_content::resume(&mut i.store.conn)?;
                crate::invalidation::resume(&mut i.store.conn)?;
                crate::control_sweep::resume(&mut i.store.conn)?;
                i.store.maintain_retention()?;
                i.store.storage_probe()?;
            }
            if repair_heads {
                i.queue.jobs.retain(|w| !matches!(w.job, Job::Heads(_)));
            }
            if repair_selection {
                let selected = i.selected.clone();
                let open = i.reader_open;
                i.store.remember_selection(selected.as_ref(), open)?;
            }
            if i.storage_stopped {
                i.store.activity_recovery(now())?;
                i.store.set_due("discovery", 0)?;
                i.queue.jobs.retain(|w| !matches!(w.job, Job::Activity(_)));
                i.health =
                    "Storage recovered; reconciling activity before completing catch-up.".into();
            }
            i.store.cache_writes = true;
            i.storage_stopped = false;
            i.storage_check_at = now() + 60;
        }
        Ok(true)
    }
    fn next(&self) -> Result<Option<(Work, Transport, CancellationToken, u64)>> {
        let mut i = self.lock()?;
        if !Self::check_storage(&mut i)? {
            return Ok(None);
        }
        let paused = i.store.settings()?.paused;
        if !paused && now().saturating_sub(i.last_tick) > 45 {
            i.store.activity_recovery(now())?;
            i.queue.jobs.retain(|w| !matches!(w.job, Job::Activity(_)));
            i.store.set_due("discovery", 0)?;
        }
        i.last_tick = now();
        if i.transport.is_none() {
            return Ok(None);
        }
        if now() < i.store.due("remote")? {
            return Ok(None);
        }
        if !paused && i.locked {
            i.queue.push(Work {
                ticket: None,
                job: Job::Bootstrap,
                manual: false,
                foreground: false,
            });
        }
        if !paused && !i.locked {
            let offline: bool = i.store.conn.query_row(
                "SELECT EXISTS(SELECT 1 FROM activity_cycle WHERE mode IN ('offline','recovery_offline'))",
                [],
                |r| r.get(0),
            )?;
            if offline && i.store.budget_eligible("core", now())? {
                i.store.activity_recovery(now())?;
                i.queue.jobs.retain(|w| !matches!(w.job, Job::Activity(_)));
                i.store.set_due("discovery", 0)?;
            }
            if i.store.due("discovery")? <= now() {
                drop(i);
                self.request_discovery(false)?;
                i = self.lock()?;
            }
            self.schedule_heads_locked(&mut i, false)?;
            self.schedule_activity(&mut i, false)?;
        }
        while let Some(work) = i.queue.jobs.pop_front() {
            if work.ticket.is_none() {
                i.store.enqueue(
                    &format!("legacy:{}", work.job.coalesce_key()),
                    &work.job,
                    work.manual,
                    work.foreground,
                    0,
                    false,
                )?;
            }
        }
        // Materialize at most 128 runnable descriptors; the remainder stay on disk.
        for (seq, _key, job, manual, foreground) in i.store.pending(now())? {
            if paused && !manual {
                continue;
            }
            let job: Job = serde_json::from_str(&job)?;
            if i.locked && !matches!(job, Job::Bootstrap) {
                continue;
            }
            if !Self::job_eligible(&i.store, &job, now())? {
                continue;
            }
            i.queue.push(Work {
                job,
                manual,
                foreground,
                ticket: Some(seq),
            });
        }
        // Bootstrap/explicit detail jobs also use the persisted queue.
        let Some(work) = i.queue.pop() else {
            return Ok(None);
        };
        if paused && !work.manual {
            return Ok(None);
        }
        (i.storage_check)(i.path.parent().unwrap())?;
        i.busy = true;
        i.current = CancellationToken::new();
        Ok(Some((
            work,
            i.transport.clone().unwrap(),
            i.current.clone(),
            i.store.generation()?,
        )))
    }
    pub(crate) async fn response(
        &self,
        t: &Transport,
        op: ReadOp,
        c: &CancellationToken,
    ) -> Result<Response> {
        let notification_url = match &op {
            ReadOp::Activity(read) if read.url.starts_with("notifications?") => {
                Some(read.url.clone())
            }
            _ => None,
        };
        let resource = match &op {
            ReadOp::Graph(_) => "graphql",
            ReadOp::Search { .. } | ReadOp::Authored { .. } => "search",
            _ => "core",
        };
        {
            let i = self.lock()?;
            if i.storage_stopped {
                return Err(Error::StorageUnavailable(i.health.clone()));
            }
            (i.storage_check)(i.path.parent().unwrap())?;
            if !i.store.budget_eligible(resource, now())? {
                return Err(Error::Safe(format!(
                    "Waiting for the GitHub {resource} budget/backoff deadline."
                )));
            }
        }
        let output = t.read(op, c).await?;
        if transport::indicates_auth_loss(&output) {
            let mut i = self.lock()?;
            i.locked = true;
            i.store.invalidate()?;
            i.queue.jobs.clear();
            return Err(Error::Safe(
                "Stored gh authentication was lost. Restore the original account externally."
                    .into(),
            ));
        }
        let payload = output.stdout.len();
        let r = Response::parse(output)?;
        let mut i = self.lock()?;
        i.store.record_response(resource, &r, payload, now())?;
        if let Some(url) = notification_url {
            let interval = r
                .headers
                .get("x-poll-interval")
                .and_then(|s| s.parse::<u64>().ok())
                .unwrap_or(60)
                .max(1);
            let deadline = now().saturating_add(interval).min(i64::MAX as u64);
            let next = deadline.max(i.store.due("activity:notifications")?);
            i.store.set_due("activity:notifications", next)?;
            i.store.conn.execute("INSERT INTO api_validator(url,next_at) VALUES(?,?) ON CONFLICT(url) DO UPDATE SET next_at=excluded.next_at",rusqlite::params![url,deadline as i64])?;
        }
        if r.status == 401 {
            i.locked = true;
            i.store.invalidate()?;
            i.queue.jobs.clear();
            i.health =
                "Stored gh authentication was lost. Restore the original account externally."
                    .into();
        }
        Ok(r)
    }
    pub(crate) async fn identity(
        &self,
        t: &Transport,
        c: &CancellationToken,
        generation: u64,
    ) -> Result<Identity> {
        let response = self.response(t, ReadOp::Identity, c).await?;
        let id = github::parse_identity(&response)?;
        let mut i = self.lock()?;
        if c.is_cancelled() || i.store.generation()? != generation {
            return Err(Error::Safe("Obsolete identity result discarded.".into()));
        }
        if let Err(err) = i.store.bind(&id) {
            i.locked = true;
            i.store.invalidate()?;
            i.queue.jobs.clear();
            i.health = err.to_string();
            return Err(err);
        }
        Ok(id)
    }
    async fn execute(
        &self,
        w: &Work,
        t: &Transport,
        c: &CancellationToken,
        generation: u64,
    ) -> Result<()> {
        if let Job::Activity(job) = &w.job {
            return self.execute_activity(w, t, c, generation, job).await;
        }
        if matches!(w.job, Job::Discover(_) | Job::Heads(_) | Job::Contexts(_)) {
            return self.execute_sync(w, t, c, generation).await;
        }
        if w.job == Job::Bootstrap {
            let version = t.read(ReadOp::Version, c).await?;
            if !version.success {
                return Err(Error::Safe("Unsupported gh executable/version.".into()));
            }
            let version = String::from_utf8(version.stdout)
                .map_err(|_| Error::Safe("Unsupported gh version output.".into()))?;
            if !version.starts_with("gh version ") {
                return Err(Error::Safe(
                    "The selected executable did not identify itself as gh.".into(),
                ));
            }
            let login = github::parse_auth(t.read(ReadOp::AuthStatus, c).await?)?;
            let id = self.identity(t, c, generation).await?;
            if login != id.login {
                return Err(Error::Safe(
                    "gh identity changed during validation; no content was accepted.".into(),
                ));
            }
            let mut i = self.lock()?;
            if i.store.generation()? != generation || c.is_cancelled() {
                return Ok(());
            }
            i.locked = false;
            i.version = Some(version.lines().next().unwrap_or_default().into());
            i.health =
                "Identity validated. Reconciling authored and review-request watches…".into();
            drop(i);
            self.request_discovery(w.manual)?;
            return Ok(());
        }
        let id = self.identity(t, c, generation).await?;
        if matches!(&w.job,Job::Authored(page) if *page>1)
            && crate::pr_purge::source_restart(&self.lock()?.store.conn, "authored")?
        {
            self.lock()?.queue.push(Work {
                ticket: None,
                job: Job::Authored(1),
                manual: w.manual,
                foreground: false,
            });
            return Ok(());
        }
        let op = match &w.job {
            Job::Authored(page) => ReadOp::Authored {
                login: id.login,
                page: *page,
            },
            Job::Overview(repo, number) => ReadOp::Overview {
                repo: repo.clone(),
                number: *number,
            },
            Job::Stacks(repo, number, _) => ReadOp::Stacks {
                repo: repo.clone(),
                number: *number,
            },
            Job::StackDetail(repo, _, _, number) => ReadOp::StackDetail {
                repo: repo.clone(),
                number: *number,
            },
            Job::Activity(_)
            | Job::Bootstrap
            | Job::Discover(_)
            | Job::Heads(_)
            | Job::Contexts(_) => unreachable!(),
        };
        // Every remote read is surrounded by stable-ID checks; external gh changes still have an unavoidable race window.
        let response = self.response(t, op, c).await?;
        if response.next_deadline(now()) > now() {
            return Err(Error::Safe(
                "GitHub quota/backoff deadline reached. Refresh remains queued until eligible."
                    .into(),
            ));
        }
        let after = self.identity(t, c, generation).await?;
        if id.id != after.id {
            return Err(Error::Safe(
                "Identity changed during read; result discarded.".into(),
            ));
        }
        let mut i = self.lock()?;
        if c.is_cancelled() || i.store.generation()? != generation || i.locked {
            return Ok(());
        }
        match &w.job {
            Job::Authored(page) => {
                let p = github::parse_search(&response)?;
                if *page == 1 {
                    i.store.start_authored()?;
                    i.store.set_due("authored", now() + 60)?;
                }
                if p.incomplete || p.total > 1000 {
                    return Err(Error::Safe("GitHub Search returned an incomplete or source-limited membership set; previous successful content and watch reasons are retained.".into()));
                }
                i.store.stage_authored(&p.prs, generation)?;
                if u64::from(*page) * 100 < p.total {
                    if p.prs.is_empty() {
                        return Err(Error::Safe("Membership traversal changed while paging; reconciliation is required.".into()));
                    }
                    i.queue.push(Work {
                        ticket: None,
                        job: Job::Authored(*page + 1),
                        manual: w.manual,
                        foreground: false,
                    });
                } else {
                    i.store.finish_authored(generation)?;
                    i.health="Authored PR list refreshed. Activity and review-request monitoring are not implemented yet.".into();
                }
            }
            Job::Overview(repo, number) => {
                let key = w.job.key();
                if response.status == 404
                    || (response.status == 403 && response.next_deadline(now()) == 0)
                {
                    i.store.activity_purge(repo, *number)?;
                    return Err(Error::Safe("Pull request unavailable; cached content purged without inferring deletion.".into()));
                }
                let data = github::parse_overview(&response, repo, *number)?;
                let head = data.head.clone();
                let selected =
                    i.reader_open && i.selected.as_ref() == Some(&(repo.clone(), *number));
                i.store.register_selected(&data, selected)?;
                i.store.put(&key, &data, generation)?;
                i.store.set_due(&key, now() + 10)?;
                i.queue.push(Work {
                    ticket: None,
                    job: Job::Stacks(repo.clone(), *number, head),
                    manual: w.manual,
                    foreground: false,
                });
            }
            Job::Stacks(repo, number, head) => {
                let key = w.job.key();
                let snapshot = i.store.get::<Overview>(&key)?;
                if let Some(mut pr) = snapshot.data
                    && pr.head == *head
                {
                    match github::stack_number(&response) {
                        Ok(Some(stack)) => {
                            i.queue.push(Work {
                                ticket: None,
                                job: Job::StackDetail(repo.clone(), *number, head.clone(), stack),
                                manual: w.manual,
                                foreground: false,
                            });
                        }
                        Ok(None) => {
                            pr.stack = StackState::None;
                            i.store.put(&key, &pr, generation)?;
                        }
                        Err(err) => {
                            pr.stack = StackState::Unknown {
                                reason: err.to_string(),
                            };
                            i.store.put(&key, &pr, generation)?;
                        }
                    }
                }
            }
            Job::StackDetail(repo, number, head, _) => {
                let key = w.job.key();
                let snapshot = i.store.get::<Overview>(&key)?;
                if let Some(mut pr) = snapshot.data
                    && pr.head == *head
                {
                    pr.stack =
                        github::parse_stack(&response, repo, *number).unwrap_or_else(|err| {
                            StackState::Unknown {
                                reason: err.to_string(),
                            }
                        });
                    i.store.put(&key, &pr, generation)?;
                }
            }
            Job::Activity(_)
            | Job::Bootstrap
            | Job::Discover(_)
            | Job::Heads(_)
            | Job::Contexts(_) => {}
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    // Each fixture owns an independent app and real gh subprocesses. Keep app
    // fixtures isolated so parallel libtest load does not consume their existing
    // state deadlines; concurrency within each runtime remains exercised.
    static WORKER_FIXTURES: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    async fn wait_for(mut condition: impl FnMut() -> bool) {
        tokio::time::timeout(Duration::from_secs(25), async {
            while !condition() {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("expected runtime state");
    }
    #[tokio::test]
    async fn write_fault_stops_children_and_recovery_requires_a_committed_probe() {
        let _worker_fixture = WORKER_FIXTURES.lock().await;
        use std::os::unix::fs::PermissionsExt;
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("db");
        let executable = d.path().join("gh");
        std::fs::write(&executable, include_str!("../tests/support/gh.py")).unwrap();
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
        let mut store = Store::open(&path).unwrap();
        let mut settings = store.settings().unwrap();
        settings.gh_path = Some(executable.display().to_string());
        settings.config_dir = Some(d.path().join("config").display().to_string());
        store.save_settings(&settings).unwrap();
        drop(store);
        let r = Runtime::start(path.clone(), |_| {}).unwrap();
        wait_for(|| r.activity(0).is_ok_and(|a| a.baseline)).await;
        let sql = rusqlite::Connection::open(path).unwrap();
        sql.execute_batch("CREATE TRIGGER fail_queue BEFORE INSERT ON pending BEGIN SELECT RAISE(ABORT,'fixture storage write failure'); END;").unwrap();
        {
            let i = r.lock().unwrap();
            i.store.set_due("discovery", 0).unwrap();
        }
        wait_for(|| r.status().unwrap().storage_stopped).await;
        let reads = || {
            std::fs::read_to_string(d.path().join("reads"))
                .unwrap_or_default()
                .lines()
                .count()
        };
        let stopped_count = reads();
        assert!(r.page(0).unwrap().complete, "cached reading survives");
        r.set_pause(true).unwrap();
        r.set_theme(Theme::default()).unwrap();
        r.refresh().unwrap_err(); // queue writes still fail; permission to read cannot bypass protection
        tokio::time::sleep(Duration::from_millis(1200)).await;
        assert_eq!(reads(), stopped_count);
        // Failed health probes keep the stop, even when other writes work.
        sql.execute_batch("CREATE TRIGGER fail_probe BEFORE UPDATE ON meta BEGIN SELECT RAISE(ABORT,'fixture probe failure'); END; DROP TRIGGER fail_queue;").unwrap();
        r.lock().unwrap().storage_check_at = 0;
        tokio::time::sleep(Duration::from_millis(1200)).await;
        assert!(r.status().unwrap().storage_stopped);
        assert_eq!(reads(), stopped_count);
        sql.execute_batch("DROP TRIGGER fail_probe").unwrap();
        r.lock().unwrap().storage_check_at = 0;
        wait_for(|| !r.status().unwrap().storage_stopped).await;
        assert!(r.status().unwrap().settings.paused);
        assert!(r.activity(0).unwrap().recovery.is_some());
        assert_eq!(
            reads(),
            stopped_count,
            "repair never resumes a persistent Pause"
        );
        // A partial PASSIVE checkpoint must enter the same protection path.
        sql.execute_batch("BEGIN; SELECT * FROM meta;").unwrap();
        r.set_theme(Theme::default()).unwrap();
        r.lock().unwrap().storage_check_at = 0;
        wait_for(|| r.status().unwrap().storage_stopped).await;
        assert_eq!(reads(), stopped_count);
        sql.execute_batch("ROLLBACK").unwrap();
        r.lock().unwrap().storage_check_at = 0;
        wait_for(|| !r.status().unwrap().storage_stopped).await;
        // A local settings failure also stops the monitor immediately.
        sql.execute_batch("CREATE TRIGGER fail_settings BEFORE INSERT ON settings BEGIN SELECT RAISE(ABORT,'fixture settings failure'); END;").unwrap();
        assert!(r.set_theme(Theme::default()).is_err());
        assert!(r.status().unwrap().storage_stopped);
        assert_eq!(reads(), stopped_count);
        sql.execute_batch("DROP TRIGGER fail_settings").unwrap();
        r.shutdown().await;
    }
    #[tokio::test]
    async fn control_sweeps_protect_startup_and_resume_before_remote_eligibility() {
        let _worker_fixture = WORKER_FIXTURES.lock().await;
        for kind in ["heads", "activity", "manual"] {
            let (d, path) = fixture_store();
            let mut s = Store::open(&path).unwrap();
            s.bind(&Identity {
                id: 42,
                login: "reader".into(),
            })
            .unwrap();
            let mut settings = s.settings().unwrap();
            settings.paused = true;
            s.save_settings(&settings).unwrap();
            s.conn.execute_batch("INSERT INTO due VALUES('activity:notifications',4070908800); INSERT INTO activity VALUES('A','a/b',1,'comment',4070908800,1,'eligible',1); INSERT INTO seen VALUES('A',0); INSERT INTO dedup VALUES('A'); INSERT INTO activity_intent VALUES(1,'pr','attempted',1); INSERT INTO intent_member VALUES(1,'A'); INSERT INTO alert_attempt VALUES('attempt',1);").unwrap();
            for k in 0..257 {
                s.conn
                    .execute(
                        "INSERT INTO watch_locator(id,repo,number,due) VALUES(?,'a/b',?,999)",
                        rusqlite::params![format!("P{k:04}"), k],
                    )
                    .unwrap();
                s.conn
                    .execute(
                        "INSERT INTO watch_source VALUES(?,'direct')",
                        [format!("P{k:04}")],
                    )
                    .unwrap();
                s.conn
                    .execute(
                        "INSERT INTO activity_stage VALUES('a/b',1,?,'{}')",
                        [format!("S{k:04}")],
                    )
                    .unwrap();
                s.conn
                    .execute(
                        "INSERT INTO pending(key,job,manual,foreground,at) VALUES(?,?,0,0,0)",
                        rusqlite::params![format!("heads-{k:04}"), "{\"Heads\":[]}"],
                    )
                    .unwrap();
            }
            let (table, action, pred) = match kind {
                "heads" => ("pending", "DELETE", "OLD.key='heads-0200'"),
                "activity" => ("activity_stage", "DELETE", "OLD.id='S0200'"),
                _ => ("watch_locator", "UPDATE", "OLD.id='P0200'"),
            };
            s.conn.execute_batch(&format!("CREATE TRIGGER fail_control BEFORE {action} ON {table} WHEN {pred} BEGIN SELECT RAISE(ABORT,'late control sweep'); END;")).unwrap();
            assert!(
                crate::control_sweep::run(
                    &mut s.conn,
                    kind,
                    serde_json::json!({"repo":"a/b","number":1})
                )
                .is_err()
            );
            drop(s);
            let r = Runtime::start(path.clone(), |_| {}).unwrap();
            assert!(
                r.status().unwrap().locked
                    && r.status().unwrap().storage_stopped
                    && r.status().unwrap().settings.paused
            );
            assert!(r.refresh().is_err());
            assert_eq!(reads(d.path()), 0);
            r.lock().unwrap().storage_check_at = 0;
            wait_for(|| r.lock().unwrap().storage_check_at != 0).await;
            assert!(r.status().unwrap().storage_stopped);
            assert_eq!(reads(d.path()), 0);
            let sql = rusqlite::Connection::open(&path).unwrap();
            sql.execute_batch("DROP TRIGGER fail_control").unwrap();
            r.lock().unwrap().storage_check_at = 0;
            wait_for(|| !r.status().unwrap().storage_stopped).await;
            assert!(r.status().unwrap().locked && r.status().unwrap().settings.paused);
            assert_eq!(reads(d.path()), 0);
            assert!(!crate::control_sweep::pending(&sql).unwrap());
            assert_eq!(r.lock().unwrap().store.identity().unwrap().unwrap().id, 42);
            for query in [
                "SELECT COUNT(*) FROM seen WHERE source_id='A' AND seen=0",
                "SELECT COUNT(*) FROM dedup WHERE source_id='A'",
                "SELECT COUNT(*) FROM activity_intent WHERE state='attempted'",
                "SELECT COUNT(*) FROM intent_member WHERE activity='A'",
                "SELECT COUNT(*) FROM alert_attempt",
            ] {
                assert_eq!(sql.query_row(query, [], |r| r.get::<_, i64>(0)).unwrap(), 1);
            }
            assert_eq!(
                r.lock()
                    .unwrap()
                    .store
                    .due("activity:notifications")
                    .unwrap(),
                4070908800
            );
            r.shutdown().await;
        }
    }

    #[tokio::test]
    async fn manual_control_fault_preserves_cached_navigation_and_paused_repair() {
        let _worker_fixture = WORKER_FIXTURES.lock().await;
        let (d, path) = fixture_store();
        let r = Runtime::start(path.clone(), |_| {}).unwrap();
        wait_for(|| r.activity(0).is_ok_and(|p| p.baseline)).await;
        r.set_pause(true).unwrap();
        wait_for(|| !r.status().unwrap().busy).await;
        let sql = rusqlite::Connection::open(&path).unwrap();
        sql.execute("UPDATE watch_locator SET due=0", []).unwrap();
        sql.execute("INSERT INTO due VALUES('remote',4070908800) ON CONFLICT(key) DO UPDATE SET at=excluded.at",[]).unwrap();
        for k in 0..257 {
            sql.execute(
                "INSERT INTO watch_locator(id,repo,number,due) VALUES(?,'fixture/repo',?,999)",
                rusqlite::params![format!("M{k:04}"), k],
            )
            .unwrap();
            sql.execute(
                "INSERT INTO watch_source VALUES(?,'direct')",
                [format!("M{k:04}")],
            )
            .unwrap();
        }
        let before = r.activity(0).unwrap();
        let read_count = reads(d.path());
        sql.execute_batch("CREATE TRIGGER fail_manual BEFORE UPDATE OF due ON watch_locator WHEN OLD.id='M0200' BEGIN SELECT RAISE(ABORT,'late manual reset'); END;").unwrap();
        assert!(r.refresh().is_err());
        assert!(r.status().unwrap().storage_stopped);
        assert_eq!(
            sql.query_row(
                "SELECT COUNT(*) FROM watch_locator WHERE id GLOB 'M*' AND due!=0",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            130
        );
        assert_eq!(r.activity(0).unwrap().items.len(), before.items.len());
        assert!(r.page(0).is_ok());
        assert_eq!(reads(d.path()), read_count);
        r.lock().unwrap().storage_check_at = 0;
        wait_for(|| r.lock().unwrap().storage_check_at != 0).await;
        assert!(r.status().unwrap().storage_stopped);
        assert!(r.refresh().is_err());
        assert_eq!(reads(d.path()), read_count);
        sql.execute_batch("DROP TRIGGER fail_manual").unwrap();
        r.lock().unwrap().storage_check_at = 0;
        wait_for(|| !r.status().unwrap().storage_stopped).await;
        assert!(r.status().unwrap().settings.paused);
        assert!(!crate::control_sweep::pending(&sql).unwrap());
        assert_eq!(reads(d.path()), read_count);
        assert_eq!(
            sql.query_row(
                "SELECT COUNT(*) FROM watch_locator WHERE id GLOB 'M*' AND due!=0",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            0
        );
        assert_eq!(r.activity(0).unwrap().items.len(), before.items.len());
        r.shutdown().await;
    }

    #[tokio::test]
    async fn query_cursor_checkpoint_fault_protects_cached_session_and_repairs_paused() {
        let _worker_fixture = WORKER_FIXTURES.lock().await;
        let (d, path) = fixture_store();
        let r = Runtime::start(path.clone(), |_| {}).unwrap();
        wait_for(|| r.activity(0).is_ok_and(|p| p.baseline)).await;
        r.set_pause(true).unwrap();
        wait_for(|| !r.status().unwrap().busy).await;
        let sql = rusqlite::Connection::open(&path).unwrap();
        sql.execute("INSERT INTO due VALUES('remote',4070908800) ON CONFLICT(key) DO UPDATE SET at=excluded.at",[]).unwrap();
        sql.execute_batch("UPDATE activity_cycle SET mode='recovery',boundary=123; WITH RECURSIVE x(n) AS(VALUES(0) UNION ALL SELECT n+1 FROM x WHERE n<255) INSERT INTO api_validator(url,next_at) SELECT printf('zz-query-%04d',n),4070908800 FROM x;").unwrap();
        r.lock().unwrap().store.maintain_retention().unwrap();
        let cursor: String = sql
            .query_row(
                "SELECT position FROM retention_cursor WHERE class='validators'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let reads_before = reads(d.path());
        let poll = r
            .lock()
            .unwrap()
            .store
            .due("activity:notifications")
            .unwrap();
        let page = r.page(0).unwrap();
        sql.execute_batch("CREATE TRIGGER fail_query_cursor BEFORE INSERT ON retention_cursor WHEN NEW.class='validators' BEGIN SELECT RAISE(ABORT,'cursor checkpoint fault'); END;").unwrap();
        r.lock().unwrap().storage_check_at = 0;
        wait_for(|| r.status().unwrap().storage_stopped).await;
        assert_eq!(reads(d.path()), reads_before);
        assert_eq!(r.page(0).unwrap().items.len(), page.items.len());
        assert!(r.activity(0).unwrap().baseline);
        assert!(r.refresh().is_err());
        assert_eq!(
            sql.query_row(
                "SELECT position FROM retention_cursor WHERE class='validators'",
                [],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
            cursor
        );
        r.lock().unwrap().storage_check_at = 0;
        wait_for(|| r.lock().unwrap().storage_check_at != 0).await;
        assert!(r.status().unwrap().storage_stopped);
        assert_eq!(reads(d.path()), reads_before);
        sql.execute_batch("DROP TRIGGER fail_query_cursor").unwrap();
        r.lock().unwrap().storage_check_at = 0;
        wait_for(|| !r.status().unwrap().storage_stopped).await;
        assert!(r.status().unwrap().settings.paused);
        assert_eq!(reads(d.path()), reads_before);
        assert_eq!(
            r.lock()
                .unwrap()
                .store
                .due("activity:notifications")
                .unwrap(),
            poll
        );
        assert_eq!(
            sql.query_row(
                "SELECT boundary FROM activity_cycle WHERE mode='recovery'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            123
        );
        assert_eq!(sql.query_row("SELECT COUNT(*) FROM api_validator WHERE url GLOB 'zz-query-*' AND next_at=4070908800",[],|r|r.get::<_,i64>(0)).unwrap(),256);
        assert_eq!(r.page(0).unwrap().items.len(), page.items.len());
        r.shutdown().await;
    }

    fn fixture_store() -> (tempfile::TempDir, PathBuf) {
        use std::os::unix::fs::PermissionsExt;
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("db");
        let executable = d.path().join("gh");
        std::fs::write(&executable, include_str!("../tests/support/gh.py")).unwrap();
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
        let mut store = Store::open(&path).unwrap();
        let mut settings = store.settings().unwrap();
        settings.gh_path = Some(executable.display().to_string());
        settings.config_dir = Some(d.path().join("config").display().to_string());
        store.save_settings(&settings).unwrap();
        (d, path)
    }
    fn reads(d: &std::path::Path) -> usize {
        std::fs::read_to_string(d.join("reads"))
            .unwrap_or_default()
            .lines()
            .count()
    }

    #[tokio::test]
    async fn full_ceiling_worker_publication_fault_stops_reads_and_repairs_while_paused() {
        let _worker_fixture = WORKER_FIXTURES.lock().await;
        use crate::content_budget::disposable_bytes;
        use crate::content_capacity::{CAPACITY, count, disk_backed, mixed_payloads, small_checks};
        use crate::store::CONTENT_LIMIT;
        use rusqlite::Connection;
        use std::{os::unix::fs::PermissionsExt, time::Duration};
        let _serial = CAPACITY.lock().await;
        async fn until(mut predicate: impl FnMut() -> bool) {
            tokio::time::timeout(Duration::from_secs(25), async {
                while !predicate() {
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
            })
            .await
            .unwrap();
        }
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("db");
        let executable = d.path().join("gh");
        std::fs::write(&executable, include_str!("../tests/support/gh.py")).unwrap();
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
        let mut s = Store::open(&path).unwrap();
        let mut settings = s.settings().unwrap();
        settings.gh_path = Some(executable.display().to_string());
        settings.config_dir = Some(d.path().join("config").display().to_string());
        s.save_settings(&settings).unwrap();
        drop(s);
        let r = Runtime::start(path.clone(), |_| {}).unwrap();
        until(|| r.activity(0).is_ok_and(|a| a.baseline)).await;
        r.set_pause(true).unwrap();
        until(|| !r.status().unwrap().busy).await;
        r.clear().unwrap();
        {
            let mut i = r.lock().unwrap();
            i.store.activity_recovery(123).unwrap();
            mixed_payloads(&mut i.store);
            // Make published checks the oldest victims for the actual selected
            // registration writer; no response-size or budget override is used.
            i.store
                .conn
                .execute(
                    "UPDATE content SET accessed=? WHERE key GLOB 'small-*'",
                    [now() as i64],
                )
                .unwrap();
            disk_backed(&i.store, &path);
            i.store.conn.execute_batch("CREATE TRIGGER fail_worker_capacity BEFORE DELETE ON check_context WHEN OLD.staged=0 BEGIN SELECT RAISE(ABORT,'fixture worker capacity fault'); END;").unwrap();
        }
        let sql = Connection::open(&path).unwrap();
        let ledgers = || {
            [
                "activity",
                "seen",
                "dedup",
                "activity_intent",
                "intent_member",
                "alert_attempt",
                "progress",
            ]
            .map(|table| {
                count(
                    &sql,
                    &format!(
                        "SELECT COUNT(*) FROM {table}{}",
                        if table == "progress" {
                            " WHERE key!='selection'"
                        } else {
                            ""
                        }
                    ),
                )
            })
        };
        let original = ledgers();
        let identity = r.status().unwrap().identity;
        let deadline = r.activity(0).unwrap().next_poll_at;
        let boundary = count(&sql, "SELECT boundary FROM activity_cycle");
        let revision = r.status().unwrap().revision;
        r.select("a/b".into(), 99).unwrap();
        until(|| r.status().unwrap().storage_stopped).await;
        assert_eq!(disposable_bytes(&sql).unwrap(), CONTENT_LIMIT);
        assert_eq!(small_checks(&sql), 64);
        assert_eq!(
            count(&sql, "SELECT complete FROM check_state WHERE pr_id='P'"),
            1
        );
        assert_eq!(
            count(&sql, "SELECT COUNT(*) FROM watch_locator WHERE number=99"),
            0
        );
        // Selecting commits its own remembered selection before hydration.
        assert_eq!(r.status().unwrap().revision, revision + 1);
        assert!(r.refresh().is_err());
        let reads = || {
            std::fs::read_to_string(d.path().join("reads"))
                .unwrap_or_default()
                .lines()
                .count()
        };
        let stopped = reads();
        // The interrupted manual job remains authorized and queued. Hold it
        // behind a fixture remote deadline while qualifying repair in isolation.
        sql.execute_batch("INSERT INTO due VALUES('remote',4070908800) ON CONFLICT(key) DO UPDATE SET at=excluded.at;").unwrap();
        sql.execute_batch("DROP TRIGGER fail_worker_capacity")
            .unwrap();
        r.lock().unwrap().storage_check_at = 0;
        until(|| !r.status().unwrap().storage_stopped).await;
        assert_eq!(reads(), stopped);
        assert!(r.status().unwrap().settings.paused);
        assert_eq!(r.status().unwrap().identity, identity);
        assert_eq!(r.activity(0).unwrap().next_poll_at, deadline);
        assert_eq!(count(&sql, "SELECT boundary FROM activity_cycle"), boundary);
        assert_eq!(ledgers(), original);
        // Repair recovery initialization discards abandoned Activity staging.
        assert!(disposable_bytes(&sql).unwrap() < CONTENT_LIMIT);
        sql.execute("UPDATE due SET at=0 WHERE key='remote'", [])
            .unwrap();
        r.select("a/b".into(), 99).unwrap();
        until(|| r.overview("a/b", 99).is_ok_and(|v| v.data.is_some())).await;
        assert!(r.status().unwrap().settings.paused);
        assert!(disposable_bytes(&sql).unwrap() <= CONTENT_LIMIT);
        assert_eq!(ledgers(), original);
        assert_eq!(
            count(
                &sql,
                "SELECT COUNT(*) FROM activity_intent WHERE state='pending'"
            ),
            0
        );
        r.shutdown().await;
    }

    #[tokio::test]
    async fn pr_purge_overview_fault_protects_paused_cache_and_repairs_without_neighbor_loss() {
        let _worker_fixture = WORKER_FIXTURES.lock().await;
        let (d, path) = fixture_store();
        let r = Runtime::start(path.clone(), |_| {}).unwrap();
        wait_for(|| r.activity(0).is_ok_and(|a| a.baseline)).await;
        r.select("a/b".into(), 1).unwrap();
        wait_for(|| r.overview("a/b", 1).is_ok_and(|v| v.data.is_some())).await;
        r.set_pause(true).unwrap();
        wait_for(|| !r.status().unwrap().busy).await;
        let sql = rusqlite::Connection::open(&path).unwrap();
        let n = |q: &str| sql.query_row(q, [], |r| r.get::<_, i64>(0)).unwrap();
        let ledgers = || {
            [
                "activity",
                "seen",
                "dedup",
                "activity_intent",
                "intent_member",
                "alert_attempt",
            ]
            .map(|t| n(&format!("SELECT COUNT(*) FROM {t}")))
        };
        let original = ledgers();
        let before = r.status().unwrap();
        let deadline = r.activity(0).unwrap().next_poll_at;
        let epoch = n("SELECT epoch FROM watch_locator WHERE id='PR_1'");
        let neighbor = n("SELECT COUNT(*) FROM check_context WHERE pr_id='PR_2'");
        assert!(neighbor > 0);
        for k in 0..257 {
            sql.execute(
                "INSERT INTO content VALUES(?,'{}',4070908800,4070908800)",
                [format!("api:repos/a/b/pulls/1/comments?fixture={k:03}")],
            )
            .unwrap();
        }
        sql.execute_batch("CREATE TRIGGER fail_pr_purge BEFORE DELETE ON content WHEN OLD.key='api:repos/a/b/pulls/1/comments?fixture=200' BEGIN SELECT RAISE(ABORT,'fixture PR purge'); END;").unwrap();
        std::fs::write(d.path().join("scenario.json"), r#"{"deny_overview":1}"#).unwrap();
        r.select("a/b".into(), 1).unwrap();
        wait_for(|| r.status().unwrap().storage_stopped).await;
        let remaining = n(
            "SELECT COUNT(*) FROM content WHERE key GLOB 'api:repos/a/b/pulls/1/comments?fixture=*'",
        );
        assert!(remaining > 0 && remaining < 257);
        assert!(!r.status().unwrap().locked);
        assert!(r.overview("a/b", 1).unwrap().data.is_none());
        assert!(r.checks("a/b", 1, 0).unwrap().items.is_empty());
        assert!(!r.checks("a/b", 2, 0).unwrap().items.is_empty());
        assert!(!r.page(0).unwrap().complete);
        assert!(r.refresh().is_err());
        assert_eq!(ledgers(), original);
        assert!(
            r.activity(0)
                .unwrap()
                .items
                .iter()
                .filter(|a| a.number == 1)
                .all(|a| a.text.is_none())
        );
        sql.execute("INSERT INTO due VALUES('remote',4070908800) ON CONFLICT(key) DO UPDATE SET at=excluded.at",[]).unwrap();
        let stopped = reads(d.path());
        r.lock().unwrap().storage_check_at = 0;
        wait_for(|| r.lock().unwrap().storage_check_at != 0).await;
        assert!(r.status().unwrap().storage_stopped);
        assert_eq!(reads(d.path()), stopped);
        sql.execute_batch("DROP TRIGGER fail_pr_purge").unwrap();
        r.lock().unwrap().storage_check_at = 0;
        wait_for(|| !r.status().unwrap().storage_stopped).await;
        assert_eq!(reads(d.path()), stopped);
        assert_eq!(r.status().unwrap().identity, before.identity);
        assert!(r.status().unwrap().settings.paused);
        assert_eq!(r.activity(0).unwrap().next_poll_at, deadline);
        assert_eq!(ledgers(), original);
        assert_eq!(
            n("SELECT epoch FROM watch_locator WHERE id='PR_1'"),
            epoch + 1
        );
        assert_eq!(
            n("SELECT COUNT(*) FROM check_context WHERE pr_id='PR_2'"),
            neighbor
        );
        assert!(!crate::pr_purge::any_pending(&r.lock().unwrap().store.conn).unwrap());
        assert!(r.overview("a/b", 1).unwrap().data.is_none());
        r.shutdown().await;
        drop(r);
        let r = Runtime::start(path, |_| {}).unwrap();
        assert!(r.status().unwrap().locked && r.status().unwrap().settings.paused);
        assert_eq!(reads(d.path()), stopped);
        assert_eq!(ledgers(), original);
        r.shutdown().await;
    }

    #[tokio::test]
    async fn interrupted_activity_and_summary_repair_before_remote_eligibility() {
        let _worker_fixture = WORKER_FIXTURES.lock().await;
        for kind in ["activity", "summary"] {
            let (d, path) = fixture_store();
            let mut s = crate::activity_bounded::fixture(&path, kind, "recovery");
            s.conn.execute_batch("CREATE TRIGGER fail_activity_repair BEFORE UPDATE OF applied ON publication_row WHEN NEW.applied=1 AND NEW.seq>(SELECT MIN(seq)+200 FROM publication_row) BEGIN SELECT RAISE(ABORT,'late Activity repair'); END;").unwrap();
            let old = serde_json::to_value(s.activity_page(0).unwrap()).unwrap();
            let result = if kind == "summary" {
                s.activity_finish_recovery(false)
            } else {
                s.activity_publish("a/b", 1, false, &crate::activity_crash::identity())
            };
            assert!(result.is_err());
            let mut settings = s.settings().unwrap();
            settings.paused = true;
            s.save_settings(&settings).unwrap();
            drop(s);
            let r = Runtime::start(path.clone(), |_| {}).unwrap();
            assert!(
                r.status().unwrap().locked
                    && r.status().unwrap().settings.paused
                    && r.status().unwrap().storage_stopped
            );
            assert_eq!(reads(d.path()), 0);
            assert!(r.refresh().is_err());
            {
                let mut i = r.lock().unwrap();
                let mut page = i.store.activity_page(0).unwrap();
                page.revision = old["revision"].as_u64().unwrap();
                assert_eq!(serde_json::to_value(page).unwrap(), old);
                i.storage_check_at = 0;
            }
            wait_for(|| r.lock().unwrap().storage_check_at != 0).await;
            assert!(r.status().unwrap().storage_stopped);
            assert_eq!(reads(d.path()), 0);
            let c = rusqlite::Connection::open(&path).unwrap();
            c.execute_batch("DROP TRIGGER fail_activity_repair")
                .unwrap();
            r.lock().unwrap().storage_check_at = 0;
            wait_for(|| !r.status().unwrap().storage_stopped).await;
            assert_eq!(reads(d.path()), 0);
            assert!(r.status().unwrap().settings.paused && r.status().unwrap().locked);
            assert!(!crate::publication::pending(&c).unwrap());
            assert_eq!(
                c.query_row(
                    "SELECT COUNT(*) FROM activity WHERE id LIKE 'N%'",
                    [],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
                if kind == "summary" { 513 } else { 257 }
            );
            if kind == "summary" {
                // Repair runs retention after completing publication. The expired
                // minimal record goes away; its seen/dedup replay barriers survive.
                assert_eq!(
                    c.query_row(
                        "SELECT COUNT(*) FROM activity WHERE id='N1-0001'",
                        [],
                        |r| r.get::<_, i64>(0)
                    )
                    .unwrap(),
                    0
                );
                assert_eq!(
                    c.query_row(
                        "SELECT COUNT(*) FROM dedup WHERE source_id='N1-0001'",
                        [],
                        |r| r.get::<_, i64>(0)
                    )
                    .unwrap(),
                    1
                );
                assert_eq!(
                    c.query_row(
                        "SELECT COUNT(*) FROM seen WHERE source_id='N1-0001'",
                        [],
                        |r| r.get::<_, i64>(0)
                    )
                    .unwrap(),
                    1
                );
            }
            assert_eq!(
                c.query_row(
                    "SELECT COUNT(*) FROM intent_member WHERE intent>2",
                    [],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
                if kind == "summary" { 512 } else { 1 }
            );
            assert_eq!(
                c.query_row(
                    "SELECT COUNT(*) FROM activity_intent WHERE state IN ('pending','recovery')",
                    [],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
                0
            );
            assert_eq!(
                c.query_row("SELECT state FROM activity_intent WHERE id=1", [], |r| {
                    r.get::<_, String>(0)
                })
                .unwrap(),
                "attempted"
            );
            assert_eq!(
                r.lock()
                    .unwrap()
                    .store
                    .due("activity:notifications")
                    .unwrap(),
                4070908800
            );
            r.shutdown().await;
        }
    }

    #[tokio::test]
    async fn partial_publication_startup_stays_locked_paused_and_repairs_before_reads() {
        let _worker_fixture = WORKER_FIXTURES.lock().await;
        let (d, path) = fixture_store();
        let (mut s, args) = crate::publication::tests::seed(&path, "head");
        s.conn
            .execute(
                "UPDATE activity SET occurred=?,created=?",
                rusqlite::params![now() as i64, now() as i64],
            )
            .unwrap();
        crate::publication::tests::start(&mut s, "head", &args);
        s.conn.execute_batch("CREATE TRIGGER fail_start_publication BEFORE UPDATE ON publication_row WHEN NEW.applied=1 BEGIN SELECT RAISE(ABORT,'fixture startup publication'); END;").unwrap();
        drop(s);
        let r = Runtime::start(path.clone(), |_| {}).unwrap();
        assert!(
            r.status().unwrap().locked
                && r.status().unwrap().settings.paused
                && r.status().unwrap().storage_stopped
        );
        assert_eq!(reads(d.path()), 0);
        assert!(r.refresh().is_err());
        r.lock().unwrap().storage_check_at = 0;
        wait_for(|| r.lock().unwrap().storage_check_at != 0).await;
        assert!(r.status().unwrap().storage_stopped);
        assert_eq!(reads(d.path()), 0);
        let sql = rusqlite::Connection::open(&path).unwrap();
        sql.execute_batch("DROP TRIGGER fail_start_publication")
            .unwrap();
        r.lock().unwrap().storage_check_at = 0;
        wait_for(|| !r.status().unwrap().storage_stopped).await;
        assert!(r.status().unwrap().locked && r.status().unwrap().settings.paused);
        assert_eq!(reads(d.path()), 0);
        assert!(!crate::publication::pending(&sql).unwrap());
        assert_eq!(
            sql.query_row("SELECT epoch FROM watch_locator WHERE id='P1'", [], |r| r
                .get::<_, i64>(
                0
            ))
            .unwrap(),
            1
        );
        assert_eq!(
            sql.query_row("SELECT seen FROM seen WHERE source_id='A'", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
        assert_eq!(
            sql.query_row(
                "SELECT COUNT(*) FROM activity_intent WHERE state IN ('attempted','unavailable')",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            2
        );
        assert!(r.activity(0).is_err());
        r.shutdown().await;
    }

    #[tokio::test]
    async fn publication_failure_protects_old_source_and_repair_exposes_one_complete_result() {
        let _worker_fixture = WORKER_FIXTURES.lock().await;
        let (d, path) = fixture_store();
        let r = Runtime::start(path.clone(), |_| {}).unwrap();
        wait_for(|| r.activity(0).is_ok_and(|a| a.baseline)).await;
        r.select("a/b".into(), 1).unwrap();
        wait_for(|| r.overview("a/b", 1).is_ok_and(|v| v.data.is_some())).await;
        r.set_pause(true).unwrap();
        wait_for(|| !r.status().unwrap().busy).await;
        let sql = rusqlite::Connection::open(&path).unwrap();
        let count = |q: &str| sql.query_row(q, [], |r| r.get::<_, i64>(0)).unwrap();
        let history = || {
            [
                "activity",
                "seen",
                "dedup",
                "activity_intent",
                "intent_member",
                "alert_attempt",
            ]
            .map(|t| count(&format!("SELECT COUNT(*) FROM {t}")))
        };
        let original = history();
        let old = r
            .page(0)
            .unwrap()
            .items
            .iter()
            .map(|p| p.id.clone())
            .collect::<Vec<_>>();
        let identity = r.status().unwrap().identity;
        let deadline = r.activity(0).unwrap().next_poll_at;
        sql.execute_batch("CREATE TRIGGER fail_publication BEFORE UPDATE ON publication_row WHEN NEW.applied=1 AND OLD.tbl='watch_locator' AND json_extract(OLD.k,'$[0]')='PR_200' BEGIN SELECT RAISE(ABORT,'fixture publication'); END;").unwrap();
        std::fs::write(d.path().join("scenario.json"), r#"{"authored":257}"#).unwrap();
        {
            let mut i = r.lock().unwrap();
            i.store
                .enqueue(
                    "discover:authored",
                    &Job::Discover(crate::sync::Discovery::new("authored", "")),
                    true,
                    false,
                    0,
                    true,
                )
                .unwrap();
        }
        r.notify_sync();
        wait_for(|| r.status().unwrap().storage_stopped).await;
        assert_eq!(
            r.page(0)
                .unwrap()
                .items
                .iter()
                .map(|p| p.id.clone())
                .collect::<Vec<_>>(),
            old
        );
        assert_eq!(history(), original);
        assert!(crate::publication::pending(&sql).unwrap());
        assert!(count("SELECT COUNT(*) FROM publication_row WHERE applied=1") > 128);
        assert!(r.reader_closed().is_err());
        assert!(r.refresh().is_err());
        let stopped = reads(d.path());
        r.lock().unwrap().storage_check_at = 0;
        wait_for(|| r.lock().unwrap().storage_check_at != 0).await;
        assert!(r.status().unwrap().storage_stopped);
        assert_eq!(reads(d.path()), stopped);
        sql.execute("INSERT INTO due VALUES('remote',4070908800) ON CONFLICT(key) DO UPDATE SET at=excluded.at",[]).unwrap();
        sql.execute_batch("DROP TRIGGER fail_publication").unwrap();
        r.lock().unwrap().storage_check_at = 0;
        wait_for(|| !r.status().unwrap().storage_stopped).await;
        assert_eq!(reads(d.path()), stopped);
        assert_eq!(history(), original);
        assert_eq!(r.status().unwrap().identity, identity);
        assert!(r.status().unwrap().settings.paused);
        assert_eq!(r.activity(0).unwrap().next_poll_at, deadline);
        assert_eq!(
            count("SELECT COUNT(*) FROM watch_source WHERE source='authored'"),
            257
        );
        assert!(!crate::publication::pending(&sql).unwrap());
        assert_eq!(
            count("SELECT COUNT(*) FROM watch_source WHERE source='selected'"),
            0
        );
        assert_eq!(r.status().unwrap().watch_count, 257);
        r.shutdown().await;
        drop(r);
        let r = Runtime::start(path, |_| {}).unwrap();
        assert!(r.status().unwrap().locked && r.status().unwrap().settings.paused);
        assert_eq!(reads(d.path()), stopped);
        r.shutdown().await;
    }

    #[tokio::test]
    async fn full_ceiling_bounded_admission_fault_protects_and_repairs_without_partial_registration()
     {
        let _worker_fixture = WORKER_FIXTURES.lock().await;
        use crate::content_budget::{
            admission_tests::{n, seed_full_with_rows},
            disposable_bytes,
        };
        let _serial = crate::content_capacity::CAPACITY.lock().await;
        let (d, path) = fixture_store();
        let r = Runtime::start(path.clone(), |_| {}).unwrap();
        wait_for(|| r.activity(0).is_ok_and(|a| a.baseline)).await;
        r.set_pause(true).unwrap();
        wait_for(|| !r.status().unwrap().busy).await;
        let sql = rusqlite::Connection::open(&path).unwrap();
        let ledgers = || {
            [
                "activity",
                "seen",
                "dedup",
                "activity_intent",
                "intent_member",
                "alert_attempt",
            ]
            .map(|t| n(&sql, &format!("SELECT COUNT(*) FROM {t}")))
        };
        let original = ledgers();
        let before = r.status().unwrap();
        let deadline = r.activity(0).unwrap().next_poll_at;
        {
            let mut i = r.lock().unwrap();
            seed_full_with_rows(&mut i.store, 2);
        }
        assert_eq!(disposable_bytes(&sql).unwrap(), crate::store::CONTENT_LIMIT);
        sql.execute_batch("CREATE TRIGGER fail_bounded_admission BEFORE DELETE ON content WHEN OLD.key='lru-0086' BEGIN SELECT RAISE(ABORT,'fixture bounded admission'); END;").unwrap();
        r.select("a/b".into(), 99).unwrap();
        wait_for(|| r.status().unwrap().storage_stopped).await;
        let left = n(
            &sql,
            "SELECT (SELECT COUNT(*) FROM content WHERE key GLOB 'lru-*')+(SELECT COUNT(*) FROM check_context WHERE pr_id GLOB 'lru-*')",
        );
        assert!(
            left < 513 && left > 400,
            "first bounded batch committed: {left}"
        );
        assert_eq!(
            n(&sql, "SELECT COUNT(*) FROM watch_locator WHERE number=99"),
            0
        );
        assert_eq!(
            n(
                &sql,
                "SELECT COUNT(*) FROM content WHERE key='pr:a/b:99' OR key='list:PR_99'"
            ),
            0
        );
        assert_eq!(r.status().unwrap().watch_count, before.watch_count);
        assert!(r.status().unwrap().revision > before.revision + 1);
        assert_eq!(ledgers(), original);
        assert!(r.refresh().is_err());
        let stopped = reads(d.path());
        sql.execute("INSERT INTO due VALUES('remote',4070908800) ON CONFLICT(key) DO UPDATE SET at=excluded.at",[]).unwrap();
        sql.execute_batch("DROP TRIGGER fail_bounded_admission")
            .unwrap();
        r.lock().unwrap().storage_check_at = 0;
        wait_for(|| !r.status().unwrap().storage_stopped).await;
        assert_eq!(reads(d.path()), stopped);
        assert!(r.status().unwrap().settings.paused);
        assert_eq!(r.status().unwrap().identity, before.identity);
        assert_eq!(r.activity(0).unwrap().next_poll_at, deadline);
        assert_eq!(ledgers(), original);
        sql.execute("DELETE FROM due WHERE key='remote'", [])
            .unwrap();
        r.notify_sync();
        wait_for(|| r.overview("a/b", 99).is_ok_and(|p| p.data.is_some())).await;
        assert_eq!(
            n(&sql, "SELECT COUNT(*) FROM watch_locator WHERE number=99"),
            1
        );
        assert_eq!(ledgers(), original);
        assert!(disposable_bytes(&sql).unwrap() <= crate::store::CONTENT_LIMIT);
        assert!(r.status().unwrap().settings.paused);
        r.shutdown().await;
        drop(r);
        let r = Runtime::start(path, |_| {}).unwrap();
        assert!(r.status().unwrap().locked && r.status().unwrap().settings.paused);
        let stopped = reads(d.path());
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(reads(d.path()), stopped);
        r.shutdown().await;
    }

    #[tokio::test]
    async fn source_initialization_failure_protects_membership_and_retries_fresh_after_repair() {
        let _worker_fixture = WORKER_FIXTURES.lock().await;
        use std::sync::{
            Arc,
            atomic::{AtomicU64, Ordering},
        };
        let (d, path) = fixture_store();
        let emitted = Arc::new(AtomicU64::new(0));
        let event = emitted.clone();
        let r = Runtime::start(path.clone(), move |revision| {
            event.store(revision, Ordering::SeqCst);
        })
        .unwrap();
        wait_for(|| r.activity(0).is_ok_and(|a| a.baseline)).await;
        r.set_pause(true).unwrap();
        wait_for(|| !r.status().unwrap().busy).await;
        let sql = rusqlite::Connection::open(&path).unwrap();
        let n = |query: &str| {
            sql.query_row(query, [], |row| row.get::<_, i64>(0))
                .unwrap()
        };
        for i in 0..257 {
            sql.execute(
                "INSERT INTO source_stage VALUES('authored',?,'{}')",
                [format!("old-{i:03}")],
            )
            .unwrap();
        }
        sql.execute_batch("INSERT INTO source_stage VALUES('independent','neighbor','{}');
            CREATE TRIGGER fail_source_init BEFORE DELETE ON source_stage WHEN OLD.id='old-200' BEGIN SELECT RAISE(ABORT,'fixture initialization'); END;").unwrap();
        let before = r.status().unwrap();
        let deadline = r.activity(0).unwrap().next_poll_at;
        let ledgers = || {
            [
                "activity",
                "seen",
                "dedup",
                "activity_intent",
                "intent_member",
                "alert_attempt",
            ]
            .map(|t| n(&format!("SELECT COUNT(*) FROM {t}")))
        };
        let original = ledgers();
        {
            let mut i = r.lock().unwrap();
            i.store
                .enqueue(
                    "discover:authored",
                    &Job::Discover(crate::sync::Discovery::new("authored", "")),
                    true,
                    false,
                    0,
                    true,
                )
                .unwrap();
        }
        r.notify_sync();
        wait_for(|| r.status().unwrap().storage_stopped).await;
        assert_eq!(
            n("SELECT COUNT(*) FROM source_stage WHERE source='authored'"),
            129
        );
        assert_eq!(
            n("SELECT COUNT(*) FROM source_stage WHERE source='independent'"),
            1
        );
        assert_eq!(
            n("SELECT COUNT(*) FROM source_restart WHERE source='authored'"),
            1
        );
        assert_eq!(r.status().unwrap().watch_count, before.watch_count);
        assert_eq!(ledgers(), original);
        assert!(!r.page(0).unwrap().complete);
        assert!(!r.page(0).unwrap().items.is_empty());
        assert!(emitted.load(Ordering::SeqCst) > before.revision);
        assert!(r.refresh().is_err());
        let stopped = reads(d.path());
        sql.execute("INSERT INTO due VALUES('remote',4070908800) ON CONFLICT(key) DO UPDATE SET at=excluded.at",[]).unwrap();
        sql.execute_batch("DROP TRIGGER fail_source_init").unwrap();
        r.lock().unwrap().storage_check_at = 0;
        wait_for(|| !r.status().unwrap().storage_stopped).await;
        assert_eq!(reads(d.path()), stopped);
        assert!(r.status().unwrap().settings.paused);
        assert_eq!(r.status().unwrap().identity, before.identity);
        assert_eq!(r.activity(0).unwrap().next_poll_at, deadline);
        assert_eq!(ledgers(), original);
        assert_eq!(
            n("SELECT COUNT(*) FROM source_restart WHERE source='authored'"),
            1
        );
        sql.execute("DELETE FROM due WHERE key='remote'", [])
            .unwrap();
        r.notify_sync();
        wait_for(|| r.page(0).is_ok_and(|p| p.complete)).await;
        assert_eq!(
            n("SELECT COUNT(*) FROM source_restart WHERE source='authored'"),
            0
        );
        assert_eq!(
            n("SELECT COUNT(*) FROM source_stage WHERE source='authored'"),
            0
        );
        assert_eq!(
            n("SELECT COUNT(*) FROM source_stage WHERE source='independent'"),
            1
        );
        assert_eq!(r.status().unwrap().watch_count, before.watch_count);
        assert_eq!(ledgers(), original);
        assert!(r.status().unwrap().settings.paused);
        r.shutdown().await;
        drop(r);
        let r = Runtime::start(path, |_| {}).unwrap();
        assert!(r.status().unwrap().locked && r.status().unwrap().settings.paused);
        let stopped = reads(d.path());
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(reads(d.path()), stopped);
        r.shutdown().await;
    }

    #[tokio::test]
    async fn pr_purge_discovery_restarts_before_reading_an_obsolete_page() {
        let _worker_fixture = WORKER_FIXTURES.lock().await;
        let (d, path) = fixture_store();
        let r = Runtime::start(path.clone(), |_| {}).unwrap();
        wait_for(|| r.activity(0).is_ok_and(|a| a.baseline)).await;
        r.set_pause(true).unwrap();
        wait_for(|| !r.status().unwrap().busy).await;
        let sql = rusqlite::Connection::open(&path).unwrap();
        sql.execute_batch("INSERT INTO source_stage SELECT 'authored',substr(key,6),json FROM content WHERE key IN ('list:PR_1','list:PR_2');INSERT INTO source_stage SELECT 'independent',substr(key,6),json FROM content WHERE key='list:PR_2';").unwrap();
        {
            let mut i = r.lock().unwrap();
            i.store.activity_purge("a/b", 1).unwrap();
            assert!(crate::pr_purge::source_restart(&i.store.conn, "authored").unwrap());
            let mut old = crate::sync::Discovery::new("authored", "");
            old.cursor = Some("1".into());
            old.received = 1;
            old.total = Some(2);
            i.store
                .enqueue(
                    "discover:authored",
                    &Job::Discover(old),
                    true,
                    false,
                    0,
                    true,
                )
                .unwrap();
        }
        r.notify_sync();
        wait_for(|| r.page(0).is_ok_and(|p| p.complete && p.items.len() == 1)).await;
        assert_eq!(
            sql.query_row(
                "SELECT COUNT(*) FROM source_restart WHERE source='authored'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            0
        );
        assert_eq!(
            sql.query_row(
                "SELECT COUNT(*) FROM source_stage WHERE source='independent'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            1
        );
        assert_eq!(r.status().unwrap().watch_count, 3);
        assert!(r.status().unwrap().settings.paused);
        assert!(!r.status().unwrap().storage_stopped);
        let reads = std::fs::read_to_string(d.path().join("reads")).unwrap();
        let authored = reads
            .lines()
            .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
            .filter(|v| v["query"].as_str().is_some_and(|q| q.contains("Authored")))
            .collect::<Vec<_>>();
        assert!(!authored.is_empty());
        assert!(authored.iter().all(|v| v["variables"]["after"].is_null()));
        r.shutdown().await;
    }

    #[tokio::test]
    async fn pr_purge_partial_startup_is_locked_paused_and_restartable() {
        let _worker_fixture = WORKER_FIXTURES.lock().await;
        let (d, path) = fixture_store();
        let mut s = Store::open(&path).unwrap();
        s.bind(&Identity {
            id: 42,
            login: "reader".into(),
        })
        .unwrap();
        let mut settings = s.settings().unwrap();
        settings.paused = true;
        s.save_settings(&settings).unwrap();
        s.activity_recovery(123).unwrap();
        s.conn.execute_batch("INSERT INTO watch_locator(id,repo,number,head) VALUES('PR_1','a/b',1,'abc');INSERT INTO watch_source VALUES('PR_1','authored');
            INSERT INTO due VALUES('activity:notifications',4070908800);INSERT INTO api_validator VALUES('notifications','etag',NULL,4070908800);
            INSERT INTO activity VALUES('A','a/b',1,'comment',1,1,'eligible',1);INSERT INTO seen VALUES('A',0);INSERT INTO dedup VALUES('A');INSERT INTO activity_intent VALUES(1,'pr','attempted',1);INSERT INTO intent_member VALUES(1,'A');INSERT INTO alert_attempt VALUES('attempt',1);").unwrap();
        s.conn
            .execute(
                "UPDATE activity SET occurred=? WHERE id='A'",
                [now() as i64],
            )
            .unwrap();
        for k in 0..257 {
            s.conn
                .execute(
                    "INSERT INTO content VALUES(?,'{}',4070908800,4070908800)",
                    [format!("api:repos/a/b/issues/1/comments?fixture={k:03}")],
                )
                .unwrap();
        }
        s.conn.execute_batch("CREATE TRIGGER fail_startup_pr_purge BEFORE DELETE ON content WHEN OLD.key='api:repos/a/b/issues/1/comments?fixture=200' BEGIN SELECT RAISE(ABORT,'fixture startup PR purge'); END;").unwrap();
        assert!(s.activity_purge("a/b", 1).is_err());
        let revision = s.revision().unwrap();
        drop(s);
        let r = Runtime::start(path.clone(), |_| {}).unwrap();
        let status = r.status().unwrap();
        assert!(status.locked && status.settings.paused && status.storage_stopped);
        assert_eq!(status.revision, revision);
        assert_eq!(reads(d.path()), 0);
        assert!(r.overview("a/b", 1).is_err());
        assert!(r.refresh().is_err());
        r.lock().unwrap().storage_check_at = 0;
        wait_for(|| r.lock().unwrap().storage_check_at != 0).await;
        assert!(r.status().unwrap().storage_stopped);
        assert_eq!(reads(d.path()), 0);
        let sql = rusqlite::Connection::open(&path).unwrap();
        sql.execute_batch("DROP TRIGGER fail_startup_pr_purge")
            .unwrap();
        r.lock().unwrap().storage_check_at = 0;
        wait_for(|| !r.status().unwrap().storage_stopped).await;
        let status = r.status().unwrap();
        assert!(status.locked && status.settings.paused);
        assert_eq!(status.identity.unwrap().id, 42);
        assert_eq!(reads(d.path()), 0);
        for (q, expected) in [
            ("SELECT COUNT(*) FROM content", 0),
            ("SELECT COUNT(*) FROM watch_source", 1),
            ("SELECT COUNT(*) FROM activity", 1),
            ("SELECT seen FROM seen", 0),
            (
                "SELECT COUNT(*) FROM activity_intent WHERE state='attempted'",
                1,
            ),
            ("SELECT COUNT(*) FROM alert_attempt", 1),
            ("SELECT boundary FROM activity_cycle", 123),
            (
                "SELECT at FROM due WHERE key='activity:notifications'",
                4070908800,
            ),
            ("SELECT next_at FROM api_validator", 4070908800),
        ] {
            assert_eq!(
                sql.query_row(q, [], |r| r.get::<_, i64>(0)).unwrap(),
                expected,
                "{q}"
            );
        }
        r.shutdown().await;
    }

    #[tokio::test]
    async fn check_purge_worker_denial_protects_and_repairs_paused_without_neighbor_loss() {
        let _worker_fixture = WORKER_FIXTURES.lock().await;
        let (d, path) = fixture_store();
        let emitted = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let event = emitted.clone();
        let r = Runtime::start(path.clone(), move |revision| {
            event.store(revision, std::sync::atomic::Ordering::SeqCst);
        })
        .unwrap();
        wait_for(|| r.activity(0).is_ok_and(|a| a.baseline)).await;
        r.select("a/b".into(), 1).unwrap();
        wait_for(|| r.overview("a/b", 1).is_ok_and(|a| a.data.is_some())).await;
        r.set_pause(true).unwrap();
        wait_for(|| !r.status().unwrap().busy).await;
        let sql = rusqlite::Connection::open(&path).unwrap();
        let count = |sql_text: &str| sql.query_row(sql_text, [], |r| r.get::<_, i64>(0)).unwrap();
        let ledgers = || {
            [
                "activity",
                "seen",
                "dedup",
                "activity_intent",
                "intent_member",
                "alert_attempt",
            ]
            .map(|t| count(&format!("SELECT COUNT(*) FROM {t}")))
        };
        let original = ledgers();
        let before = r.status().unwrap();
        let deadline = r.activity(0).unwrap().next_poll_at;
        let original_contexts = count("SELECT COUNT(*) FROM check_context WHERE pr_id='PR_1'");
        let neighbor_contexts = count("SELECT COUNT(*) FROM check_context WHERE pr_id='PR_2'");
        assert!(neighbor_contexts > 0);
        let target = crate::sync::WatchTarget {
            id: "PR_1".into(),
            repo: "a/b".into(),
            number: 1,
            generation: count("SELECT epoch FROM watch_locator WHERE id='PR_1'") as u64,
        };
        for n in 0..257 {
            sql.execute(
                "INSERT INTO check_context VALUES('PR_1','abc',?,'{}',0)",
                [format!("extra-{n:03}")],
            )
            .unwrap();
        }
        sql.execute_batch("CREATE TRIGGER fail_check_purge BEFORE DELETE ON check_context WHEN OLD.pr_id='PR_1' AND OLD.id='extra-200' BEGIN SELECT RAISE(ABORT,'fixture check purge'); END;").unwrap();
        std::fs::write(d.path().join("scenario.json"), r#"{"deny_checks":true}"#).unwrap();
        {
            let mut i = r.lock().unwrap();
            i.store
                .enqueue(
                    "heads:check-denial",
                    &Job::Heads(vec![target.clone()]),
                    true,
                    true,
                    0,
                    true,
                )
                .unwrap();
        }
        r.notify_sync();
        wait_for(|| r.status().unwrap().storage_stopped).await;
        assert!(!r.status().unwrap().locked);
        assert_eq!(
            count("SELECT COUNT(*) FROM check_context WHERE pr_id='PR_1'"),
            original_contexts + 257 - 128
        );
        assert_eq!(
            count("SELECT epoch FROM watch_locator WHERE id='PR_1'"),
            target.generation as i64 + 1
        );
        assert!(emitted.load(std::sync::atomic::Ordering::SeqCst) > before.revision);
        let denied = r.checks("a/b", 1, 0).unwrap();
        assert!(denied.items.is_empty());
        assert_eq!(denied.error.as_deref(), Some(crate::check_purge::MESSAGE));
        assert_eq!(
            count("SELECT COUNT(*) FROM check_context WHERE pr_id='PR_2'"),
            neighbor_contexts
        );
        assert!(!r.checks("a/b", 2, 0).unwrap().items.is_empty());
        assert!(r.overview("a/b", 1).unwrap().data.is_some());
        assert!(r.refresh().is_err());
        sql.execute("INSERT INTO due VALUES('remote',4070908800) ON CONFLICT(key) DO UPDATE SET at=excluded.at",[]).unwrap();
        let stopped = reads(d.path());
        r.lock().unwrap().storage_check_at = 0;
        wait_for(|| r.lock().unwrap().storage_check_at != 0).await;
        assert!(r.status().unwrap().storage_stopped);
        assert_eq!(reads(d.path()), stopped);
        sql.execute_batch("DROP TRIGGER fail_check_purge").unwrap();
        r.lock().unwrap().storage_check_at = 0;
        wait_for(|| !r.status().unwrap().storage_stopped).await;
        assert_eq!(reads(d.path()), stopped);
        assert!(r.status().unwrap().settings.paused);
        assert_eq!(r.status().unwrap().identity, before.identity);
        assert_eq!(r.activity(0).unwrap().next_poll_at, deadline);
        assert_eq!(ledgers(), original);
        assert_eq!(
            count("SELECT COUNT(*) FROM check_context WHERE pr_id='PR_1'"),
            0
        );
        assert_eq!(
            count("SELECT COUNT(*) FROM check_context WHERE pr_id='PR_2'"),
            neighbor_contexts
        );
        assert!(!crate::check_purge::any_pending(&r.lock().unwrap().store.conn).unwrap());
        assert!(!r.lock().unwrap().store.deny_checks(&target).unwrap());
        r.shutdown().await;
        drop(r);
        let r = Runtime::start(path, |_| {}).unwrap();
        assert!(r.status().unwrap().locked && r.status().unwrap().settings.paused);
        assert_eq!(reads(d.path()), stopped);
        assert_eq!(ledgers(), original);
        r.shutdown().await;
    }

    #[tokio::test]
    async fn check_purge_partial_startup_stays_locked_and_paused_until_repair() {
        let _worker_fixture = WORKER_FIXTURES.lock().await;
        let (d, path) = fixture_store();
        let mut s = Store::open(&path).unwrap();
        s.bind(&Identity {
            id: 42,
            login: "reader".into(),
        })
        .unwrap();
        let mut settings = s.settings().unwrap();
        settings.paused = true;
        s.save_settings(&settings).unwrap();
        s.activity_recovery(123).unwrap();
        s.conn.execute_batch("INSERT INTO watch_locator(id,repo,number,head) VALUES('PR_1','a/b',1,'abc');INSERT INTO watch_source VALUES('PR_1','authored');
            INSERT INTO check_state VALUES('PR_1','abc',1,1,NULL,4070908800);INSERT INTO due VALUES('activity:notifications',4070908800);INSERT INTO api_validator VALUES('notifications','etag',NULL,4070908800);").unwrap();
        for n in 0..257 {
            s.conn
                .execute(
                    "INSERT INTO check_context VALUES('PR_1','abc',?,'{}',0)",
                    [format!("extra-{n:03}")],
                )
                .unwrap();
        }
        s.conn.execute_batch("CREATE TRIGGER fail_startup_check_purge BEFORE DELETE ON check_context WHEN OLD.id='extra-200' BEGIN SELECT RAISE(ABORT,'fixture startup check purge'); END;").unwrap();
        assert!(
            s.deny_checks(&crate::sync::WatchTarget {
                id: "PR_1".into(),
                repo: "a/b".into(),
                number: 1,
                generation: 0
            })
            .is_err()
        );
        let revision = s.revision().unwrap();
        drop(s);
        let r = Runtime::start(path.clone(), |_| {}).unwrap();
        let status = r.status().unwrap();
        assert!(status.storage_stopped && status.locked && status.settings.paused);
        assert_eq!(status.revision, revision);
        assert_eq!(reads(d.path()), 0);
        assert!(r.checks("a/b", 1, 0).is_err());
        assert!(r.refresh().is_err());
        r.lock().unwrap().storage_check_at = 0;
        wait_for(|| r.lock().unwrap().storage_check_at != 0).await;
        assert!(r.status().unwrap().storage_stopped);
        assert_eq!(reads(d.path()), 0);
        let sql = rusqlite::Connection::open(&path).unwrap();
        sql.execute_batch("DROP TRIGGER fail_startup_check_purge")
            .unwrap();
        r.lock().unwrap().storage_check_at = 0;
        wait_for(|| !r.status().unwrap().storage_stopped).await;
        let status = r.status().unwrap();
        assert!(status.locked && status.settings.paused);
        assert_eq!(status.identity.unwrap().id, 42);
        assert_eq!(reads(d.path()), 0);
        assert_eq!(
            sql.query_row(
                "SELECT COUNT(*) FROM check_context WHERE pr_id='PR_1'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            0
        );
        assert_eq!(
            sql.query_row("SELECT boundary FROM activity_cycle", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            123
        );
        assert_eq!(
            sql.query_row(
                "SELECT at FROM due WHERE key='activity:notifications'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            4070908800
        );
        assert_eq!(
            sql.query_row("SELECT next_at FROM api_validator", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            4070908800
        );
        r.shutdown().await;
    }

    #[tokio::test]
    async fn clear_partial_failure_repairs_paused_without_reads_or_old_generation_publication() {
        let _worker_fixture = WORKER_FIXTURES.lock().await;
        let (d, path) = fixture_store();
        let emitted = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let event = emitted.clone();
        let r = Runtime::start(path.clone(), move |revision| {
            event.store(revision, std::sync::atomic::Ordering::SeqCst);
        })
        .unwrap();
        wait_for(|| r.activity(0).is_ok_and(|a| a.baseline)).await;
        r.select("a/b".into(), 1).unwrap();
        wait_for(|| r.overview("a/b", 1).is_ok_and(|v| v.data.is_some())).await;
        r.set_pause(true).unwrap();
        wait_for(|| !r.status().unwrap().busy).await;
        let sql = rusqlite::Connection::open(&path).unwrap();
        sql.execute_batch("INSERT INTO due VALUES('remote',4070908800) ON CONFLICT(key) DO UPDATE SET at=excluded.at;").unwrap();
        for n in 0..257 {
            sql.execute("INSERT INTO stage VALUES(?,'{}')", [n.to_string()])
                .unwrap();
            sql.execute(
                "INSERT INTO activity_stage VALUES('a/b',1,?,'{}')",
                [n.to_string()],
            )
            .unwrap();
        }
        sql.execute_batch("CREATE TRIGGER fail_runtime_clear BEFORE DELETE ON activity_stage BEGIN SELECT RAISE(ABORT,'fixture clear_content fault'); END;").unwrap();
        let before = r.status().unwrap();
        let deadline = r.activity(0).unwrap().next_poll_at;
        let ledgers = || {
            [
                "activity",
                "seen",
                "dedup",
                "activity_intent",
                "intent_member",
                "alert_attempt",
            ]
            .map(|table| {
                sql.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| {
                    r.get::<_, i64>(0)
                })
                .unwrap()
            })
        };
        let original = ledgers();
        let stopped = reads(d.path());
        let generation = r.lock().unwrap().store.generation().unwrap();
        emitted.store(0, std::sync::atomic::Ordering::SeqCst);
        assert!(r.clear().is_err());
        assert_eq!(
            emitted.load(std::sync::atomic::Ordering::SeqCst),
            before.revision + 1
        );
        assert!(r.status().unwrap().storage_stopped);
        assert_eq!(
            r.lock().unwrap().store.generation().unwrap(),
            generation + 1
        );
        assert_eq!(
            sql.query_row("SELECT COUNT(*) FROM content", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            0
        );
        let remaining = sql
            .query_row("SELECT COUNT(*) FROM stage", [], |r| r.get::<_, i64>(0))
            .unwrap();
        assert!(remaining > 0 && remaining < 257);
        assert!(r.overview("a/b", 1).unwrap().data.is_none());
        assert!(r.checks("a/b", 1, 0).unwrap().items.is_empty());
        assert!(!r.page(0).unwrap().complete);
        assert!(r.refresh().is_err());
        assert_eq!(reads(d.path()), stopped);
        r.lock().unwrap().storage_check_at = 0;
        wait_for(|| r.lock().unwrap().storage_check_at != 0).await;
        assert!(r.status().unwrap().storage_stopped);
        assert_eq!(reads(d.path()), stopped);
        sql.execute_batch("DROP TRIGGER fail_runtime_clear")
            .unwrap();
        r.lock().unwrap().storage_check_at = 0;
        wait_for(|| !r.status().unwrap().storage_stopped).await;
        assert_eq!(reads(d.path()), stopped);
        assert!(r.status().unwrap().settings.paused);
        assert_eq!(r.status().unwrap().identity, before.identity);
        assert_eq!(r.activity(0).unwrap().next_poll_at, deadline);
        assert_eq!(ledgers(), original);
        assert!(!crate::clear_content::pending(&r.lock().unwrap().store.conn).unwrap());
        assert!(
            !r.lock()
                .unwrap()
                .store
                .put("late", &"obsolete", generation)
                .unwrap()
        );
        assert!(r.overview("a/b", 1).unwrap().data.is_none());
        r.shutdown().await;
        drop(r);
        // Restart after repair remains locked and paused until explicit identity
        // validation, with no additional synthetic child invocation.
        let r = Runtime::start(path, |_| {}).unwrap();
        assert!(r.status().unwrap().locked && r.status().unwrap().settings.paused);
        assert_eq!(reads(d.path()), stopped);
        assert_eq!(ledgers(), original);
        r.shutdown().await;
    }

    #[tokio::test]
    async fn invalidation_partial_failure_repairs_paused_without_reads_or_old_generation_publication()
     {
        let _worker_fixture = WORKER_FIXTURES.lock().await;
        let (d, path) = fixture_store();
        let r = Runtime::start(path.clone(), |_| {}).unwrap();
        wait_for(|| r.activity(0).is_ok_and(|a| a.baseline)).await;
        r.select("a/b".into(), 1).unwrap();
        wait_for(|| r.overview("a/b", 1).is_ok_and(|v| v.data.is_some())).await;
        r.set_pause(true).unwrap();
        wait_for(|| !r.status().unwrap().busy).await;
        let sql = rusqlite::Connection::open(&path).unwrap();
        sql.execute_batch("INSERT INTO due VALUES('remote',4070908800) ON CONFLICT(key) DO UPDATE SET at=excluded.at;").unwrap();
        for n in 0..257 {
            sql.execute("INSERT INTO stage VALUES(?,'{}')", [n.to_string()])
                .unwrap();
            sql.execute(
                "INSERT INTO activity_stage VALUES('a/b',1,?,'{}')",
                [n.to_string()],
            )
            .unwrap();
        }
        sql.execute_batch("CREATE TRIGGER fail_runtime_invalidation BEFORE DELETE ON activity_stage BEGIN SELECT RAISE(ABORT,'fixture invalidation fault'); END;").unwrap();
        let before = r.status().unwrap();
        let deadline = r.activity(0).unwrap().next_poll_at;
        let ledgers = || {
            [
                "activity",
                "seen",
                "dedup",
                "activity_intent",
                "intent_member",
                "alert_attempt",
            ]
            .map(|table| {
                sql.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| {
                    r.get::<_, i64>(0)
                })
                .unwrap()
            })
        };
        let original = ledgers();
        let stopped = reads(d.path());
        let generation = r.lock().unwrap().store.generation().unwrap();
        assert!(
            r.configure(
                before.settings.gh_path.clone().unwrap(),
                before.settings.config_dir.clone().unwrap()
            )
            .is_err()
        );
        assert!(r.status().unwrap().storage_stopped);
        assert_eq!(
            r.lock().unwrap().store.generation().unwrap(),
            generation + 1
        );
        assert_eq!(
            sql.query_row("SELECT COUNT(*) FROM stage", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            1
        );
        assert!(r.overview("a/b", 1).unwrap().data.is_some());
        assert!(
            r.checks("a/b", 1, 0)
                .unwrap()
                .commits
                .iter()
                .all(|c| !c.complete)
        );
        assert!(r.refresh().is_err());
        assert_eq!(reads(d.path()), stopped);
        r.lock().unwrap().storage_check_at = 0;
        wait_for(|| r.lock().unwrap().storage_check_at != 0).await;
        assert!(r.status().unwrap().storage_stopped);
        assert_eq!(reads(d.path()), stopped);
        sql.execute_batch("DROP TRIGGER fail_runtime_invalidation")
            .unwrap();
        r.lock().unwrap().storage_check_at = 0;
        wait_for(|| !r.status().unwrap().storage_stopped).await;
        assert_eq!(reads(d.path()), stopped);
        assert!(r.status().unwrap().settings.paused);
        assert_eq!(r.status().unwrap().identity, before.identity);
        assert_eq!(r.activity(0).unwrap().next_poll_at, deadline);
        assert_eq!(ledgers(), original);
        assert!(!crate::invalidation::pending(&r.lock().unwrap().store.conn).unwrap());
        assert!(
            !r.lock()
                .unwrap()
                .store
                .put("late", &"obsolete", generation)
                .unwrap()
        );
        assert!(r.overview("a/b", 1).unwrap().data.is_some());
        r.shutdown().await;
        drop(r);
        // Restart after repair remains locked and paused until explicit identity
        // validation, with no additional synthetic child invocation.
        let r = Runtime::start(path, |_| {}).unwrap();
        assert!(r.status().unwrap().locked && r.status().unwrap().settings.paused);
        assert_eq!(reads(d.path()), stopped);
        assert_eq!(ledgers(), original);
        r.shutdown().await;
    }

    #[tokio::test]
    async fn enrollment_fault_protects_paused_runtime_and_repair_preserves_history() {
        let _worker_fixture = WORKER_FIXTURES.lock().await;
        let (d, path) = fixture_store();
        let r = Runtime::start(path.clone(), |_| {}).unwrap();
        wait_for(|| r.activity(0).is_ok_and(|a| a.baseline)).await;
        r.set_pause(true).unwrap();
        wait_for(|| !r.status().unwrap().busy).await;
        let sql = rusqlite::Connection::open(&path).unwrap();
        // Keep manual scheduling observable without dispatching synthetic reads.
        sql.execute_batch("INSERT INTO due VALUES('remote',4070908800) ON CONFLICT(key) DO UPDATE SET at=excluded.at;
            INSERT INTO due VALUES('activity:notifications',4070908800) ON CONFLICT(key) DO UPDATE SET at=excluded.at;
            UPDATE activity_cycle SET mode='recovery',boundary=123;").unwrap();
        let tx = sql.unchecked_transaction().unwrap();
        for n in 1..=257 {
            tx.execute(
                "INSERT INTO watch_locator(id,repo,number) VALUES(?,'enroll/repo',?)",
                rusqlite::params![format!("enroll-{n}"), n],
            )
            .unwrap();
            tx.execute(
                "INSERT INTO watch_source VALUES(?,'direct')",
                [format!("enroll-{n}")],
            )
            .unwrap();
        }
        tx.commit().unwrap();
        sql.execute_batch("CREATE TRIGGER fail_enrollment BEFORE INSERT ON activity_target WHEN NEW.repo='enroll/repo' AND NEW.number=200 BEGIN SELECT RAISE(ABORT,'fixture enrollment failure'); END;").unwrap();
        let identity = r.status().unwrap().identity;
        let ledger = || {
            [
                "seen",
                "dedup",
                "activity_intent",
                "intent_member",
                "alert_attempt",
            ]
            .map(|table| {
                sql.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                    row.get::<_, i64>(0)
                })
                .unwrap()
            })
        };
        let history = ledger();
        let stopped_reads = reads(d.path());
        r.refresh().unwrap(); // first bounded enrollment batch
        assert!(r.refresh().is_err()); // second batch fails after earlier commit
        assert!(r.status().unwrap().storage_stopped);
        assert_eq!(
            sql.query_row(
                "SELECT COUNT(*) FROM activity_target WHERE repo='enroll/repo'",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
            128
        );
        assert!(r.refresh().is_err());
        assert_eq!(reads(d.path()), stopped_reads);
        sql.execute_batch("DROP TRIGGER fail_enrollment").unwrap();
        r.lock().unwrap().storage_check_at = 0;
        wait_for(|| !r.status().unwrap().storage_stopped).await;
        assert!(r.status().unwrap().settings.paused);
        assert_eq!(r.status().unwrap().identity, identity);
        assert_eq!(reads(d.path()), stopped_reads);
        assert_eq!(ledger(), history);
        assert_eq!(
            sql.query_row("SELECT boundary FROM activity_cycle", [], |row| row
                .get::<_, i64>(0))
                .unwrap(),
            123
        );
        assert_eq!(r.activity(0).unwrap().next_poll_at, 4070908800);
        // Repair does not enroll or hydrate automatically through Pause. Later
        // explicit passes finish enrollment under the same remote deadline.
        r.refresh().unwrap();
        r.refresh().unwrap();
        assert_eq!(
            sql.query_row(
                "SELECT COUNT(*) FROM activity_target WHERE repo='enroll/repo'",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
            257
        );
        assert_eq!(reads(d.path()), stopped_reads);
        assert_eq!(ledger(), history);
        assert!(r.status().unwrap().settings.paused);
        r.shutdown().await;
    }
    #[tokio::test]
    async fn selected_publication_eviction_fault_protects_and_repairs_without_resuming() {
        let _worker_fixture = WORKER_FIXTURES.lock().await;
        let (d, path) = fixture_store();
        let r = Runtime::start(path.clone(), |_| {}).unwrap();
        wait_for(|| r.activity(0).is_ok_and(|a| a.baseline)).await;
        r.select("a/b".into(), 1).unwrap();
        wait_for(|| r.overview("a/b", 1).is_ok_and(|v| v.data.is_some())).await;
        r.set_pause(true).unwrap();
        wait_for(|| !r.status().unwrap().busy).await;
        let sql = rusqlite::Connection::open(&path).unwrap();
        sql.execute_batch(
            "INSERT INTO content VALUES('expired-eviction','{}',1,1);
            CREATE TRIGGER fail_eviction BEFORE DELETE ON content WHEN OLD.key='expired-eviction'
            BEGIN SELECT RAISE(ABORT,'fixture eviction failure'); END;",
        )
        .unwrap();
        // Manual selection traverses the real child runner and overview parser.
        // Publication must roll back before the worker enters storage protection.
        r.select("a/b".into(), 99).unwrap();
        wait_for(|| r.status().unwrap().storage_stopped).await;
        assert_eq!(
            sql.query_row(
                "SELECT COUNT(*) FROM watch_locator WHERE number=99",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
            0
        );
        assert_eq!(
            sql.query_row(
                "SELECT COUNT(*) FROM content WHERE key='pr:a/b:99'",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
            0
        );
        assert!(r.overview("a/b", 1).unwrap().data.is_some());
        assert!(r.refresh().is_err());
        let stopped_reads = reads(d.path());
        r.select("a/b".into(), 1).unwrap();
        assert_eq!(reads(d.path()), stopped_reads);
        sql.execute_batch("DROP TRIGGER fail_eviction").unwrap();
        r.lock().unwrap().storage_check_at = 0;
        wait_for(|| !r.status().unwrap().storage_stopped).await;
        assert!(r.status().unwrap().settings.paused);
        assert_eq!(reads(d.path()), stopped_reads);
        r.select("a/b".into(), 99).unwrap();
        wait_for(|| r.overview("a/b", 99).is_ok_and(|v| v.data.is_some())).await;
        assert!(r.status().unwrap().settings.paused);
        assert_eq!(
            sql.query_row(
                "SELECT COUNT(*) FROM activity_intent WHERE state='pending'",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
            0
        );
        r.shutdown().await;
    }
    #[tokio::test]
    async fn retention_runs_while_paused_and_failure_protects_until_repair() {
        let _worker_fixture = WORKER_FIXTURES.lock().await;
        let (d, path) = fixture_store();
        let r = Runtime::start(path.clone(), |_| {}).unwrap();
        wait_for(|| r.activity(0).is_ok_and(|a| a.baseline)).await;
        r.set_pause(true).unwrap();
        wait_for(|| !r.status().unwrap().busy).await;
        let stopped_reads = reads(d.path());
        let sql = rusqlite::Connection::open(&path).unwrap();
        let seed = |id: &str| {
            sql.execute(
                "INSERT INTO activity VALUES(?,'a/b',1,'comment',1,1,'eligible',1)",
                [id],
            )
            .unwrap();
            sql.execute("INSERT INTO seen VALUES(?,0)", [id]).unwrap();
            sql.execute("INSERT INTO dedup VALUES(?)", [id]).unwrap();
            sql.execute(
                "INSERT INTO content VALUES(?,'{}',?,?)",
                rusqlite::params![format!("activity-payload:{id}"), now() as i64, now() as i64],
            )
            .unwrap();
        };
        let exists = |id: &str| {
            sql.query_row(
                "SELECT EXISTS(SELECT 1 FROM activity WHERE id=?)",
                [id],
                |r| r.get::<_, bool>(0),
            )
            .unwrap()
        };
        seed("expired-paused");
        r.lock().unwrap().storage_check_at = 0;
        wait_for(|| !exists("expired-paused")).await;
        assert_eq!(reads(d.path()), stopped_reads);
        assert!(r.status().unwrap().settings.paused);
        assert_eq!(
            sql.query_row(
                "SELECT seen FROM seen WHERE source_id='expired-paused'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            0
        );

        seed("failed-expiry");
        sql.execute_batch("CREATE TRIGGER fail_retention BEFORE DELETE ON activity BEGIN SELECT RAISE(ABORT,'fixture cleanup failure'); END;").unwrap();
        r.lock().unwrap().storage_check_at = 0;
        wait_for(|| r.status().unwrap().storage_stopped).await;
        assert!(exists("failed-expiry"));
        assert!(r.activity(0).is_ok(), "protected cache remains readable");
        assert!(r.refresh().is_err());
        assert_eq!(reads(d.path()), stopped_reads);
        sql.execute_batch("DROP TRIGGER fail_retention").unwrap();
        r.lock().unwrap().storage_check_at = 0;
        wait_for(|| !r.status().unwrap().storage_stopped).await;
        assert!(!exists("failed-expiry"));
        assert!(r.status().unwrap().settings.paused);
        assert_eq!(reads(d.path()), stopped_reads);
        assert_eq!(
            sql.query_row(
                "SELECT COUNT(*) FROM activity_intent WHERE state='pending'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            0
        );
        r.shutdown().await;
    }
    #[tokio::test]
    async fn paused_check_reclamation_failure_rolls_back_and_repairs_without_reads() {
        let _worker_fixture = WORKER_FIXTURES.lock().await;
        let (d, path) = fixture_store();
        let r = Runtime::start(path.clone(), |_| {}).unwrap();
        wait_for(|| r.activity(0).is_ok_and(|a| a.baseline)).await;
        r.set_pause(true).unwrap();
        wait_for(|| !r.status().unwrap().busy).await;
        let stopped_reads = reads(d.path());
        let sql = rusqlite::Connection::open(&path).unwrap();
        sql.execute_batch("INSERT INTO check_state VALUES('expired','sha',1,1,NULL,1);
            INSERT INTO check_context VALUES('expired','sha','ctx','{}',0);
            INSERT INTO content VALUES('expired-check-companion','{}',1,1);
            CREATE TRIGGER fail_check_retention BEFORE DELETE ON check_context BEGIN SELECT RAISE(ABORT,'fixture check cleanup failure'); END;").unwrap();
        r.lock().unwrap().storage_check_at = 0;
        wait_for(|| r.status().unwrap().storage_stopped).await;
        assert!(r.refresh().is_err());
        assert_eq!(
            sql.query_row(
                "SELECT COUNT(*) FROM check_state WHERE pr_id='expired'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            1
        );
        assert_eq!(
            sql.query_row(
                "SELECT COUNT(*) FROM content WHERE key='expired-check-companion'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            1
        );
        assert_eq!(reads(d.path()), stopped_reads);
        sql.execute_batch("DROP TRIGGER fail_check_retention")
            .unwrap();
        r.lock().unwrap().storage_check_at = 0;
        wait_for(|| !r.status().unwrap().storage_stopped).await;
        assert!(r.status().unwrap().settings.paused);
        assert_eq!(reads(d.path()), stopped_reads);
        assert_eq!(
            sql.query_row(
                "SELECT COUNT(*) FROM check_context WHERE pr_id='expired'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            0
        );
        assert_eq!(
            sql.query_row(
                "SELECT COUNT(*) FROM check_state WHERE pr_id='expired'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            0
        );
        assert_eq!(
            sql.query_row(
                "SELECT COUNT(*) FROM activity_intent WHERE state='pending'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            0
        );
        r.shutdown().await;
    }
    #[tokio::test]
    async fn metadata_cleanup_fault_repairs_paused_and_rehydrates_unconditionally() {
        let _worker_fixture = WORKER_FIXTURES.lock().await;
        let (d, path) = fixture_store();
        std::fs::write(d.path().join("scenario.json"), r#"{"validators":true}"#).unwrap();
        let r = Runtime::start(path.clone(), |_| {}).unwrap();
        wait_for(|| r.activity(0).is_ok_and(|a| a.baseline)).await;
        r.set_pause(true).unwrap();
        wait_for(|| !r.status().unwrap().busy).await;
        let stopped_reads = reads(d.path());
        let activity_count = r.activity(0).unwrap().items.len();
        let sql = rusqlite::Connection::open(&path).unwrap();
        let url = "repos/a/b/issues/1/comments?per_page=50&page=1";
        assert!(
            sql.query_row(
                "SELECT etag IS NOT NULL FROM api_validator WHERE url=?",
                [url],
                |r| r.get::<_, bool>(0)
            )
            .unwrap()
        );
        sql.execute(
            "UPDATE content SET accessed=1 WHERE key=?",
            [format!("api:{url}")],
        )
        .unwrap();
        sql.execute_batch("INSERT INTO check_state VALUES('metadata-orphan','sha',1,1,NULL,1);
            CREATE TRIGGER fail_metadata BEFORE DELETE ON check_state BEGIN SELECT RAISE(ABORT,'fixture metadata failure'); END;").unwrap();
        let deadline = r.activity(0).unwrap().next_poll_at;
        r.lock().unwrap().storage_check_at = 0;
        wait_for(|| r.status().unwrap().storage_stopped).await;
        assert!(r.refresh_activity("a/b", 1).is_err());
        assert_eq!(reads(d.path()), stopped_reads);
        for (table, column, value) in [
            ("content", "key", format!("api:{url}")),
            ("resource_state", "key", format!("api:{url}")),
            ("api_validator", "url", url.into()),
            ("check_state", "pr_id", "metadata-orphan".into()),
        ] {
            assert_eq!(
                sql.query_row(
                    &format!("SELECT COUNT(*) FROM {table} WHERE {column}=?"),
                    [&value],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
                1
            );
        }
        sql.execute_batch("DROP TRIGGER fail_metadata").unwrap();
        r.lock().unwrap().storage_check_at = 0;
        wait_for(|| !r.status().unwrap().storage_stopped).await;
        assert!(r.status().unwrap().settings.paused);
        assert_eq!(reads(d.path()), stopped_reads);
        assert_eq!(r.activity(0).unwrap().next_poll_at, deadline);
        assert_eq!(
            sql.query_row(
                "SELECT COUNT(*) FROM api_validator WHERE url=?",
                [url],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            0
        );
        assert_eq!(
            sql.query_row(
                "SELECT COUNT(*) FROM resource_state WHERE key=?",
                [format!("api:{url}")],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            0
        );
        assert_eq!(
            sql.query_row(
                "SELECT COUNT(*) FROM check_state WHERE pr_id='metadata-orphan'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            0
        );
        // Explicit reads after repair repopulate reclaimed source data, without
        // replaying Activity or resuming monitoring or native delivery.
        r.refresh_activity("a/b", 1).unwrap();
        wait_for(|| reads(d.path()) > stopped_reads + 14 && !r.status().unwrap().busy).await;
        let log = std::fs::read_to_string(d.path().join("reads")).unwrap();
        let requests = log
            .lines()
            .skip(stopped_reads)
            .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
            .filter(|v| {
                v["args"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|a| a.as_str() == Some(url))
            })
            .collect::<Vec<_>>();
        assert_eq!(requests.len(), 1);
        assert!(
            !requests[0]["args"]
                .as_array()
                .unwrap()
                .iter()
                .any(|a| a.as_str().is_some_and(
                    |s| s.starts_with("If-None-Match:") || s.starts_with("If-Modified-Since:")
                ))
        );
        assert!(
            sql.query_row(
                "SELECT etag IS NOT NULL FROM api_validator WHERE url=?",
                [url],
                |r| r.get::<_, bool>(0)
            )
            .unwrap()
        );
        assert_eq!(r.activity(0).unwrap().items.len(), activity_count);
        assert!(r.status().unwrap().settings.paused);
        assert_eq!(
            sql.query_row(
                "SELECT COUNT(*) FROM activity_intent WHERE state='pending'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            0
        );
        r.shutdown().await;
    }
    #[tokio::test]
    async fn all_writes_denied_preserves_cached_navigation_and_repair_does_not_replay() {
        let _worker_fixture = WORKER_FIXTURES.lock().await;
        let (d, path) = fixture_store();
        let r = Runtime::start(path.clone(), |_| {}).unwrap();
        wait_for(|| r.activity(0).is_ok_and(|a| a.baseline)).await;
        r.select("a/b".into(), 1).unwrap();
        wait_for(|| r.overview("a/b", 1).is_ok_and(|v| v.data.is_some())).await;
        r.set_pause(true).unwrap();
        wait_for(|| !r.status().unwrap().busy).await;
        let before = r.overview("a/b", 1).unwrap();
        let poll = r.activity(0).unwrap().next_poll_at;
        let revision = r.status().unwrap().revision;
        // Actual SQLite READONLY responses on every write of this runtime's
        // connection, rather than a fault on one statement or table.
        r.lock()
            .unwrap()
            .store
            .conn
            .execute_batch("PRAGMA query_only=ON")
            .unwrap();
        // A check read discovers the denied access-time write and retries protected.
        assert!(r.checks("a/b", 1, 0).is_ok());
        assert!(r.status().unwrap().storage_stopped);
        assert_eq!(
            r.overview("a/b", 1).unwrap().data.unwrap().head,
            before.data.unwrap().head
        );
        assert!(r.status().unwrap().storage_stopped);
        assert!(r.page(0).unwrap().complete);
        assert!(r.requested_page(0).unwrap().complete);
        assert!(r.activity(0).unwrap().baseline);
        assert!(r.checks("a/b", 1, 0).is_ok());
        r.select("a/b".into(), 1).unwrap();
        assert!(r.refresh().is_err());
        assert!(r.refresh_activity("a/b", 1).is_err());
        assert!(r.set_pause(false).is_err());
        assert!(r.set_theme(Theme::default()).is_err());
        assert!(r.mark_activity_seen(&["missing".into()]).is_err());
        assert!(r.clear().is_err());
        assert!(r.status().unwrap().settings.paused);
        assert_eq!(r.status().unwrap().revision, revision);
        let count = reads(d.path());
        r.lock().unwrap().storage_check_at = 0;
        tokio::time::sleep(Duration::from_millis(1200)).await;
        assert!(r.status().unwrap().storage_stopped);
        assert_eq!(reads(d.path()), count);
        r.lock()
            .unwrap()
            .store
            .conn
            .execute_batch("PRAGMA query_only=OFF")
            .unwrap();
        r.lock().unwrap().storage_check_at = 0;
        wait_for(|| !r.status().unwrap().storage_stopped).await;
        assert!(r.status().unwrap().settings.paused);
        assert_eq!(r.activity(0).unwrap().next_poll_at, poll);
        assert_eq!(reads(d.path()), count);
        r.shutdown().await;
        let reopened = Store::open(&path).unwrap();
        assert!(reopened.settings().unwrap().paused);
        assert_eq!(
            reopened
                .conn
                .query_row(
                    "SELECT COUNT(*) FROM activity_intent WHERE state='pending'",
                    [],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
            0
        );
    }
    #[tokio::test]
    async fn low_space_restart_defers_writes_and_preserves_pause_identity_and_deadlines() {
        let _worker_fixture = WORKER_FIXTURES.lock().await;
        use std::sync::atomic::{AtomicBool, Ordering};
        let (d, path) = fixture_store();
        let r = Runtime::start(path.clone(), |_| {}).unwrap();
        wait_for(|| r.activity(0).is_ok_and(|a| a.baseline)).await;
        r.set_pause(true).unwrap();
        r.shutdown().await;
        drop(r);
        let mut store = Store::open(&path).unwrap();
        // Model an interrupted recovery, whose original boundary must survive.
        store.activity_recovery(now()).unwrap();
        let identity = store.identity().unwrap();
        let revision = store.revision().unwrap();
        let poll = store.due("activity:notifications").unwrap();
        let boundary: i64 = store
            .conn
            .query_row("SELECT boundary FROM activity_cycle", [], |r| r.get(0))
            .unwrap();
        drop(store);
        let before = std::fs::read(&path).unwrap();
        let count = reads(d.path());
        let healthy = Arc::new(AtomicBool::new(false));
        let probe_health = healthy.clone();
        let check: StorageCheck = Arc::new(move |_| {
            if probe_health.load(Ordering::SeqCst) {
                Ok(())
            } else {
                Err(Error::StorageUnavailable(
                    "Fixture: below free-space threshold".into(),
                ))
            }
        });
        let r = Runtime::start_checked(path.clone(), |_| {}, check).unwrap();
        let s = r.status().unwrap();
        assert!(s.storage_stopped && s.locked && s.settings.paused);
        assert_eq!(s.identity, identity);
        assert_eq!(s.revision, revision);
        assert!(s.storage.managed_bytes >= before.len() as u64);
        assert!(
            r.page(0).is_err(),
            "storage protection cannot bypass startup identity lock"
        );
        assert!(r.refresh().is_err());
        assert!(r.set_pause(false).is_err());
        tokio::time::sleep(Duration::from_millis(1200)).await;
        assert_eq!(reads(d.path()), count);
        assert_eq!(std::fs::read(&path).unwrap(), before);
        healthy.store(true, Ordering::SeqCst);
        r.lock().unwrap().storage_check_at = 0;
        wait_for(|| !r.status().unwrap().storage_stopped).await;
        assert!(r.status().unwrap().settings.paused);
        assert_eq!(
            reads(d.path()),
            count,
            "repair does not bypass Pause for identity reads"
        );
        assert_eq!(
            r.lock()
                .unwrap()
                .store
                .due("activity:notifications")
                .unwrap(),
            poll
        );
        assert_eq!(
            r.lock()
                .unwrap()
                .store
                .conn
                .query_row("SELECT boundary FROM activity_cycle", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            boundary
        );
        r.refresh().unwrap();
        wait_for(|| !r.status().unwrap().locked && r.page(0).is_ok_and(|p| p.complete)).await;
        assert!(r.status().unwrap().settings.paused);
        assert_eq!(r.activity(0).unwrap().next_poll_at, poll);
        r.shutdown().await;
    }
    #[tokio::test]
    async fn partial_startup_cleanup_stays_locked_and_paused_until_repaired() {
        let _worker_fixture = WORKER_FIXTURES.lock().await;
        let (d, path) = fixture_store();
        let first = Runtime::start(path.clone(), |_| {}).unwrap();
        wait_for(|| first.activity(0).is_ok_and(|a| a.baseline)).await;
        first.set_pause(true).unwrap();
        wait_for(|| !first.status().unwrap().busy).await;
        first.shutdown().await;
        let stopped_reads = reads(d.path());
        let sql = rusqlite::Connection::open(&path).unwrap();
        for n in 0..129 {
            sql.execute("INSERT INTO stage VALUES(?,'{}')", [n.to_string()])
                .unwrap();
        }
        sql.execute_batch("INSERT INTO source_stage VALUES('authored','unfinished','{}');
            UPDATE activity_cycle SET mode='recovery',boundary=123;
            UPDATE due SET at=4070908800 WHERE key='activity:notifications';
            CREATE TRIGGER fail_partial_cleanup BEFORE DELETE ON source_stage BEGIN SELECT RAISE(ABORT,'fixture late cleanup fault'); END;").unwrap();
        let ledger_counts = || {
            [
                "seen",
                "dedup",
                "watch_locator",
                "activity_intent",
                "intent_member",
                "alert_attempt",
            ]
            .map(|table| {
                sql.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| {
                    r.get::<_, i64>(0)
                })
                .unwrap()
            })
        };
        let before = ledger_counts();
        let r = Runtime::start(path.clone(), |_| {}).unwrap();
        assert!(r.status().unwrap().storage_stopped);
        assert!(r.status().unwrap().settings.paused);
        assert!(r.status().unwrap().locked);
        assert!(r.page(0).is_err());
        assert!(r.refresh().is_err());
        // The first 128-row transaction committed; the failing second batch
        // rolled back its companion final stage row.
        assert_eq!(
            sql.query_row("SELECT COUNT(*) FROM stage", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            1
        );
        r.lock().unwrap().storage_check_at = 0;
        wait_for(|| r.lock().unwrap().storage_check_at != 0).await;
        assert!(r.status().unwrap().storage_stopped);
        assert_eq!(reads(d.path()), stopped_reads);
        assert_eq!(
            sql.query_row("SELECT COUNT(*) FROM stage", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            1
        );
        sql.execute_batch("DROP TRIGGER fail_partial_cleanup")
            .unwrap();
        r.lock().unwrap().storage_check_at = 0;
        wait_for(|| !r.status().unwrap().storage_stopped).await;
        for table in ["stage", "source_stage"] {
            assert_eq!(
                sql.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r
                    .get::<_, i64>(0))
                    .unwrap(),
                0
            );
        }
        assert_eq!(ledger_counts(), before);
        assert!(r.status().unwrap().settings.paused);
        assert!(r.status().unwrap().locked);
        assert_eq!(reads(d.path()), stopped_reads);
        assert_eq!(
            sql.query_row("SELECT boundary FROM activity_cycle", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            123
        );
        assert_eq!(
            sql.query_row(
                "SELECT at FROM due WHERE key='activity:notifications'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            4070908800
        );
        r.refresh().unwrap();
        wait_for(|| !r.status().unwrap().locked && r.page(0).is_ok_and(|p| p.complete)).await;
        assert!(r.status().unwrap().settings.paused);
        assert_eq!(r.activity(0).unwrap().next_poll_at, 4070908800);
        assert_eq!(
            sql.query_row(
                "SELECT COUNT(*) FROM activity_intent WHERE state='pending'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            0
        );
        r.shutdown().await;
    }
    #[tokio::test]
    async fn partial_recovery_initialization_blocks_reads_and_repairs_without_resuming() {
        let _worker_fixture = WORKER_FIXTURES.lock().await;
        let (d, path) = fixture_store();
        let first = Runtime::start(path.clone(), |_| {}).unwrap();
        wait_for(|| first.activity(0).is_ok_and(|a| a.baseline)).await;
        first.set_pause(true).unwrap();
        wait_for(|| !first.status().unwrap().busy).await;
        first.shutdown().await;
        let stopped_reads = reads(d.path());
        let sql = rusqlite::Connection::open(&path).unwrap();
        sql.execute_batch(
            "DELETE FROM activity_target;
            UPDATE activity_cycle SET mode='normal',boundary=1;
            UPDATE due SET at=4070908800 WHERE key='activity:notifications';",
        )
        .unwrap();
        for n in 0..257 {
            sql.execute("INSERT INTO activity_target VALUES('a/b',?,9,1)", [n])
                .unwrap();
        }
        sql.execute_batch("CREATE TRIGGER fail_recovery BEFORE UPDATE ON activity_target WHEN OLD.number=256 BEGIN SELECT RAISE(ABORT,'fixture late recovery fault'); END;").unwrap();
        let r = Runtime::start(path.clone(), |_| {}).unwrap();
        let boundary: i64 = sql
            .query_row("SELECT boundary FROM activity_cycle", [], |r| r.get(0))
            .unwrap();
        assert!(boundary > 1);
        assert!(r.status().unwrap().storage_stopped);
        assert!(r.status().unwrap().locked);
        assert!(r.status().unwrap().settings.paused);
        assert!(r.page(0).is_err());
        assert!(r.refresh().is_err());
        assert_eq!(
            sql.query_row(
                "SELECT COUNT(*) FROM activity_target WHERE done=1",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            1
        );
        r.lock().unwrap().storage_check_at = 0;
        wait_for(|| r.lock().unwrap().storage_check_at != 0).await;
        assert!(r.status().unwrap().storage_stopped);
        assert_eq!(reads(d.path()), stopped_reads);
        sql.execute_batch("DROP TRIGGER fail_recovery").unwrap();
        r.lock().unwrap().storage_check_at = 0;
        wait_for(|| !r.status().unwrap().storage_stopped).await;
        assert!(r.status().unwrap().settings.paused);
        assert!(r.status().unwrap().locked);
        assert_eq!(reads(d.path()), stopped_reads);
        assert_eq!(
            sql.query_row(
                "SELECT COUNT(*) FROM activity_target WHERE done!=0 OR due!=0",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            0
        );
        assert_eq!(
            sql.query_row("SELECT boundary FROM activity_cycle", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            boundary
        );
        assert_eq!(
            sql.query_row(
                "SELECT COUNT(*) FROM activity_intent WHERE state='pending'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            0
        );
        r.refresh().unwrap();
        wait_for(|| !r.status().unwrap().locked && r.page(0).is_ok_and(|p| p.complete)).await;
        assert!(r.status().unwrap().settings.paused);
        assert_eq!(r.activity(0).unwrap().next_poll_at, 4070908800);
        r.shutdown().await;
    }
    #[tokio::test]
    async fn startup_cleanup_failure_opens_protected_cache_until_cleanup_and_probe_succeed() {
        let _worker_fixture = WORKER_FIXTURES.lock().await;
        let (_d, path) = fixture_store();
        let sql = rusqlite::Connection::open(&path).unwrap();
        sql.execute_batch("INSERT INTO stage VALUES('incomplete','{}');
            CREATE TRIGGER fail_cleanup BEFORE DELETE ON stage BEGIN SELECT RAISE(ABORT,'fixture cleanup fault'); END;").unwrap();
        let r = Runtime::start(path.clone(), |_| {}).unwrap();
        assert!(r.status().unwrap().storage_stopped);
        assert!(r.lock().unwrap().startup_deferred);
        assert_eq!(
            sql.query_row("SELECT COUNT(*) FROM stage", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            1
        );
        r.lock().unwrap().storage_check_at = 0;
        tokio::time::sleep(Duration::from_millis(1200)).await;
        assert!(r.status().unwrap().storage_stopped);
        sql.execute_batch("DROP TRIGGER fail_cleanup").unwrap();
        r.lock().unwrap().storage_check_at = 0;
        wait_for(|| !r.status().unwrap().storage_stopped && !r.status().unwrap().locked).await;
        assert_eq!(
            sql.query_row("SELECT COUNT(*) FROM stage", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            0
        );
        r.shutdown().await;
    }
    #[test]
    fn queue_bounds_coalescing_and_fairness() {
        let mut q = Queue::default();
        for n in 0..128 {
            assert!(q.push(Work {
                ticket: None,
                job: Job::Overview("o/r".into(), n),
                manual: true,
                foreground: n != 127
            }));
        }
        assert!(!q.push(Work {
            ticket: None,
            job: Job::Bootstrap,
            manual: false,
            foreground: false
        }));
        assert!(q.push(Work {
            ticket: None,
            job: Job::Overview("o/r".into(), 0),
            manual: true,
            foreground: true
        }));
        for _ in 0..3 {
            assert!(q.pop().unwrap().foreground)
        }
        assert!(!q.pop().unwrap().foreground);
        assert_eq!(q.jobs.len(), 124);
    }
}
