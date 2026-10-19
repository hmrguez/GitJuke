//! Watch scheduling uses snapshot windows and bounded reservation membership.
use crate::{Error, Result, publication_completion::observe, store::Store, sync::WatchTarget};
use rusqlite::params;
use std::collections::{BinaryHeap, HashMap};

const WINDOW: usize = 128;
const FIRST: &str =
    "SELECT id,repo,number,epoch,due,last_success FROM main.watch_locator ORDER BY id LIMIT 128";
const NEXT: &str = "SELECT id,repo,number,epoch,due,last_success FROM main.watch_locator WHERE id>? ORDER BY id LIMIT 128";
type Rank = (bool, i64, String, String, i64, i64);

pub(crate) fn select(
    s: &Store,
    time: u64,
    limit: usize,
    selected: Option<&(String, u64)>,
    manual: bool,
) -> Result<Vec<WatchTarget>> {
    if s.conn.is_autocommit() {
        let tx = s.conn.unchecked_transaction()?;
        let out = select_in(s, time, limit, selected, manual)?;
        tx.commit()?;
        Ok(out)
    } else {
        select_in(s, time, limit, selected, manual)
    }
}
fn select_in(
    s: &Store,
    time: u64,
    limit: usize,
    selected: Option<&(String, u64)>,
    manual: bool,
) -> Result<Vec<WatchTarget>> {
    if crate::publication::pending(&s.conn)? {
        return Err(Error::StorageUnavailable(
            "Watch scheduling waits for pending publication recovery.".into(),
        ));
    }
    if limit == 0 {
        return Ok(vec![]);
    }
    let boundary = s.due("manual-boundary")? as i64;
    let (selected_repo, selected_number) = selected
        .map(|(r, n)| (r.as_str(), *n as i64))
        .unwrap_or(("", 0));
    // The public limit remains caller-defined. Production head batches start
    // at ten and can shrink; no hard watch-membership cap is introduced.
    let mut best: BinaryHeap<Rank> = BinaryHeap::new();
    let mut after: Option<String> = None;
    loop {
        let mut q = s.conn.prepare(if after.is_some() { NEXT } else { FIRST })?;
        let row = |r: &rusqlite::Row<'_>| {
            Ok((
                r.get(0)?,
                r.get(1)?,
                r.get(2)?,
                r.get(3)?,
                r.get(4)?,
                r.get(5)?,
            ))
        };
        let rows: Vec<(String, String, i64, i64, i64, Option<i64>)> = if let Some(id) = &after {
            q.query_map([id], row)?.collect::<rusqlite::Result<_>>()?
        } else {
            q.query_map([], row)?.collect::<rusqlite::Result<_>>()?
        };
        observe(&q, rows.len());
        #[cfg(test)]
        point(s, "watches", after.is_some())?;
        let mut candidates = vec![];
        for (id, repo, number, epoch, due, last_success) in &rows {
            let rank = (
                !(repo == selected_repo && *number == selected_number),
                *due,
                id.clone(),
                repo.clone(),
                *number,
                *epoch,
            );
            if *due > time as i64
                || (manual && last_success.is_some_and(|t| t >= boundary))
                || (best.len() == limit && best.peek().is_some_and(|last| &rank >= last))
            {
                continue;
            }
            let mut q=s.conn.prepare("SELECT EXISTS(SELECT 1 FROM main.watch_source WHERE id=?1) AND NOT EXISTS(SELECT 1 FROM main.pr_purge WHERE repo=?2 COLLATE NOCASE AND number=?3)")?;
            let eligible: bool = q.query_row(params![id, repo, number], |r| r.get(0))?;
            observe(&q, 1);
            if eligible {
                candidates.push(rank);
            }
        }
        if !candidates.is_empty() {
            let mut reserved: HashMap<&str, bool> =
                candidates.iter().map(|r| (r.2.as_str(), false)).collect();
            reservations(s, &mut reserved)?;
            for rank in &candidates {
                if reserved[rank.2.as_str()] {
                    continue;
                }
                if best.len() < limit {
                    best.push(rank.clone());
                } else if best.peek().is_some_and(|last| rank < last) {
                    best.pop();
                    best.push(rank.clone());
                }
            }
        }
        if rows.len() < WINDOW {
            break;
        }
        after = rows.last().map(|r| r.0.clone());
    }
    Ok(best
        .into_sorted_vec()
        .into_iter()
        .map(|(_, _, id, repo, number, epoch)| WatchTarget {
            id,
            repo,
            number: number as u64,
            generation: epoch as u64,
        })
        .collect())
}
fn reservations(s: &Store, reserved: &mut HashMap<&str, bool>) -> Result<()> {
    let mut after = None;
    loop {
        let mut q = s.conn.prepare(if after.is_some() {
            "SELECT seq FROM main.pending WHERE seq>? ORDER BY seq LIMIT 128"
        } else {
            "SELECT seq FROM main.pending ORDER BY seq LIMIT 128"
        })?;
        let seqs: Vec<i64> = if let Some(seq) = after {
            q.query_map([seq], |r| r.get(0))?
                .collect::<rusqlite::Result<_>>()?
        } else {
            q.query_map([], |r| r.get(0))?
                .collect::<rusqlite::Result<_>>()?
        };
        observe(&q, seqs.len());
        #[cfg(test)]
        point(s, "pending", after.is_some())?;
        for seq in &seqs {
            let mut q = s
                .conn
                .prepare("SELECT json(job -> '$.Heads') FROM main.pending WHERE seq=?")?;
            let heads: Option<String> = q.query_row([seq], |r| r.get(0))?;
            observe(&q, 1);
            if let Some(heads) = heads {
                crate::watch_reservations::each(&s.conn, &heads, |id| {
                    if let Some(flag) = reserved.get_mut(id) {
                        *flag = true;
                    }
                    Ok(())
                })?;
            }
            #[cfg(test)]
            point(s, "reservations", true)?;
        }
        if seqs.len() < WINDOW {
            return Ok(());
        }
        after = seqs.last().copied();
    }
}

#[cfg(test)]
type WindowHook = Option<Box<dyn FnMut(&Store, &str, bool) -> Result<()>>>;
#[cfg(test)]
thread_local! {static ON_WINDOW:std::cell::RefCell<WindowHook>=const{std::cell::RefCell::new(None)};}
#[cfg(test)]
fn point(s: &Store, phase: &str, later: bool) -> Result<()> {
    ON_WINDOW.with(|hook| -> Result<()> {
        if let Some(f) = hook.borrow_mut().as_mut() {
            f(s, phase, later)?;
        }
        Ok(())
    })?;
    if later && std::env::var("GITJUKE_TEST_WATCH_PHASE").as_deref() == Ok(phase) {
        std::fs::write(
            std::env::var_os("GITJUKE_TEST_WATCH_MARKER").unwrap(),
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
#[path = "watch_scheduling_tests.rs"]
mod tests;
