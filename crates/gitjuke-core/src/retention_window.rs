//! Indexed keyset windows bound candidates examined, not just victims returned.
//! Disposable cursors advance with their deletions; they never certify recovery.
use crate::{Error, Result};
use rusqlite::{OptionalExtension, Transaction, params};
use serde_json::Value;

pub(crate) const WINDOW: usize = 128;
#[derive(Clone, Copy, Debug)]
pub(crate) enum Class {
    Orphans,
    Checks,
    Validators,
    Resources,
    States,
}
impl Class {
    fn name(self) -> &'static str {
        match self {
            Self::Orphans => "orphans",
            Self::Checks => "checks",
            Self::Validators => "validators",
            Self::Resources => "resources",
            Self::States => "states",
        }
    }
    fn cost(self) -> usize {
        if matches!(self, Self::Checks) { 2 } else { 1 }
    }
    fn width(self) -> usize {
        if matches!(self, Self::Checks | Self::States) {
            3
        } else {
            1
        }
    }
    pub(crate) fn fence_sql(self) -> &'static str {
        match self {
            Self::Orphans => {
                "SELECT json_array(key) FROM main.content WHERE key>='activity-payload:' AND key<'activity-payload;' ORDER BY key DESC LIMIT 1"
            }
            Self::Checks => {
                "SELECT json_array(pr_id,commit_sha,id) FROM main.check_context INDEXED BY check_published_keys WHERE staged=0 ORDER BY pr_id DESC,commit_sha DESC,id DESC LIMIT 1"
            }
            Self::Validators => {
                "SELECT json_array(url) FROM main.api_validator ORDER BY url DESC LIMIT 1"
            }
            Self::Resources => {
                "SELECT json_array(key) FROM main.resource_state WHERE key>='api:' AND key<'api;' ORDER BY key DESC LIMIT 1"
            }
            Self::States => {
                "SELECT json_array(accessed,pr_id,commit_sha) FROM main.check_state INDEXED BY check_expiry WHERE accessed<:cutoff ORDER BY accessed DESC,pr_id DESC,commit_sha DESC LIMIT 1"
            }
        }
    }
    fn fence_bound(self) -> &'static str {
        match self {
            Self::Orphans | Self::Resources => "key<=json_extract(:ceiling,'$[0]')",
            Self::Validators => "url<=json_extract(:ceiling,'$[0]')",
            Self::Checks => {
                "(pr_id,commit_sha,id)<=(json_extract(:ceiling,'$[0]'),json_extract(:ceiling,'$[1]'),json_extract(:ceiling,'$[2]'))"
            }
            Self::States => {
                "(accessed,pr_id,commit_sha)<=(json_extract(:ceiling,'$[0]'),json_extract(:ceiling,'$[1]'),json_extract(:ceiling,'$[2]'))"
            }
        }
    }
    pub(crate) fn sql(self, after: bool) -> String {
        let (columns, table, base, order, seek, eligible) = match self {
            Self::Orphans => (
                "key",
                "main.content",
                if after {
                    "key<'activity-payload;'"
                } else {
                    "key>='activity-payload:' AND key<'activity-payload;'"
                },
                "key",
                "key>json_extract(:after,'$[0]')",
                "NOT EXISTS(SELECT 1 FROM main.activity a WHERE a.id=substr(w.key,18))",
            ),
            Self::Checks => (
                "pr_id,commit_sha,id",
                "main.check_context INDEXED BY check_published_keys",
                "staged=0",
                "pr_id,commit_sha,id",
                "(pr_id,commit_sha,id)>(json_extract(:after,'$[0]'),json_extract(:after,'$[1]'),json_extract(:after,'$[2]'))",
                "COALESCE((SELECT accessed FROM main.check_state s WHERE s.pr_id=w.pr_id AND s.commit_sha=w.commit_sha),0)<:cutoff",
            ),
            Self::Validators => (
                "url",
                "main.api_validator",
                "1",
                "url",
                "url>json_extract(:after,'$[0]')",
                "(SELECT next_at<=:cutoff FROM main.api_validator v WHERE v.url=w.url) AND NOT EXISTS(SELECT 1 FROM main.content c WHERE c.key='api:'||w.url)",
            ),
            Self::Resources => (
                "key",
                "main.resource_state",
                if after {
                    "key<'api;'"
                } else {
                    "key>='api:' AND key<'api;'"
                },
                "key",
                "key>json_extract(:after,'$[0]')",
                "NOT EXISTS(SELECT 1 FROM main.content c WHERE c.key=w.key) AND NOT EXISTS(SELECT 1 FROM main.api_validator v WHERE v.url=substr(w.key,5))",
            ),
            Self::States => (
                "accessed,pr_id,commit_sha",
                "main.check_state INDEXED BY check_expiry",
                "accessed<:cutoff",
                "accessed,pr_id,commit_sha",
                "(accessed,pr_id,commit_sha)>(json_extract(:after,'$[0]'),json_extract(:after,'$[1]'),json_extract(:after,'$[2]'))",
                "NOT EXISTS(SELECT 1 FROM main.check_context c WHERE c.pr_id=w.pr_id AND c.commit_sha=w.commit_sha) AND NOT EXISTS(SELECT 1 FROM main.pending p WHERE p.key='checks:'||w.pr_id||':'||w.commit_sha) AND NOT EXISTS(SELECT 1 FROM main.watch_locator l WHERE l.id=w.pr_id AND (l.head=w.commit_sha OR l.merge_sha=w.commit_sha) AND EXISTS(SELECT 1 FROM main.watch_source s WHERE s.id=l.id))",
            ),
        };
        format!(
            "WITH w AS MATERIALIZED(SELECT {columns} FROM {table} WHERE {base} AND {} {} ORDER BY {order} LIMIT {WINDOW}) SELECT json_array({columns}),({eligible}) FROM w",
            self.fence_bound(),
            if after {
                format!("AND {seek}")
            } else {
                String::new()
            }
        )
    }
}
fn candidates(
    tx: &Transaction<'_>,
    class: Class,
    cursor: Option<&str>,
    cutoff: u64,
    ceiling: &str,
) -> Result<Vec<(String, bool)>> {
    let mut q = tx.prepare(&class.sql(cursor.is_some()))?;
    q.raw_bind_parameter(q.parameter_index(":ceiling")?.unwrap(), ceiling)?;
    if let Some(i) = q.parameter_index(":after")? {
        q.raw_bind_parameter(i, cursor)?;
    }
    if let Some(i) = q.parameter_index(":cutoff")? {
        q.raw_bind_parameter(i, cutoff as i64)?;
    }
    let mut out: Vec<(String, bool)> = Vec::new();
    {
        let mut rows = q.raw_query();
        while let Some(r) = rows.next()? {
            out.push((r.get(0)?, r.get(1)?));
        }
    }
    #[cfg(test)]
    observe(&q, out.len());
    // SQL does not promise an outer scan order for a materialized CTE. Sort
    // only this bounded metadata window before advancing a native tuple cursor.
    #[derive(PartialEq, Eq, PartialOrd, Ord)]
    enum Part {
        Number(i64),
        Text(String),
    }
    let mut ordered = out
        .into_iter()
        .map(|(key, eligible)| -> Result<_> {
            let v: Value = serde_json::from_str(&key)?;
            let parts = v
                .as_array()
                .ok_or_else(|| Error::StorageUnavailable("Invalid retention identity.".into()))?
                .iter()
                .map(|v| {
                    if let Some(n) = v.as_i64() {
                        Ok(Part::Number(n))
                    } else if let Some(s) = v.as_str() {
                        Ok(Part::Text(s.into()))
                    } else {
                        Err(Error::StorageUnavailable(
                            "Invalid retention identity.".into(),
                        ))
                    }
                })
                .collect::<Result<Vec<_>>>()?;
            Ok((parts, key, eligible))
        })
        .collect::<Result<Vec<_>>>()?;
    ordered.sort_unstable_by(|a, b| a.0.cmp(&b.0));
    Ok(ordered
        .into_iter()
        .map(|(_, key, eligible)| (key, eligible))
        .collect())
}
fn fence(tx: &Transaction<'_>, class: Class, cutoff: u64) -> Result<Option<String>> {
    let mut q = tx.prepare(class.fence_sql())?;
    if let Some(i) = q.parameter_index(":cutoff")? {
        q.raw_bind_parameter(i, cutoff as i64)?;
    }
    let out = {
        let mut rows = q.raw_query();
        rows.next()?.map(|r| r.get(0)).transpose()?
    };
    #[cfg(test)]
    observe(&q, usize::from(out.is_some()));
    Ok(out)
}
/// Returns visible mutations; cursor bookkeeping consumes the same allowance,
/// but moving through retained candidates alone does not bump presentation.
pub(crate) fn sweep(
    tx: &Transaction<'_>,
    class: Class,
    cutoff: u64,
    allowance: &mut usize,
) -> Result<usize> {
    if *allowance <= class.cost() {
        return Ok(0);
    }
    let saved: Option<(String, String)> = tx
        .query_row(
            "SELECT position,ceiling FROM retention_cursor WHERE class=?",
            [class.name()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    for cursor in saved
        .iter()
        .flat_map(|(position, ceiling)| [position, ceiling])
    {
        let key: Value = serde_json::from_str(cursor).map_err(|_| {
            Error::StorageUnavailable(
                "Invalid retention cursor; original database preserved.".into(),
            )
        })?;
        let valid = key.as_array().is_some_and(|a| {
            a.len() == class.width()
                && a.iter().enumerate().all(|(i, k)| {
                    if matches!(class, Class::States) && i == 0 {
                        k.as_i64().is_some()
                    } else {
                        k.is_string()
                    }
                })
        });
        let prefix_valid = match class {
            Class::Orphans => key[0]
                .as_str()
                .is_some_and(|s| s.starts_with("activity-payload:")),
            Class::Resources => key[0].as_str().is_some_and(|s| s.starts_with("api:")),
            _ => true,
        };
        if !(valid && prefix_valid) {
            return Err(Error::StorageUnavailable(
                "Invalid retention cursor; original database preserved.".into(),
            ));
        }
    }
    let mut ceiling = match saved.as_ref() {
        Some((_, ceiling)) => Some(ceiling.clone()),
        None => fence(tx, class, cutoff)?,
    };
    let mut rows = match ceiling.as_deref() {
        Some(ceiling) => candidates(
            tx,
            class,
            saved.as_ref().map(|(p, _)| p.as_str()),
            cutoff,
            ceiling,
        )?,
        None => Vec::new(),
    };
    if rows.is_empty() && saved.is_some() {
        ceiling = fence(tx, class, cutoff)?;
        rows = match ceiling.as_deref() {
            Some(ceiling) => candidates(tx, class, None, cutoff, ceiling)?,
            None => Vec::new(),
        };
    }
    let before = tx.total_changes();
    let mut visible = 0;
    let mut last = None;
    let mut processed = 0;
    for (key, eligible) in &rows {
        // Reserve cursor bookkeeping even if this is the final partial window.
        if *eligible && visible + class.cost() + 1 > *allowance {
            break;
        }
        if *eligible {
            let k: Value = serde_json::from_str(key)?;
            visible += match class {
                Class::Orphans | Class::Resources => tx.execute(
                    if matches!(class, Class::Orphans) {
                        "DELETE FROM content WHERE key=?"
                    } else {
                        "DELETE FROM resource_state WHERE key=?"
                    },
                    [k[0].as_str()],
                )?,
                Class::Validators => {
                    tx.execute("DELETE FROM api_validator WHERE url=?", [k[0].as_str()])?
                }
                Class::States => tx.execute(
                    "DELETE FROM check_state WHERE pr_id=? AND commit_sha=?",
                    params![k[1].as_str(), k[2].as_str()],
                )?,
                Class::Checks => {
                    let updated=tx.execute("UPDATE check_state SET complete=0,error='Cached checks evicted' WHERE pr_id=? AND commit_sha=? AND (complete!=0 OR error IS NOT 'Cached checks evicted')",params![k[0].as_str(),k[1].as_str()])?;
                    updated+tx.execute("DELETE FROM check_context WHERE pr_id=? AND commit_sha=? AND id=? AND staged=0",params![k[0].as_str(),k[1].as_str(),k[2].as_str()])?
                }
            };
        }
        processed += 1;
        last = Some(key);
    }
    if processed == rows.len() && rows.len() < WINDOW {
        tx.execute("DELETE FROM retention_cursor WHERE class=?", [class.name()])?;
    } else if let Some(last) = last {
        tx.execute("INSERT INTO retention_cursor VALUES(?,?,?) ON CONFLICT(class) DO UPDATE SET position=excluded.position,ceiling=excluded.ceiling",params![class.name(),last,ceiling])?;
    }
    *allowance -= (tx.total_changes() - before) as usize;
    Ok(visible)
}
#[cfg(test)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Metrics {
    pub rows: usize,
    pub vm: i32,
    pub scans: i32,
    pub sorts: i32,
    pub autoindexes: i32,
}
#[cfg(test)]
thread_local! {static METRICS:std::cell::RefCell<Vec<Metrics>>=const {std::cell::RefCell::new(Vec::new())};}
#[cfg(test)]
fn observe(q: &rusqlite::Statement<'_>, rows: usize) {
    use rusqlite::StatementStatus::*;
    METRICS.with(|m| {
        m.borrow_mut().push(Metrics {
            rows,
            vm: q.get_status(VmStep),
            scans: q.get_status(FullscanStep),
            sorts: q.get_status(Sort),
            autoindexes: q.get_status(AutoIndex),
        })
    });
}
#[cfg(test)]
pub(crate) fn metrics() -> Vec<Metrics> {
    METRICS.with(|m| std::mem::take(&mut *m.borrow_mut()))
}
