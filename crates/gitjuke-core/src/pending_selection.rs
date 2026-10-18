//! Durable queue selection: bounded metadata windows, then exact descriptor reads.
use crate::{Error, Result, publication_completion::observe, store::Store};
use std::{cmp::Reverse, collections::BinaryHeap};

pub(crate) type PendingRow = (i64, String, String, bool, bool);
const WINDOW: usize = 128;
const FIRST: &str =
    "SELECT seq,at,resource,manual,foreground FROM main.pending ORDER BY seq LIMIT 128";
const NEXT: &str =
    "SELECT seq,at,resource,manual,foreground FROM main.pending WHERE seq>? ORDER BY seq LIMIT 128";

pub(crate) fn select(s: &Store, time: u64) -> Result<Vec<PendingRow>> {
    if s.conn.is_autocommit() {
        let tx = s.conn.unchecked_transaction()?;
        let out = select_in(s, time)?;
        tx.commit()?;
        Ok(out)
    } else {
        select_in(s, time)
    }
}
fn select_in(s: &Store, time: u64) -> Result<Vec<PendingRow>> {
    // Native main-table windows cannot schedule from partially applied state
    // hidden by protected views. Startup/repair owns publication completion.
    if crate::publication::pending(&s.conn)? {
        return Err(Error::StorageUnavailable(
            "Durable queue selection waits for pending publication recovery.".into(),
        ));
    }
    if !s.budget_eligible("core", time)? {
        return Ok(vec![]);
    }
    // Preserve the existing policy: core errors propagate; a failed optional
    // resource budget read makes that resource ineligible. All share this snapshot.
    let resources: Vec<_> = ["core", "search", "graphql"]
        .into_iter()
        .filter(|r| s.budget_eligible(r, time).unwrap_or(false))
        .collect();
    let paused = s.settings()?.paused;
    let mut after = None;
    // Rank matches SQL ORDER BY foreground DESC,seq without negation overflow.
    // Keep raw integer priority until final descriptor conversion to bool.
    let mut best: BinaryHeap<(Reverse<i64>, i64)> = BinaryHeap::with_capacity(WINDOW);
    let mut background = None;
    loop {
        let mut q = s.conn.prepare(if after.is_some() { NEXT } else { FIRST })?;
        let row =
            |r: &rusqlite::Row<'_>| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?));
        let rows: Vec<(i64, i64, String, i64, i64)> = if let Some(seq) = after {
            q.query_map([seq], row)?.collect::<rusqlite::Result<_>>()?
        } else {
            q.query_map([], row)?.collect::<rusqlite::Result<_>>()?
        };
        observe(&q, rows.len());
        #[cfg(test)]
        point(s, "candidates", after.is_some())?;
        for (seq, at, resource, manual, foreground) in &rows {
            // The preceding SQL embedded the unsigned time as a numeric literal,
            // so values above i64::MAX still admit all signed SQLite deadlines.
            if i128::from(*at) > i128::from(time)
                || !resources.contains(&resource.as_str())
                || (paused && *manual != 1)
            {
                continue;
            }
            if *foreground == 0 && background.is_none() {
                background = Some(*seq);
            }
            let rank = (Reverse(*foreground), *seq);
            if best.len() < WINDOW {
                best.push(rank);
            } else if best.peek().is_some_and(|last| &rank < last) {
                best.pop();
                best.push(rank);
            }
        }
        if rows.len() < WINDOW {
            break;
        }
        after = rows.last().map(|r| r.0);
    }
    // Preserve the exact previous reservation: only a full all-foreground page
    // replaces its last priority-ranked job with the oldest eligible background.
    if best.len() == WINDOW
        && best.iter().all(|(Reverse(f), _)| *f != 0)
        && let Some(seq) = background
    {
        best.pop();
        best.push((Reverse(0), seq));
    }
    let mut ids: Vec<_> = best.into_iter().map(|(_, seq)| seq).collect();
    ids.sort_unstable();
    let mut out = Vec::with_capacity(ids.len());
    for seq in ids {
        let mut q = s
            .conn
            .prepare("SELECT seq,key,job,manual,foreground FROM main.pending WHERE seq=?")?;
        let row = q.query_row([seq], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
        })?;
        observe(&q, 1);
        out.push(row);
        #[cfg(test)]
        point(s, "descriptors", out.len() > 1)?;
    }
    Ok(out)
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
    if later && std::env::var("GITJUKE_TEST_PENDING_PHASE").as_deref() == Ok(phase) {
        std::fs::write(
            std::env::var_os("GITJUKE_TEST_PENDING_MARKER").unwrap(),
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
#[path = "pending_selection_tests.rs"]
mod tests;
