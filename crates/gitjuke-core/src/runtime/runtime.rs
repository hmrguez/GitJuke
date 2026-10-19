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
    Logs(crate::logs::LogJob),
    Query(crate::saved_queries::QueryJob),
    Conversation(crate::conversation::ConversationJob),
    Changes(crate::changes::ChangesJob),
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
            Self::Logs(p) => format!("log:{}", p.seq),
            Self::Query(j) => j.key(),
            Self::Conversation(p) => p.key(),
            Self::Changes(p) => p.key(),
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
    pub(crate) jobs: VecDeque<Work>,
    foreground_run: usize,
}
impl Queue {
    pub(crate) fn discard_queries(&mut self) {
        self.jobs.retain(|w| !matches!(w.job, Job::Query(_)));
    }
    pub(crate) fn discard_conversation(&mut self) {
        self.jobs.retain(|w| !matches!(w.job, Job::Conversation(_)));
    }
    pub(crate) fn discard_changes(&mut self) {
        self.jobs.retain(|w| !matches!(w.job, Job::Changes(_)));
    }
    pub(crate) fn push(&mut self, w: Work) -> bool {
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
    pub(crate) conversation_seq: u64,
    pub(crate) conversation_running: bool,
    pub(crate) log_slot: Option<crate::logs::LogSlot>,
    pub(crate) log_seq: u64,
    pub(crate) log_running: bool,
    pub(crate) changes_seq: u64,
    pub(crate) changes_running: bool,
    pub(crate) active_query: Option<(i64, u64)>,
    last_tick: u64,
    pub(crate) storage_stopped: bool,
    storage_check_at: u64,
    startup_deferred: bool,
    storage_check: StorageCheck,
}
type StorageCheck = Arc<dyn Fn(&std::path::Path) -> Result<()> + Send + Sync>;

#[derive(Clone)]
pub struct Runtime {
    pub(crate) alert_probe: Arc<Mutex<Option<crate::runtime_alerts::Probe>>>,
    pub(crate) alert_sink: Arc<Mutex<Option<Arc<dyn crate::alerts::AlertSink>>>>,
    pub(crate) inner: Arc<Mutex<Inner>>,
    notify: Arc<Notify>,
    pub(crate) stop: CancellationToken,
    finished: CancellationToken,
    pub(crate) on_revision: Arc<dyn Fn(u64) + Send + Sync>,
}
impl Runtime {
    pub fn start(path: PathBuf, on_revision: impl Fn(u64) + Send + Sync + 'static) -> Result<Self> {
        Self::start_checked(path, on_revision, Arc::new(storage_healthy))
    }
    /// Initialize hidden-reader membership before spawning the monitor worker.
    pub fn start_background(
        path: PathBuf,
        on_revision: impl Fn(u64) + Send + Sync + 'static,
    ) -> Result<Self> {
        Self::start_mode(path, on_revision, Arc::new(storage_healthy), false)
    }
    fn start_checked(
        path: PathBuf,
        on_revision: impl Fn(u64) + Send + Sync + 'static,
        storage_check: StorageCheck,
    ) -> Result<Self> {
        Self::start_mode(path, on_revision, storage_check, true)
    }
    fn start_mode(
        path: PathBuf,
        on_revision: impl Fn(u64) + Send + Sync + 'static,
        storage_check: StorageCheck,
        reader_open: bool,
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
            alert_probe: Arc::new(Mutex::new(None)),
            alert_sink: Arc::new(Mutex::new(None)),
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
                reader_open,
                reader_visible: reader_open,
                batch_size: 10,
                context_size: 25,
                path,
                conversation_seq: 0,
                conversation_running: false,
                log_slot: None,
                log_seq: 0,
                log_running: false,
                changes_seq: 0,
                changes_running: false,
                active_query: None,
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
        Self::cancel_logs_locked(&mut i);
        Self::cancel_changes_locked(&mut i);
        Self::cancel_conversation_locked(&mut i);
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
        Self::cancel_logs_locked(&mut i);
        Self::cancel_changes_locked(&mut i);
        Self::cancel_conversation_locked(&mut i);
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
            Self::cancel_logs_locked(&mut i);
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
                Self::store_operation(&mut i, |s| {
                    s.set_due("saved-query", 0)?;
                    s.set_due("discovery", 0)
                })?;
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
        Self::cancel_logs_locked(&mut i);
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
        Self::cancel_logs_locked(&mut i);
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
    /// Stop eligibility and cancel active reads before the desktop schedules its async reap.
    pub fn request_shutdown(&self) {
        self.stop.cancel();
        self.notify.notify_one();
    }
    pub(crate) fn work_token(&self) -> CancellationToken {
        self.stop.child_token()
    }
    pub async fn shutdown(&self) {
        self.request_shutdown();
        if let Ok(mut i) = self.lock() {
            Self::cancel_logs_locked(&mut i);
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
            let result = self.dispatch_alert().await;
            if let Err(err) = self.protect_storage(result)
                && let Ok(mut i) = self.lock()
                && !i.storage_stopped
            {
                i.health = err.to_string();
                let _ = Self::store_operation(&mut i, |s| s.record_failure("core", now()));
            }
            let work = self.next();
            match work {
                Ok(Some((work, transport, token, generation))) => {
                    if self.stop.is_cancelled() {
                        break;
                    }
                    let mut result = self.execute(&work, &transport, &token, generation).await;
                    if let Ok(mut i) = self.lock() {
                        i.busy = false;
                        i.log_running = false;
                        i.changes_running = false;
                        i.conversation_running = false;
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
                                if token.is_cancelled()
                                    && matches!(work.job, Job::Changes(_) | Job::Conversation(_))
                                    && let Some(seq) = work.ticket
                                {
                                    i.store.ack(seq)?;
                                }
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
                                    } else if let Job::Query(_) = &work.job {
                                        if let Some(seq) = work.ticket {
                                            i.store.ack(seq)?;
                                        }
                                    } else if matches!(
                                        &work.job,
                                        Job::Changes(_) | Job::Conversation(_)
                                    ) {
                                        if let Some(seq) = work.ticket {
                                            i.store.ack(seq)?;
                                        }
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
        Self::cancel_logs_locked(i);
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
                i.store.resume_changes_cleanup()?;
                i.store.resume_conversation_cleanup()?;
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
        if self.stop.is_cancelled() {
            return Ok(None);
        }
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
        if !paused
            && !i.locked
            && i.reader_open
            && let Some((id, version)) = i.active_query
            && i.store.due("saved-query")? <= now()
        {
            Self::queue_query(&mut i, id, version, 1, false)?;
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
        Self::maintain_logs(&mut i)?;
        // A memory-only log request waits through core quota/backoff while other resources run.
        let deferred_log = if !i.store.budget_eligible("core", now())? {
            i.queue
                .jobs
                .iter()
                .position(|w| matches!(w.job, Job::Logs(_)))
                .and_then(|n| i.queue.jobs.remove(n))
        } else {
            None
        };
        let work = i.queue.pop();
        if let Some(log) = deferred_log {
            i.queue.push(log);
        }
        let Some(work) = work else {
            return Ok(None);
        };
        if paused && !work.manual {
            return Ok(None);
        }
        (i.storage_check)(i.path.parent().unwrap())?;
        i.log_running = matches!(work.job, Job::Logs(_));
        i.changes_running = matches!(work.job, Job::Changes(_));
        i.conversation_running = matches!(work.job, Job::Conversation(_));
        i.busy = true;
        i.current = self.work_token();
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
        let log_read = matches!(&op, ReadOp::ActionsLog { .. });
        let notification_url = match &op {
            ReadOp::Activity(read) if read.url.starts_with("notifications?") => {
                Some(read.url.clone())
            }
            _ => None,
        };
        let resource = match &op {
            ReadOp::Graph(_) | ReadOp::Conversation(_) => "graphql",
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
            Self::cancel_logs_locked(&mut i);
            i.locked = true;
            i.store.invalidate()?;
            i.queue.jobs.clear();
            return Err(Error::Safe(
                "Stored gh authentication was lost. Restore the original account externally."
                    .into(),
            ));
        }
        let payload = output.stdout.len();
        let r = if log_read {
            Response::parse_log(output)?
        } else {
            Response::parse(output)?
        };
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
            Self::cancel_logs_locked(&mut i);
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
            Self::cancel_logs_locked(&mut i);
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
        if let Job::Logs(job) = &w.job {
            return self.execute_logs(t, c, generation, job).await;
        }
        if let Job::Query(job) = &w.job {
            return self.execute_query(t, c, generation, job, w.manual).await;
        }
        if let Job::Conversation(job) = &w.job {
            return self.execute_conversation(t, c, generation, job).await;
        }
        if let Job::Changes(job) = &w.job {
            return self.execute_changes(t, c, generation, job).await;
        }
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
            Job::Logs(_)
            | Job::Query(_)
            | Job::Conversation(_)
            | Job::Changes(_)
            | Job::Activity(_)
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
            Job::Logs(_)
            | Job::Query(_)
            | Job::Conversation(_)
            | Job::Changes(_)
            | Job::Activity(_)
            | Job::Bootstrap
            | Job::Discover(_)
            | Job::Heads(_)
            | Job::Contexts(_) => {}
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;

#[cfg(test)]
#[path = "runtime_lifecycle_tests.rs"]
mod lifecycle_tests;
