//! Admission reads bounded identity/length windows in one caller-owned snapshot.
//! Total traversal time still grows with cardinality; no persisted accounting can
//! become stale after a failed transaction or a restart.
use crate::Result;
use rusqlite::{Connection, params};
use std::collections::BinaryHeap;

pub(crate) const WINDOW: usize = 128;

#[derive(Default)]
pub(crate) struct Bytes {
    pub total: i64,
    pub protected: i64,
}

// Rowid is only an ephemeral traversal position, never durable identity. The
// first seek includes MIN so even an explicitly inserted minimum rowid counts.
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
    let mut out = Bytes::default();
    for table in [
        "content",
        "stage",
        "source_stage",
        "check_context",
        "activity_stage",
        "publication_row",
    ] {
        let (length, protected) = match table {
            "content" => ("octet_length(json)", "0"),
            "check_context" => ("octet_length(json)", "staged=1"),
            "publication_row" => (
                "CASE WHEN (tbl='content' AND old_json IS NOT NULL) OR (tbl='check_context' AND json_extract(old_json,'$[4]')=0) THEN old_bytes ELSE 0 END",
                "1",
            ),
            _ => ("octet_length(json)", "1"),
        };
        let mut after = None;
        loop {
            let mut q = c.prepare(&format!("SELECT rowid,{length},{protected} FROM main.{table} WHERE rowid{}? ORDER BY rowid LIMIT {WINDOW}", if after.is_some() { ">" } else { ">=" }))?;
            let mut count = 0;
            {
                let mut rows = q.query([after.unwrap_or(i64::MIN)])?;
                while let Some(r) = rows.next()? {
                    after = Some(r.get::<_, i64>(0)?);
                    let n: i64 = r.get(1)?;
                    out.total += n;
                    if r.get::<_, bool>(2)? {
                        out.protected += n;
                    }
                    count += 1;
                }
            }
            observe(&q, count);
            #[cfg(test)]
            point();
            if count < WINDOW {
                break;
            }
        }
    }
    let inputs = crate::publication::input_bytes(c)?;
    out.total += inputs;
    out.protected += inputs;
    Ok(out)
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
    // Check timestamps belong to their snapshot, not the contexts. Walk the
    // published-key index before joining exact state keys. Empty/staged snapshots
    // cannot create an unbounded rejected prefix inside one SQL statement.
    let mut after: Option<(String, String, String)> = None;
    loop {
        let seek = if after.is_some() {
            "AND (pr_id,commit_sha,id)>(?1,?2,?3)"
        } else {
            "AND ?1 IS NULL AND ?2 IS NULL AND ?3 IS NULL"
        };
        let mut q = c.prepare(&format!("WITH w AS MATERIALIZED(SELECT pr_id,commit_sha,id,octet_length(json) AS bytes FROM main.check_context INDEXED BY check_published_keys WHERE staged=0 {seek} ORDER BY pr_id,commit_sha,id LIMIT {WINDOW}) SELECT COALESCE(s.accessed,0),1,w.pr_id,w.commit_sha,w.id,w.bytes FROM w LEFT JOIN main.check_state s ON s.pr_id=w.pr_id AND s.commit_sha=w.commit_sha"))?;
        let window = q
            .query_map(
                params![
                    after.as_ref().map(|k| &k.0),
                    after.as_ref().map(|k| &k.1),
                    after.as_ref().map(|k| &k.2)
                ],
                victim,
            )?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        observe(&q, window.len());
        let count = window.len();
        // The materialized CTE's output order is unspecified. Advance using the
        // maximum native key, never the last row SQLite happens to return.
        for v in window {
            let key = (v.2.clone(), v.3.clone(), v.4.clone());
            if after.as_ref().is_none_or(|last| &key > last) {
                after = Some(key);
            }
            if cutoff.is_none_or(|t| v.0 < t as i64) {
                retain(&mut best, v, allowance);
            }
        }
        if count < WINDOW {
            break;
        }
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
mod tests;
