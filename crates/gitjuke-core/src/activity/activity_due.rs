//! Snapshot-consistent hydration eligibility with bounded candidate buffers.
use crate::{Error, Result, publication_completion::observe};
use rusqlite::{Connection, params};
use std::collections::BinaryHeap;

const WINDOW: usize = 128;
const FIRST: &str =
    "SELECT repo,number,due,done FROM main.activity_target ORDER BY repo,number LIMIT 128";
const NEXT: &str = "SELECT repo,number,due,done FROM main.activity_target WHERE (repo,number)>(?1,?2) ORDER BY repo,number LIMIT 128";
const LOCATORS: &str = "SELECT id FROM main.watch_locator INDEXED BY watch_locator_location WHERE repo=?1 AND number=?2 ORDER BY id LIMIT 128";
const NEXT_LOCATORS: &str = "SELECT id FROM main.watch_locator INDEXED BY watch_locator_location WHERE repo=?1 AND number=?2 AND id>?3 ORDER BY id LIMIT 128";

pub(crate) fn select(c: &Connection, time: u64) -> Result<Vec<(String, u64)>> {
    if c.is_autocommit() {
        let tx = c.unchecked_transaction()?;
        let out = select_in(&tx, time)?;
        tx.commit()?;
        Ok(out)
    } else {
        select_in(c, time)
    }
}

fn select_in(c: &Connection, time: u64) -> Result<Vec<(String, u64)>> {
    // Native indexed scans cannot read the union-based protected reader views.
    // Startup/repair must finish publication before scheduling from main tables.
    if crate::publication::pending(c)? {
        return Err(Error::StorageUnavailable(
            "Activity hydration eligibility waits for pending publication recovery.".into(),
        ));
    }
    let mut after: Option<(String, i64)> = None;
    let mut best: BinaryHeap<(i64, String, i64)> = BinaryHeap::with_capacity(WINDOW);
    loop {
        let mut q = c.prepare(if after.is_some() { NEXT } else { FIRST })?;
        let row = |r: &rusqlite::Row<'_>| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?));
        let rows: Vec<(String, i64, i64, i64)> = if let Some((repo, number)) = &after {
            q.query_map(params![repo, number], row)?
                .collect::<rusqlite::Result<_>>()?
        } else {
            q.query_map([], row)?.collect::<rusqlite::Result<_>>()?
        };
        observe(&q, rows.len());
        #[cfg(test)]
        point(c, "targets", after.is_some())?;
        for (repo, number, due, done) in &rows {
            // Preserve the existing u64-to-SQLite-time conversion and exact
            // signed due/repository/number ordering, independent of scan order.
            let candidate = (*due, repo.clone(), *number);
            if *due > time as i64
                || (best.len() == WINDOW && best.peek().is_some_and(|last| &candidate >= last))
            {
                continue;
            }
            let mut q = c.prepare(
                "SELECT EXISTS(SELECT 1 FROM main.pending WHERE key='activity:pr:'||?1||':'||?2)",
            )?;
            let pending: bool = q.query_row(params![repo, number], |r| r.get(0))?;
            observe(&q, 1);
            if pending || (*done != 0 && !watched(c, repo, *number)?) {
                continue;
            }
            if best.len() == WINDOW {
                best.pop();
            }
            best.push(candidate);
        }
        if rows.len() < WINDOW {
            break;
        }
        after = rows.last().map(|(r, n, _, _)| (r.clone(), *n));
    }
    // Rust UTF-8 byte order matches the native BINARY text collation. Only the
    // bounded heap is sorted; signed numbers are converted after ordering.
    Ok(best
        .into_sorted_vec()
        .into_iter()
        .map(|(_, r, n)| (r, n as u64))
        .collect())
}

fn watched(c: &Connection, repo: &str, number: i64) -> Result<bool> {
    let mut after: Option<String> = None;
    loop {
        let mut q = c.prepare(if after.is_some() {
            NEXT_LOCATORS
        } else {
            LOCATORS
        })?;
        let ids: Vec<String> = if let Some(id) = &after {
            q.query_map(params![repo, number, id], |r| r.get(0))?
                .collect::<rusqlite::Result<_>>()?
        } else {
            q.query_map(params![repo, number], |r| r.get(0))?
                .collect::<rusqlite::Result<_>>()?
        };
        observe(&q, ids.len());
        #[cfg(test)]
        point(c, "locators", after.is_some())?;
        for id in &ids {
            // At most two reasons are examined: only the single unique exact
            // source 'selected' can be rejected for an indexed identity.
            let mut q = c.prepare(
                "SELECT EXISTS(SELECT 1 FROM main.watch_source WHERE id=? AND source!='selected')",
            )?;
            let eligible: bool = q.query_row([id], |r| r.get(0))?;
            observe(&q, 1);
            if eligible {
                return Ok(true);
            }
        }
        if ids.len() < WINDOW {
            return Ok(false);
        }
        after = ids.into_iter().last();
    }
}

#[cfg(test)]
type WindowHook = Option<Box<dyn FnMut(&Connection, &str, bool) -> Result<()>>>;
#[cfg(test)]
thread_local! {static ON_WINDOW:std::cell::RefCell<WindowHook>=const{std::cell::RefCell::new(None)};}
#[cfg(test)]
fn point(c: &Connection, relation: &str, later: bool) -> Result<()> {
    ON_WINDOW.with(|hook| -> Result<()> {
        if let Some(f) = hook.borrow_mut().as_mut() {
            f(c, relation, later)?;
        }
        Ok(())
    })?;
    if later && std::env::var("GITJUKE_TEST_HYDRATION_RELATION").as_deref() == Ok(relation) {
        std::fs::write(
            std::env::var_os("GITJUKE_TEST_HYDRATION_MARKER").unwrap(),
            relation,
        )
        .unwrap();
        loop {
            std::thread::park();
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "activity_due_tests.rs"]
mod tests;
