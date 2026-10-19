//! Maintained shared byte totals and indexed snapshot-frontier LRU selection.
use crate::Result;
use rusqlite::{Connection, params};
use std::collections::BinaryHeap;

pub(crate) const WINDOW: usize = 128;

#[derive(Default)]
pub(crate) struct Bytes {
    pub total: i64,
    pub protected: i64,
}

// A singleton read observes the same transaction as candidate writes. Reporting
// owns a read snapshot when there is no caller transaction.
pub(crate) fn bytes(c: &Connection) -> Result<Bytes> {
    if c.is_autocommit() {
        let tx = c.unchecked_transaction()?;
        let result = bytes_in(&tx)?;
        tx.commit()?;
        Ok(result)
    } else {
        bytes_in(c)
    }
}
fn bytes_in(c: &Connection) -> Result<Bytes> {
    let mut q = c.prepare("SELECT total,protected,invalid FROM main.shared_bytes WHERE id=1")?;
    let (total, protected, invalid): (i64, i64, i64) =
        q.query_row([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
    observe(&q, 1);
    #[cfg(test)]
    point();
    if invalid != 0 {
        return Err(crate::Error::StorageUnavailable("Invalid stored publication input; original database preserved. Repair storage before monitoring resumes.".into()));
    }
    Ok(Bytes { total, protected })
}

// Access time, payload class, then native keys match the settled shared order.
// Bytes are last and cannot affect ordering of distinct identities.
pub(crate) type Victim = (i64, i64, String, String, String, i64);
fn retain(heap: &mut BinaryHeap<Victim>, v: Victim, allowance: usize) {
    if heap.len() < allowance {
        heap.push(v);
    } else if heap.peek().is_some_and(|last| &v < last) {
        heap.pop();
        heap.push(v);
    }
}

pub(crate) fn victims(
    c: &Connection,
    allowance: usize,
    cutoff: Option<u64>,
) -> Result<Vec<Victim>> {
    assert!(allowance <= WINDOW);
    if allowance == 0 {
        return Ok(vec![]);
    }
    let mut best = BinaryHeap::with_capacity(allowance);
    // content_lru supplies the complete order, so only its first allowance keys
    // can belong to the global prefix. No payload body is materialized.
    let filter = if cutoff.is_some() {
        "WHERE accessed<?1"
    } else {
        "WHERE ?1 IS NULL"
    };
    let mut q = c.prepare(&format!("SELECT accessed,0,key,'','',octet_length(json) FROM main.content INDEXED BY content_lru {filter} ORDER BY accessed,key LIMIT ?2"))?;
    let content = q
        .query_map(params![cutoff.map(|t| t as i64), allowance as i64], victim)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    observe(&q, content.len());
    for v in content {
        retain(&mut best, v, allowance);
    }
    // A frontier exists only for snapshots with published contexts. Seek at
    // most allowance snapshots, then advance only streams contributing victims.
    // No empty-state/staged prefix or all-context traversal is needed.
    let check_filter = if cutoff.is_some() {
        "WHERE l.accessed<?1"
    } else {
        "WHERE ?1 IS NULL"
    };
    let mut q = c.prepare(&format!("SELECT l.accessed,1,l.pr_id,l.commit_sha,l.first_id,octet_length(c.json) FROM main.shared_check_lru l INDEXED BY shared_check_order CROSS JOIN main.check_context c ON c.pr_id=l.pr_id AND c.commit_sha=l.commit_sha AND c.id=l.first_id AND c.staged=0 {check_filter} ORDER BY l.accessed,l.pr_id,l.commit_sha LIMIT ?2"))?;
    let seeds = q
        .query_map(params![cutoff.map(|t| t as i64), allowance as i64], victim)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    observe(&q, seeds.len());
    let mut streams: BinaryHeap<std::cmp::Reverse<Victim>> =
        seeds.into_iter().map(std::cmp::Reverse).collect();
    for _ in 0..allowance {
        let Some(std::cmp::Reverse(v)) = streams.pop() else {
            break;
        };
        let mut q = c.prepare("SELECT ?4,1,pr_id,commit_sha,id,octet_length(json) FROM main.check_context INDEXED BY check_published_keys WHERE pr_id=?1 AND commit_sha=?2 AND staged=0 AND id>?3 ORDER BY id LIMIT 1")?;
        use rusqlite::OptionalExtension;
        let next = q
            .query_row(params![v.2, v.3, v.4, v.0], victim)
            .optional()?;
        observe(&q, usize::from(next.is_some()));
        if let Some(next) = next {
            streams.push(std::cmp::Reverse(next));
        }
        retain(&mut best, v, allowance);
    }
    Ok(best.into_sorted_vec())
}
fn victim(r: &rusqlite::Row<'_>) -> rusqlite::Result<Victim> {
    Ok((
        r.get(0)?,
        r.get(1)?,
        r.get(2)?,
        r.get(3)?,
        r.get(4)?,
        r.get(5)?,
    ))
}

#[cfg(not(test))]
fn observe(_: &rusqlite::Statement<'_>, _: usize) {}
#[cfg(test)]
#[derive(Debug)]
pub(crate) struct Metrics {
    pub rows: usize,
    pub vm: i32,
    pub scans: i32,
    pub sorts: i32,
    pub autoindexes: i32,
}
#[cfg(test)]
thread_local! {static METRICS:std::cell::RefCell<Option<Vec<Metrics>>>=const {std::cell::RefCell::new(None)};}
#[cfg(test)]
fn observe(q: &rusqlite::Statement<'_>, rows: usize) {
    use rusqlite::StatementStatus::*;
    METRICS.with(|m| {
        if let Some(m) = m.borrow_mut().as_mut() {
            m.push(Metrics {
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
pub(crate) fn metrics() -> Vec<Metrics> {
    METRICS.with(|m| m.borrow_mut().take().unwrap_or_default())
}
#[cfg(test)]
thread_local! {static ON_WINDOW:std::cell::RefCell<Option<Box<dyn FnOnce()>>>=const {std::cell::RefCell::new(None)};}
#[cfg(test)]
fn point() {
    ON_WINDOW.with(|f| {
        if let Some(f) = f.borrow_mut().take() {
            f();
        }
    });
    if let Ok(marker) = std::env::var("GITJUKE_TEST_ADMISSION_QUERY_MARKER") {
        std::fs::write(marker, "window").unwrap();
        loop {
            std::thread::park();
        }
    }
}

// Stream the one durable request's payload values. RawValue preserves SQLite's
// UTF-8 JSON substring charge without constructing an array of every input.
// The request text itself and individual body lengths are not byte-bounded here.
pub(crate) fn input_bytes(kind: &str, args: &str) -> Result<i64> {
    use serde::de::{DeserializeSeed, IgnoredAny, MapAccess, SeqAccess, Visitor};
    use serde_json::value::RawValue;
    struct Array;
    impl<'de> Visitor<'de> for Array {
        type Value = i64;
        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            f.write_str("payload array")
        }
        fn visit_seq<A: SeqAccess<'de>>(self, mut a: A) -> std::result::Result<i64, A::Error> {
            let mut total = 0;
            while let Some(v) = a.next_element::<&'de RawValue>()? {
                total += v.get().len() as i64;
            }
            Ok(total)
        }
    }
    impl<'de> DeserializeSeed<'de> for Array {
        type Value = i64;
        fn deserialize<D: serde::Deserializer<'de>>(
            self,
            d: D,
        ) -> std::result::Result<i64, D::Error> {
            d.deserialize_seq(self)
        }
    }
    struct Input<'a>(&'a str);
    impl<'de> Visitor<'de> for Input<'_> {
        type Value = i64;
        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            f.write_str("publication request")
        }
        fn visit_map<A: MapAccess<'de>>(self, mut a: A) -> std::result::Result<i64, A::Error> {
            let mut total = 0;
            while let Some(k) = a.next_key::<String>()? {
                if (self.0 == "checks" && k == "contexts")
                    || (self.0 == "source_page" && k == "prs")
                {
                    total += a.next_value_seed(Array)?;
                } else if matches!(self.0, "selected" | "head") && k == "pr" {
                    let v = a.next_value::<&'de RawValue>()?;
                    if v.get() != "null" {
                        total += v.get().len() as i64;
                    }
                } else {
                    a.next_value::<IgnoredAny>()?;
                }
            }
            Ok(total)
        }
    }
    use serde::Deserializer;
    let mut d = serde_json::Deserializer::from_str(args);
    let invalid = |_| {
        crate::Error::StorageUnavailable(
        "Invalid stored publication input; original database preserved. Repair storage before monitoring resumes.".into(),
    )
    };
    let total = d.deserialize_map(Input(kind)).map_err(invalid)?;
    d.end().map_err(invalid)?;
    Ok(total)
}

#[cfg(test)]
fn capture() {
    METRICS.with(|m| *m.borrow_mut() = Some(Vec::new()));
}

#[cfg(test)]
#[path = "admission_query_tests.rs"]
pub(crate) mod tests;
