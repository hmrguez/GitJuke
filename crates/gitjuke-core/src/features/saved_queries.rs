//! Local configuration and version-bound, disposable Search pages. No watch enrollment.
use crate::{
    Error, Result, github,
    model::*,
    now,
    runtime::{Inner, Job, Runtime},
    store::{Store, bump},
    transport::{ReadOp, Transport},
};
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SavedQuery {
    pub id: i64,
    pub name: String,
    pub search: String,
    pub version: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueryList {
    pub items: Vec<SavedQuery>,
    pub next_offset: Option<usize>,
    pub revision: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct QueryJob {
    pub id: i64,
    pub version: u64,
    pub run: u64,
    pub page: u32,
}
impl QueryJob {
    pub fn key(&self) -> String {
        format!("query:{}:{}:{}", self.id, self.version, self.page)
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueryResults {
    pub items: Vec<PullRequest>,
    pub total: u64,
    pub next_page: Option<u32>,
    pub source_complete: bool,
    pub run: u64,
    pub generation: u64,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QueryPage {
    pub query: SavedQuery,
    pub items: Vec<PullRequest>,
    pub stacks: Vec<PullRequestStack>,
    pub total: Option<u64>,
    pub cached_count: usize,
    pub next_page: Option<u32>,
    pub fetched_at: Option<u64>,
    pub complete: bool,
    pub error: Option<String>,
    pub revision: u64,
}
fn invalid() -> Error {
    Error::Safe("Query no longer exists or was edited. Reopen its current version.".into())
}
impl Store {
    pub fn saved_query(&self, id: i64) -> Result<SavedQuery> {
        self.conn
            .query_row(
                "SELECT id,name,search,version FROM saved_query WHERE id=?",
                [id],
                |r| {
                    Ok(SavedQuery {
                        id: r.get(0)?,
                        name: r.get(1)?,
                        search: r.get(2)?,
                        version: r.get::<_, i64>(3)? as u64,
                    })
                },
            )
            .optional()?
            .ok_or_else(invalid)
    }
    pub fn saved_queries(&self, offset: usize) -> Result<QueryList> {
        let tx = self.conn.unchecked_transaction()?;
        let mut q = self.conn.prepare(
            "SELECT id,name,search,version FROM saved_query ORDER BY id LIMIT 101 OFFSET ?",
        )?;
        let mut items = q
            .query_map([i64::try_from(offset).unwrap_or(i64::MAX)], |r| {
                Ok(SavedQuery {
                    id: r.get(0)?,
                    name: r.get(1)?,
                    search: r.get(2)?,
                    version: r.get::<_, i64>(3)? as u64,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let more = items.len() > 100;
        items.truncate(100);
        let revision = self.revision()?;
        drop(q);
        tx.commit()?;
        Ok(QueryList {
            items,
            next_offset: more.then_some(offset.saturating_add(100)),
            revision,
        })
    }
    pub fn save_query(
        &mut self,
        id: Option<i64>,
        version: Option<u64>,
        name: &str,
        search: &str,
    ) -> Result<SavedQuery> {
        let name = name.trim();
        let search = search.trim();
        if name.is_empty()
            || name.len() > 160
            || search.is_empty()
            || search.len() > 2048
            || name.chars().any(char::is_control)
            || search.chars().any(char::is_control)
        {
            return Err(Error::Safe("Use a name up to 160 UTF-8 bytes and a search up to 2048 bytes, without control characters.".into()));
        }
        let tx = self.conn.transaction()?;
        let id = if let Some(id) = id {
            if tx.execute(
                "UPDATE saved_query SET name=?,search=?,version=version+1,limited=0 WHERE id=? AND version=?",
                params![name, search, id, version.and_then(|v| i64::try_from(v).ok())],
            )? != 1
            {
                return Err(invalid());
            }
            id
        } else {
            tx.execute(
                "INSERT INTO saved_query(name,search,version) VALUES(?,?,1)",
                params![name, search],
            )?;
            tx.last_insert_rowid()
        };
        bump(&tx)?;
        tx.commit()?;
        self.saved_query(id)
    }
    pub fn delete_query(&mut self, id: i64, version: u64) -> Result<()> {
        let tx = self.conn.transaction()?;
        if tx.execute(
            "DELETE FROM saved_query WHERE id=? AND version=?",
            params![id, i64::try_from(version).map_err(|_| invalid())?],
        )? != 1
        {
            return Err(invalid());
        }
        tx.execute(
            "DELETE FROM due WHERE key=?",
            [format!("query-denied:{id}")],
        )?;
        bump(&tx)?;
        tx.commit()?;
        Ok(())
    }
    fn query_current(&self, j: &QueryJob) -> Result<bool> {
        match self.saved_query(j.id) {
            Ok(q) => Ok(
                q.version == j.version && j.run > self.due(&format!("query-denied:{}", j.id))?
            ),
            Err(Error::Safe(_)) => Ok(false),
            Err(e) => Err(e),
        }
    }
    pub fn query_page(&mut self, id: i64, version: u64, page: u32) -> Result<QueryPage> {
        self.conn.execute_batch("SAVEPOINT saved_query_read")?;
        let result = self.query_page_in(id, version, page);
        match result {
            Ok(page) => {
                if let Err(err) = self.conn.execute_batch("RELEASE saved_query_read") {
                    let _ = self
                        .conn
                        .execute_batch("ROLLBACK TO saved_query_read; RELEASE saved_query_read");
                    return Err(err.into());
                }
                Ok(page)
            }
            Err(err) => {
                let _ = self
                    .conn
                    .execute_batch("ROLLBACK TO saved_query_read; RELEASE saved_query_read");
                Err(err)
            }
        }
    }
    fn query_page_in(&mut self, id: i64, version: u64, page: u32) -> Result<QueryPage> {
        if !(1..=10).contains(&page) {
            return Err(Error::Safe(
                "Search exposes at most ten 100-result pages.".into(),
            ));
        }
        let query = self.saved_query(id)?;
        if query.version != version {
            return Err(invalid());
        }
        let generation = self.generation()?;
        let key = |p| {
            QueryJob {
                id,
                version,
                page: p,
                run: 0,
            }
            .key()
        };
        // At most ten bounded cached pages. Never combine different Search traversals.
        let denied_run = self.due(&format!("query-denied:{id}"))?;
        let first = self.get::<QueryResults>(&key(1))?;
        let run = first
            .data
            .as_ref()
            .filter(|p| p.generation == generation && p.run > denied_run)
            .map(|p| p.run);
        let mut ids = std::collections::HashSet::new();
        let mut cached_count = 0;
        let mut all_complete = true;
        let mut selected = None;
        for n in 1..=10 {
            let s = if n == 1 {
                first.clone()
            } else {
                self.get::<QueryResults>(&key(n))?
            };
            if n == page {
                selected = Some(s.clone());
            }
            if let Some(p) = &s.data
                && Some(p.run) == run
                && p.generation == generation
            {
                for item in &p.items {
                    if !ids.insert(item.id.clone()) {
                        all_complete = false;
                    }
                }
                cached_count = ids.len();
                all_complete &= first
                    .data
                    .as_ref()
                    .is_some_and(|first| first.total == p.total)
                    && p.source_complete;
            }
        }
        let s = selected.unwrap();
        let data = s
            .data
            .filter(|p| Some(p.run) == run && p.generation == generation);
        let total = first
            .data
            .as_ref()
            .filter(|p| p.generation == generation && p.run > denied_run)
            .map(|p| p.total);
        let complete = total.is_some_and(|t| t <= 1000 && cached_count as u64 == t)
            && all_complete
            && first.complete;
        let error=s.error.or(first.error).or_else(||(denied_run>0&&run.is_none()).then(||"Query access unavailable. Refresh after restoring access.".into())).or_else(||data.as_ref().filter(|p|!p.source_complete).map(|_|"GitHub returned a partial or Search-limited set. Counts are not a complete membership claim.".into()));
        let items = data.as_ref().map(|p| p.items.clone()).unwrap_or_default();
        let bytes = serde_json::to_vec(&items)?.len();
        let stacks = self.list_stacks(&items, (900 * 1024_usize).saturating_sub(bytes))?;
        Ok(QueryPage {
            query,
            items,
            stacks,
            total,
            cached_count,
            next_page: data.as_ref().and_then(|p| p.next_page),
            fetched_at: if data.is_some() { s.fetched_at } else { None },
            complete,
            error,
            revision: self.revision()?,
        })
    }
}
impl Runtime {
    pub fn saved_queries(&self, offset: usize) -> Result<QueryList> {
        self.lock()?.store.saved_queries(offset)
    }
    pub fn save_query(
        &self,
        id: Option<i64>,
        version: Option<u64>,
        name: String,
        search: String,
    ) -> Result<SavedQuery> {
        let mut i = self.lock()?;
        let q = Self::store_operation(&mut i, |s| s.save_query(id, version, &name, &search))?;
        if i.active_query.is_some_and(|a| Some(a.0) == id) {
            i.active_query = None;
        }
        let revision = i.store.revision()?;
        drop(i);
        (self.on_revision)(revision);
        Ok(q)
    }
    pub fn delete_query(&self, id: i64, version: u64) -> Result<()> {
        let mut i = self.lock()?;
        Self::store_operation(&mut i, |s| s.delete_query(id, version))?;
        if i.active_query.is_some_and(|a| a.0 == id) {
            i.active_query = None;
        }
        let revision = i.store.revision()?;
        drop(i);
        (self.on_revision)(revision);
        Ok(())
    }
    pub fn query_page(&self, id: i64, version: u64, page: u32) -> Result<QueryPage> {
        let mut i = self.lock()?;
        if i.locked {
            return Err(Error::Safe(i.health.clone()));
        }
        Self::cached_operation(&mut i, |s| s.query_page(id, version, page))
    }
    pub async fn prepared_query_page(&self, id: i64, version: u64, page: u32) -> Result<QueryPage> {
        let mut cached = self.query_page(id, version, page)?;
        loop {
            let preparation = self.prepare_page_stacks(&cached.items).await;
            let mut result = self.query_page(id, version, page)?;
            if preparation.is_ok()
                && result.items.iter().any(|item| {
                    !cached.items.iter().any(|previous| {
                        previous.repo == item.repo && previous.number == item.number
                    })
                })
            {
                cached = result;
                continue;
            }
            match preparation {
                Err(Error::Safe(message)) => result.error = Some(message),
                other => other?,
            }
            return Ok(result);
        }
    }
    pub fn activate_query(&self, id: Option<i64>, version: Option<u64>) -> Result<()> {
        let mut i = self.lock()?;
        i.active_query = None;
        if let Some(id) = id {
            let q = i.store.saved_query(id)?;
            if Some(q.version) != version {
                return Err(invalid());
            }
            i.active_query = Some((id, q.version));
        }
        drop(i);
        self.notify_sync();
        Ok(())
    }
    pub fn request_query(&self, id: i64, version: u64, page: u32) -> Result<()> {
        let mut i = self.lock()?;
        Self::queue_query(&mut i, id, version, page, true)?;
        drop(i);
        self.notify_sync();
        Ok(())
    }
    pub(crate) fn queue_query(
        i: &mut Inner,
        id: i64,
        version: u64,
        page: u32,
        manual: bool,
    ) -> Result<()> {
        if i.locked {
            return Err(Error::Safe(i.health.clone()));
        }
        Self::require_remote_storage(i)?;
        let q = i.store.saved_query(id)?;
        if q.version != version || !(1..=10).contains(&page) {
            return Err(invalid());
        }
        let limited: bool =
            i.store
                .conn
                .query_row("SELECT limited FROM saved_query WHERE id=?", [id], |r| {
                    r.get(0)
                })?;
        if !manual && limited {
            i.store.set_due("saved-query", now() + 60)?;
            return Ok(());
        }
        if manual && limited {
            i.store
                .conn
                .execute("UPDATE saved_query SET limited=0 WHERE id=?", [id])?;
        }
        let first = i.store.get::<QueryResults>(
            &QueryJob {
                id,
                version,
                page: 1,
                run: 0,
            }
            .key(),
        )?;
        let run = if page == 1 {
            i.store.revision()?.saturating_add(1)
        } else {
            let generation = i.store.generation()?;
            let denied_run = i.store.due(&format!("query-denied:{id}"))?;
            let p = first
                .data
                .filter(|p| p.generation == generation && p.run > denied_run)
                .ok_or_else(|| Error::Safe("Refresh the first query page before paging.".into()))?;
            p.run
        };
        let j = QueryJob {
            id,
            version,
            run,
            page,
        };
        i.queue.discard_queries();
        Self::store_operation(i, |s| {
            s.enqueue(
                "saved-query:active",
                &Job::Query(j.clone()),
                manual,
                manual,
                0,
                true,
            )?;
            s.error(
                &j.key(),
                "Queued — waiting for the serialized reader and GitHub deadlines.",
            )?;
            s.set_due("saved-query", now() + 60)
        })?;
        Ok(())
    }
    pub(crate) async fn execute_query(
        &self,
        t: &Transport,
        c: &CancellationToken,
        generation: u64,
        j: &QueryJob,
        manual: bool,
    ) -> Result<()> {
        {
            let i = self.lock()?;
            if (!manual && (!i.reader_open || i.active_query != Some((j.id, j.version))))
                || !i.store.query_current(j)?
            {
                return Ok(());
            }
        }
        let before = self.identity(t, c, generation).await?;
        let q = self.lock()?.store.saved_query(j.id)?;
        let outcome = async {
            let r = self
                .response(
                    t,
                    ReadOp::Search {
                        query: format!("{} is:pr", q.search),
                        page: j.page,
                        size: 100,
                    },
                    c,
                )
                .await?;
            let after = self.identity(t, c, generation).await?;
            if before.id != after.id {
                return Err(Error::Safe("Identity changed during query read.".into()));
            }
            let p = github::parse_search(&r)?;
            if p.prs.len() > 100 {
                return Err(Error::Safe("Invalid Search page.".into()));
            }
            let reached = u64::from(j.page - 1) * 100 + p.prs.len() as u64;
            let source_complete =
                !p.incomplete && p.total <= 1000 && (reached >= p.total || p.prs.len() == 100);
            Ok(QueryResults {
                items: p.prs,
                total: p.total,
                next_page: (u64::from(j.page) * 100 < p.total && j.page < 10).then_some(j.page + 1),
                source_complete,
                run: j.run,
                generation,
            })
        }
        .await;
        let mut i = self.lock()?;
        if c.is_cancelled()
            || i.locked
            || i.store.generation()? != generation
            || !i.store.query_current(j)?
        {
            return Ok(());
        }
        // Only one process runs; a continuation must still belong to the visible first page.
        if j.page > 1 {
            let first = i.store.get::<QueryResults>(
                &QueryJob {
                    page: 1,
                    ..j.clone()
                }
                .key(),
            )?;
            if !first
                .data
                .is_some_and(|p| p.run == j.run && p.generation == generation)
            {
                return Ok(());
            }
        }
        match outcome {
            Ok(p) => {
                if serde_json::to_vec(&p)?.len() > 900 * 1024 {
                    i.store
                        .conn
                        .execute("UPDATE saved_query SET limited=1 WHERE id=?", [j.id])?;
                    i.store.error(
                        &j.key(),
                        "Search page exceeds the local IPC budget; narrow the query.",
                    )?;
                    return Ok(());
                }
                i.store.put(&j.key(), &p, generation)?;
            }
            Err(e) => {
                if matches!(e, Error::Storage(_) | Error::StorageUnavailable(_)) {
                    return Err(e);
                }
                if e.to_string().contains("output limit") {
                    i.store
                        .conn
                        .execute("UPDATE saved_query SET limited=1 WHERE id=?", [j.id])?;
                }
                if matches!(e, Error::Denied) {
                    let cutoff = i.store.revision()?.max(j.run);
                    i.store.set_due(&format!("query-denied:{}", j.id), cutoff)?;
                }
                i.store.error(&j.key(), &e.to_string())?;
            }
        }
        Ok(())
    }
}
