//! Bounded native candidate reads, including protected old publication images.
use crate::{
    Error, Result,
    model::{WatchPage, WatchRow},
    publication_completion::observe,
    store::Store,
};
use rusqlite::{Connection, OptionalExtension, params};
use std::collections::BTreeSet;
const WINDOW: usize = 128;
const PAGE: usize = 100;
const BYTES: usize = 900 * 1024;
struct Reader<'a> {
    c: &'a Connection,
    overlay: bool,
}
#[derive(Clone, Copy)]
enum Keys<'a> {
    Locators,
    Reasons(&'a str),
}

pub(crate) fn page(s: &Store, offset: usize) -> Result<WatchPage> {
    if s.conn.is_autocommit() {
        let tx = s.conn.unchecked_transaction()?;
        let out = page_in(s, offset)?;
        tx.commit()?;
        Ok(out)
    } else {
        page_in(s, offset)
    }
}
fn page_in(s: &Store, offset: usize) -> Result<WatchPage> {
    let phase: Option<String> = s
        .conn
        .query_row("SELECT phase FROM main.publication WHERE id=1", [], |r| {
            r.get(0)
        })
        .optional()?;
    let reader = Reader {
        c: &s.conn,
        overlay: phase.is_some_and(|p| p != "committed"),
    };
    // SQLite treats a negative bound OFFSET as zero. Keep that established
    // conversion while checking returned-offset arithmetic instead of wrapping.
    let skip = if (offset as i64) < 0 { 0 } else { offset };
    let mut skipped = 0;
    let mut after = None;
    let mut items = vec![];
    'windows: loop {
        let ids = reader.keys(Keys::Locators, after.as_deref())?;
        #[cfg(test)]
        point(s, "locators", after.is_some())?;
        for id in &ids {
            if !reader.locator_visible(id)? || !reader.has_reason(id)? {
                continue;
            }
            if skipped < skip {
                skipped += 1;
                continue;
            }
            if let Some(item) = reader.row(id)? {
                items.push(item);
            }
            if items.len() == PAGE + 1 {
                break 'windows;
            }
        }
        if ids.len() < WINDOW {
            break;
        }
        after = ids.last().cloned();
    }
    let more = items.len() > PAGE;
    items.truncate(PAGE);
    let mut bytes = serde_json::to_vec(&items)?.len();
    budget(bytes)?;
    for item in &mut items {
        reader.reasons(&item.id, |reason| {
            bytes += serde_json::to_vec(&reason)?.len() + usize::from(!item.reasons.is_empty());
            budget(bytes)?;
            item.reasons.push(reason);
            Ok(false)
        })?;
        #[cfg(test)]
        point(s, "reasons", true)?;
    }
    // Keep the original exact serialization guard as well as early rejection.
    budget(serde_json::to_vec(&items)?.len())?;
    let next_offset = if more {
        Some(
            offset
                .checked_add(items.len())
                .ok_or_else(|| Error::Safe("Watch pagination offset overflow.".into()))?,
        )
    } else {
        None
    };
    Ok(WatchPage {
        items,
        next_offset,
        revision: s.revision()?,
    })
}
fn budget(bytes: usize) -> Result<()> {
    if bytes > BYTES {
        return Err(Error::Safe(
            "Watch page exceeds the IPC byte budget.".into(),
        ));
    }
    Ok(())
}
fn merge(a: Vec<String>, b: Vec<String>) -> Vec<String> {
    let mut a = a.into_iter().peekable();
    let mut b = b.into_iter().peekable();
    let mut out = vec![];
    while out.len() < WINDOW {
        let value = match (a.peek(), b.peek()) {
            (Some(x), Some(y)) => match x.cmp(y) {
                std::cmp::Ordering::Less => a.next(),
                std::cmp::Ordering::Greater => b.next(),
                std::cmp::Ordering::Equal => {
                    b.next();
                    a.next()
                }
            },
            (Some(_), None) => a.next(),
            (None, Some(_)) => b.next(),
            (None, None) => break,
        };
        if let Some(value) = value {
            out.push(value);
        }
    }
    out
}
impl Reader<'_> {
    fn keys(&self, kind: Keys<'_>, after: Option<&str>) -> Result<Vec<String>> {
        let (sql, scope) = match kind {
            Keys::Locators => (
                if after.is_some() {
                    "SELECT id FROM main.watch_locator WHERE id>?1 ORDER BY id LIMIT 128"
                } else {
                    "SELECT id FROM main.watch_locator WHERE ?1 IS NULL ORDER BY id LIMIT 128"
                },
                None,
            ),
            Keys::Reasons(id) => (
                if after.is_some() {
                    "SELECT source FROM main.watch_source WHERE source>?1 AND id=?2 ORDER BY source LIMIT 128"
                } else {
                    "SELECT source FROM main.watch_source WHERE ?1 IS NULL AND id=?2 ORDER BY source LIMIT 128"
                },
                Some(id),
            ),
        };
        let mut q = self.c.prepare(sql)?;
        let ids: Vec<String> = if let Some(id) = scope {
            q.query_map(params![after, id], |r| r.get(0))?
                .collect::<rusqlite::Result<_>>()?
        } else {
            q.query_map([after], |r| r.get(0))?
                .collect::<rusqlite::Result<_>>()?
        };
        observe(&q, ids.len());
        if !self.overlay {
            return Ok(ids);
        }
        Ok(merge(ids, self.old_keys(kind, after)?))
    }
    fn old_keys(&self, kind: Keys<'_>, after: Option<&str>) -> Result<Vec<String>> {
        let mut keys = BTreeSet::new();
        let mut cursor = None;
        loop {
            let mut q=self.c.prepare(if cursor.is_some(){"SELECT seq,tbl,k,old_json IS NOT NULL FROM main.publication_row WHERE seq>? ORDER BY seq LIMIT 128"}else{"SELECT seq,tbl,k,old_json IS NOT NULL FROM main.publication_row ORDER BY seq LIMIT 128"})?;
            let row = |r: &rusqlite::Row<'_>| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?));
            let rows: Vec<(i64, String, String, bool)> = if let Some(cursor) = cursor {
                q.query_map([cursor], row)?
                    .collect::<rusqlite::Result<_>>()?
            } else {
                q.query_map([], row)?.collect::<rusqlite::Result<_>>()?
            };
            observe(&q, rows.len());
            for (_, table, key, old) in &rows {
                if !old
                    || table
                        != match kind {
                            Keys::Locators => "watch_locator",
                            Keys::Reasons(_) => "watch_source",
                        }
                {
                    continue;
                }
                let k: Vec<String> = serde_json::from_str(key).map_err(|_| {
                    Error::StorageUnavailable(
                        "Invalid protected watch key; original database preserved.".into(),
                    )
                })?;
                let value = match kind {
                    Keys::Locators if k.len() == 1 => &k[0],
                    Keys::Reasons(id) if k.len() == 2 => {
                        if k[0] != id {
                            continue;
                        }
                        &k[1]
                    }
                    _ => {
                        return Err(Error::StorageUnavailable(
                            "Invalid protected watch key; original database preserved.".into(),
                        ));
                    }
                };
                if after.is_some_and(|p| value.as_str() <= p) || keys.contains(value) {
                    continue;
                }
                if keys.len() < WINDOW {
                    keys.insert(value.clone());
                } else if keys.last().is_some_and(|last| value < last) {
                    keys.pop_last();
                    keys.insert(value.clone());
                }
            }
            if rows.len() < WINDOW {
                break;
            }
            cursor = rows.last().map(|r| r.0);
        }
        Ok(keys.into_iter().collect())
    }
    fn old_visible(&self, id: &str, reason: Option<&str>) -> Result<Option<bool>> {
        if !self.overlay {
            return Ok(None);
        }
        let mut q=self.c.prepare(if reason.is_some(){"SELECT old_json IS NOT NULL FROM main.publication_row WHERE tbl='watch_source' AND k=json_array(?1,?2)"}else{"SELECT old_json IS NOT NULL FROM main.publication_row WHERE tbl='watch_locator' AND k=json_array(?1)"})?;
        let out = if let Some(reason) = reason {
            q.query_row(params![id, reason], |r| r.get(0)).optional()?
        } else {
            q.query_row([id], |r| r.get(0)).optional()?
        };
        observe(&q, usize::from(out.is_some()));
        Ok(out)
    }
    fn locator_visible(&self, id: &str) -> Result<bool> {
        if let Some(old) = self.old_visible(id, None)? {
            return Ok(old);
        }
        let mut q = self
            .c
            .prepare("SELECT EXISTS(SELECT 1 FROM main.watch_locator WHERE id=?)")?;
        let out = q.query_row([id], |r| r.get(0))?;
        observe(&q, 1);
        Ok(out)
    }
    fn has_reason(&self, id: &str) -> Result<bool> {
        if !self.overlay {
            let mut q = self
                .c
                .prepare("SELECT EXISTS(SELECT 1 FROM main.watch_source WHERE id=?)")?;
            let out = q.query_row([id], |r| r.get(0))?;
            observe(&q, 1);
            return Ok(out);
        }
        let mut found = false;
        self.reasons(id, |_| {
            found = true;
            Ok(true)
        })?;
        Ok(found)
    }
    fn reasons(&self, id: &str, mut accept: impl FnMut(String) -> Result<bool>) -> Result<()> {
        let mut after = None;
        loop {
            let reasons = self.keys(Keys::Reasons(id), after.as_deref())?;
            for reason in &reasons {
                if self.old_visible(id, Some(reason))? == Some(false) {
                    continue;
                }
                if accept(reason.clone())? {
                    return Ok(());
                }
            }
            if reasons.len() < WINDOW {
                return Ok(());
            }
            after = reasons.last().cloned();
        }
    }
    fn row(&self, id: &str) -> Result<Option<WatchRow>> {
        let mut out = None;
        if self.overlay {
            let mut q=self.c.prepare("SELECT json_extract(old_json,'$[0]'),json_extract(old_json,'$[1]'),json_extract(old_json,'$[2]'),json_extract(old_json,'$[6]'),json_extract(old_json,'$[7]'),json_extract(old_json,'$[10]'),json_extract(old_json,'$[9]'),json_extract(old_json,'$[11]'),old_json IS NULL FROM main.publication_row WHERE tbl='watch_locator' AND k=json_array(?)")?;
            let old = q
                .query_row([id], |r| {
                    if r.get::<_, bool>(8)? {
                        Ok(None)
                    } else {
                        read_row(r).map(Some)
                    }
                })
                .optional()?;
            observe(&q, usize::from(old.is_some()));
            if let Some(old) = old {
                out = Some(old);
            }
        }
        let mut item = if let Some(old) = out {
            old
        } else {
            let mut q=self.c.prepare("SELECT id,repo,number,head,merge_sha,last_success,due,error FROM main.watch_locator WHERE id=?")?;
            let row = q.query_row([id], read_row).optional()?;
            observe(&q, usize::from(row.is_some()));
            row
        };
        if let Some(item) = &mut item {
            let mut q=self.c.prepare("SELECT EXISTS(SELECT 1 FROM main.pr_purge WHERE repo=? COLLATE NOCASE AND number=?)")?;
            let denied: bool = q.query_row(params![item.repo, item.number as i64], |r| r.get(0))?;
            observe(&q, 1);
            if denied {
                item.error = Some("Resource unavailable".into());
            }
        }
        Ok(item)
    }
}
fn read_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<WatchRow> {
    Ok(WatchRow {
        id: r.get(0)?,
        repo: r.get(1)?,
        number: r.get::<_, i64>(2)? as u64,
        head: r.get(3)?,
        merge: r.get(4)?,
        refreshed_at: r.get::<_, Option<i64>>(5)?.map(|n| n as u64),
        due_at: r.get::<_, i64>(6)? as u64,
        error: r.get(7)?,
        reasons: vec![],
    })
}

#[cfg(test)]
type Hook = Option<Box<dyn FnMut(&Store, &str, bool) -> Result<()>>>;
#[cfg(test)]
thread_local! {static ON_WINDOW:std::cell::RefCell<Hook>=const{std::cell::RefCell::new(None)};}
#[cfg(test)]
fn point(s: &Store, phase: &str, later: bool) -> Result<()> {
    ON_WINDOW.with(|hook| -> Result<()> {
        if let Some(f) = hook.borrow_mut().as_mut() {
            f(s, phase, later)?;
        }
        Ok(())
    })?;
    if later && std::env::var("GITJUKE_TEST_WATCH_READER_PHASE").as_deref() == Ok(phase) {
        std::fs::write(
            std::env::var_os("GITJUKE_TEST_WATCH_READER_MARKER").unwrap(),
            phase,
        )
        .unwrap();
        loop {
            std::thread::park();
        }
    }
    Ok(())
}
#[cfg(test)]
#[path = "watch_reader_tests.rs"]
mod tests;
