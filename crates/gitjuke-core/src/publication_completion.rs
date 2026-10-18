//! Completion reads bounded native candidate windows in one SQLite snapshot.
//! Readiness is recomputed; no partial cursor can certify a completed recovery.
use crate::{Result, retention::CONTENT_AGE};
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::Value;
const WINDOW: usize = 128;

pub(crate) fn published_count(c: &Connection, pr: &str, sha: &str) -> Result<i64> {
    let mut q =
        c.prepare("SELECT contexts FROM main.shared_check_lru WHERE pr_id=? AND commit_sha=?")?;
    let out = q
        .query_row(params![pr, sha], |r| r.get(0))
        .optional()?
        .unwrap_or(0);
    observe(&q, 1);
    Ok(out)
}
fn window(
    c: &Connection,
    table: &str,
    columns: &str,
    after: Option<i64>,
) -> Result<Vec<(i64, Value)>> {
    let mut q=c.prepare(&format!("SELECT rowid,json_array({columns}) FROM main.{table} WHERE rowid{}? ORDER BY rowid LIMIT {WINDOW}",if after.is_some(){">"}else{">="}))?;
    let data = q
        .query_map([after.unwrap_or(i64::MIN)], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    observe(&q, data.len());
    #[cfg(test)]
    point(table, after);
    data.into_iter()
        .map(|(id, s)| Ok((id, serde_json::from_str(&s)?)))
        .collect()
}
fn every(
    c: &Connection,
    table: &str,
    cols: &str,
    mut valid: impl FnMut(&Value) -> Result<bool>,
) -> Result<bool> {
    let mut after = None;
    loop {
        let rows = window(c, table, cols, after)?;
        for (id, r) in &rows {
            if !valid(r)? {
                return Ok(false);
            }
            after = Some(*id);
        }
        if rows.len() < WINDOW {
            return Ok(true);
        }
    }
}
fn exists(c: &Connection, sql: &str, args: impl rusqlite::Params) -> Result<bool> {
    let mut q = c.prepare(sql)?;
    let out = q.query_row(args, |r| r.get(0))?;
    observe(&q, 1);
    Ok(out)
}
fn prefix(s: &str, p: &str) -> bool {
    s.get(..p.len()).is_some_and(|a| a.eq_ignore_ascii_case(p))
}
pub(crate) fn ready(c: &Connection, time: u64) -> Result<bool> {
    if c.is_autocommit() {
        let tx = c.unchecked_transaction()?;
        let out = ready_in(&tx, time)?;
        tx.commit()?;
        Ok(out)
    } else {
        ready_in(c, time)
    }
}
fn ready_in(c: &Connection, time: u64) -> Result<bool> {
    if !exists(
        c,
        "SELECT EXISTS(SELECT 1 FROM main.activity_cycle WHERE id=1 AND mode='recovery' AND notifications_done=1)",
        [],
    )? {
        return Ok(false);
    }
    if !every(c, "source_restart", "source", |r| {
        let source = r[0].as_str().unwrap();
        if matches!(source, "authored" | "direct" | "teams") {
            return Ok(false);
        }
        if let Some(id) = source.strip_prefix("team:") {
            return Ok(!exists(
                c,
                "SELECT EXISTS(SELECT 1 FROM main.teams WHERE id=?)",
                [id],
            )?);
        }
        Ok(true)
    })? {
        return Ok(false);
    }
    if !every(c, "activity_target", "done", |r| Ok(r[0] != 0))? {
        return Ok(false);
    }
    if !every(c, "pending", "key", |r| {
        Ok(!prefix(r[0].as_str().unwrap(), "discover:")
            && !prefix(r[0].as_str().unwrap(), "checks:"))
    })? {
        return Ok(false);
    }
    if !every(
        c,
        "watch_locator",
        "id,repo,number,head,last_success,error",
        |r| {
            let id = r[0].as_str().unwrap();
            let enrolled = exists(
                c,
                "SELECT EXISTS(SELECT 1 FROM main.watch_source WHERE id=? AND source!='selected')",
                [id],
            )?;
            if enrolled
                && !exists(
                    c,
                    "SELECT EXISTS(SELECT 1 FROM main.activity_target WHERE repo=? AND number=?)",
                    params![r[1].as_str(), r[2].as_i64()],
                )?
            {
                return Ok(false);
            }
            let authored = exists(
                c,
                "SELECT EXISTS(SELECT 1 FROM main.watch_source WHERE id=? AND source='authored')",
                [id],
            )?;
            if !authored {
                return Ok(true);
            }
            if r[4].is_null() || !r[5].is_null() {
                return Ok(false);
            }
            exists(
                c,
                "SELECT EXISTS(SELECT 1 FROM main.check_state WHERE pr_id=? AND commit_sha=? AND complete=1 AND error IS NULL AND accessed>=?)",
                params![id, r[3].as_str(), time.saturating_sub(CONTENT_AGE) as i64],
            )
        },
    )? {
        return Ok(false);
    }
    let mut required = 0;
    if !every(c, "resource_state", "key,complete,error", |r| {
        let key = r[0].as_str().unwrap();
        let main = matches!(key, "authored" | "direct" | "teams");
        if main || prefix(key, "team:") {
            if r[1] != 1 || !r[2].is_null() {
                return Ok(false);
            }
            if main {
                required += 1;
            }
        }
        Ok(true)
    })? {
        return Ok(false);
    }
    Ok(required == 3)
}
/// Saturates at limit, bounding materialized keys even behind unrelated history.
pub(crate) fn catchup_keys(c: &Connection, limit: usize) -> Result<Vec<String>> {
    let mut out = vec![];
    let mut after = None;
    loop {
        let rows = window(c, "progress", "key", after)?;
        for (id, r) in &rows {
            after = Some(*id);
            if prefix(r[0].as_str().unwrap(), "catchup:") {
                out.push(r[0].as_str().unwrap().into());
                if out.len() == limit {
                    return Ok(out);
                }
            }
        }
        if rows.len() < WINDOW {
            return Ok(out);
        }
    }
}
pub(crate) fn summary_members(c: &Connection, keys: &[String], time: u64) -> Result<Vec<String>> {
    let mut out = vec![];
    for key in keys {
        let Some(id) = key.strip_prefix("catchup:") else {
            continue;
        };
        if exists(
            c,
            "SELECT EXISTS(SELECT 1 FROM main.activity a JOIN main.seen s ON s.source_id=a.id WHERE a.id=? AND a.eligible=1 AND a.occurred>=? AND s.seen=0)",
            params![
                id,
                time.saturating_sub(crate::retention::ACTIVITY_AGE) as i64
            ],
        )? {
            out.push(id.into());
        }
    }
    Ok(out)
}
/// Count the final staged/native union without materializing/sorting that union.
pub(crate) fn staged_total(
    c: &Connection,
    pr: &str,
    sha: &str,
    first: bool,
    incoming: &[String],
) -> Result<i64> {
    let tx = c.unchecked_transaction()?;
    let mut total = 0;
    let mut after: Option<String> = None;
    if !first {
        loop {
            // Separate first/subsequent seeks avoid an OR that rescans old keys.
            let sql = if after.is_some() {
                "SELECT id FROM main.check_context INDEXED BY check_staged_keys WHERE pr_id=?1 AND commit_sha=?2 AND staged=1 AND id>?3 ORDER BY id LIMIT 128"
            } else {
                "SELECT id FROM main.check_context INDEXED BY check_staged_keys WHERE pr_id=?1 AND commit_sha=?2 AND staged=1 AND ?3 IS NULL ORDER BY id LIMIT 128"
            };
            let mut q = tx.prepare(sql)?;
            let ids = q
                .query_map(params![pr, sha, after], |r| r.get::<_, String>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            observe(&q, ids.len());
            for id in &ids {
                if !incoming.contains(id) {
                    total += 1;
                }
                after = Some(id.clone());
            }
            if ids.len() < WINDOW {
                break;
            }
        }
    }
    let distinct: std::collections::BTreeSet<_> = incoming.iter().collect();
    total += distinct.len() as i64;
    tx.commit()?;
    Ok(total)
}

#[cfg(not(test))]
pub(crate) fn observe(_: &rusqlite::Statement<'_>, _: usize) {}
#[cfg(test)]
#[derive(Debug)]
pub(crate) struct Metric {
    pub rows: usize,
    pub vm: i32,
    pub scans: i32,
    pub sorts: i32,
    pub autoindexes: i32,
}
#[cfg(test)]
thread_local! {static METRICS:std::cell::RefCell<Option<Vec<Metric>>>=const{std::cell::RefCell::new(None)};}
#[cfg(test)]
pub(crate) fn observe(q: &rusqlite::Statement<'_>, rows: usize) {
    use rusqlite::StatementStatus::*;
    METRICS.with(|m| {
        if let Some(v) = m.borrow_mut().as_mut() {
            v.push(Metric {
                rows,
                vm: q.get_status(VmStep),
                scans: q.get_status(FullscanStep),
                sorts: q.get_status(Sort),
                autoindexes: q.get_status(AutoIndex),
            });
        }
    });
}
#[cfg(test)]
pub(crate) fn capture() {
    METRICS.with(|m| *m.borrow_mut() = Some(vec![]));
}
#[cfg(test)]
pub(crate) fn metrics() -> Vec<Metric> {
    METRICS.with(|m| m.borrow_mut().take().unwrap_or_default())
}

pub(crate) fn source_total(c: &Connection, source: &str) -> Result<u64> {
    let tx = c.unchecked_transaction()?;
    let mut after: Option<String> = None;
    let mut total = 0;
    loop {
        let sql = if after.is_some() {
            "SELECT id FROM main.source_stage WHERE source=?1 AND id>?2 ORDER BY id LIMIT 128"
        } else {
            "SELECT id FROM main.source_stage WHERE source=?1 AND ?2 IS NULL ORDER BY id LIMIT 128"
        };
        let mut q = tx.prepare(sql)?;
        let ids = q
            .query_map(params![source, after], |r| r.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        observe(&q, ids.len());
        total += ids.len() as u64;
        after = ids.last().cloned();
        if ids.len() < WINDOW {
            break;
        }
    }
    tx.commit()?;
    Ok(total)
}

#[cfg(test)]
type WindowHook = Option<(String, Box<dyn FnOnce()>)>;
#[cfg(test)]
thread_local! {pub(crate) static ON_WINDOW:std::cell::RefCell<WindowHook>=const{std::cell::RefCell::new(None)};}
#[cfg(test)]
fn point(table: &str, after: Option<i64>) {
    let run = ON_WINDOW.with(|f| {
        if f.borrow().as_ref().is_some_and(|(t, _)| t == table) {
            f.borrow_mut().take()
        } else {
            None
        }
    });
    if let Some((_, run)) = run {
        run();
    }
    if std::env::var("GITJUKE_TEST_COMPLETION_TABLE").as_deref() == Ok(table) && after.is_some() {
        std::fs::write(
            std::env::var_os("GITJUKE_TEST_COMPLETION_MARKER").unwrap(),
            table,
        )
        .unwrap();
        loop {
            std::thread::park();
        }
    }
}
