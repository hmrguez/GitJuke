//! Bounded physical application with a durable old-snapshot overlay. Readers
//! switch to the new snapshot only at the final two-row visibility commit.
use crate::{
    Error, Result, now,
    store::{Store, bump, storage_healthy},
};
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::Value;
use std::path::Path;

const ROWS: usize = 12;
struct Table {
    name: &'static str,
    columns: &'static [&'static str],
    keys: &'static [usize],
}
const TABLES: &[Table] = &[
    Table {
        name: "activity",
        columns: &[
            "id", "repo", "number", "kind", "occurred", "eligible", "reason", "created",
        ],
        keys: &[0],
    },
    Table {
        name: "seen",
        columns: &["source_id", "seen"],
        keys: &[0],
    },
    Table {
        name: "dedup",
        columns: &["source_id"],
        keys: &[0],
    },
    Table {
        name: "activity_target",
        columns: &["repo", "number", "due", "done"],
        keys: &[0, 1],
    },
    Table {
        name: "activity_cycle",
        columns: &["id", "mode", "boundary", "notifications_done"],
        keys: &[0],
    },
    Table {
        name: "activity_intent",
        columns: &["id", "kind", "state", "created"],
        keys: &[0],
    },
    Table {
        name: "intent_member",
        columns: &["intent", "activity"],
        keys: &[0, 1],
    },
    Table {
        name: "activity_stage",
        columns: &["repo", "number", "id", "json"],
        keys: &[0, 1, 2],
    },
    Table {
        name: "progress",
        columns: &["key", "value"],
        keys: &[0],
    },
    Table {
        name: "watch_source",
        columns: &["id", "source"],
        keys: &[0, 1],
    },
    Table {
        name: "watch_locator",
        columns: &[
            "id",
            "repo",
            "number",
            "repo_id",
            "state",
            "review",
            "head",
            "merge_sha",
            "epoch",
            "due",
            "last_success",
            "error",
            "purged_generation",
        ],
        keys: &[0],
    },
    Table {
        name: "watch",
        columns: &["id", "authored", "requested", "selected"],
        keys: &[0],
    },
    Table {
        name: "content",
        columns: &["key", "json", "fetched", "accessed"],
        keys: &[0],
    },
    Table {
        name: "resource_state",
        columns: &["key", "fetched", "complete", "error"],
        keys: &[0],
    },
    Table {
        name: "check_context",
        columns: &["pr_id", "commit_sha", "id", "json", "staged"],
        keys: &[0, 1, 2, 4],
    },
    Table {
        name: "check_state",
        columns: &[
            "pr_id",
            "commit_sha",
            "fetched",
            "complete",
            "error",
            "accessed",
        ],
        keys: &[0, 1],
    },
    Table {
        name: "teams",
        columns: &["id", "slug"],
        keys: &[0],
    },
    Table {
        name: "teams_stage",
        columns: &["id", "slug"],
        keys: &[0],
    },
    Table {
        name: "source_stage",
        columns: &["source", "id", "json"],
        keys: &[0, 1],
    },
    Table {
        name: "pending",
        columns: &[
            "seq",
            "key",
            "job",
            "manual",
            "foreground",
            "at",
            "resource",
        ],
        keys: &[0],
    },
];
fn table(name: &str) -> Result<&'static Table> {
    TABLES.iter().find(|t| t.name == name).ok_or_else(|| {
        Error::Safe("Invalid publication table; original database preserved.".into())
    })
}

fn key(t: &Table, alias: &str) -> String {
    format!(
        "json_array({})",
        t.keys
            .iter()
            .map(|i| format!("{alias}.{}", t.columns[*i]))
            .collect::<Vec<_>>()
            .join(",")
    )
}
fn row(t: &Table, alias: &str) -> String {
    format!(
        "json_array({})",
        t.columns
            .iter()
            .map(|c| format!("{alias}.{c}"))
            .collect::<Vec<_>>()
            .join(",")
    )
}
fn pred(t: &Table, alias: &str) -> String {
    t.keys
        .iter()
        .enumerate()
        .map(|(i, c)| format!("{alias}.{}=json_extract(?1,'$[{i}]')", t.columns[*c]))
        .collect::<Vec<_>>()
        .join(" AND ")
}
fn admit(c: &Connection) -> Result<()> {
    storage_healthy(
        c.path()
            .and_then(|s| Path::new(s).parent())
            .unwrap_or(Path::new("/")),
    )
}
pub(crate) fn pending(c: &Connection) -> Result<bool> {
    Ok(
        c.query_row("SELECT EXISTS(SELECT 1 FROM main.publication)", [], |r| {
            r.get(0)
        })?,
    )
}

pub(crate) fn unprotect(c: &Connection) -> Result<()> {
    let exists: bool = c.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_temp_schema WHERE type='view')",
        [],
        |r| r.get(0),
    )?;
    if !exists {
        return Ok(());
    }
    for t in TABLES {
        c.execute_batch(&format!("DROP VIEW IF EXISTS temp.{}", t.name))?;
    }
    Ok(())
}
pub(crate) fn protect(c: &Connection) -> Result<()> {
    if !pending(c)? {
        return Ok(());
    }
    for t in TABLES {
        let oldcols = t
            .columns
            .iter()
            .enumerate()
            .map(|(i, col)| format!("json_extract(r.old_json,'$[{i}]') AS {col}"))
            .collect::<Vec<_>>()
            .join(",");
        c.execute_batch(&format!("CREATE TEMP VIEW IF NOT EXISTS {name} AS SELECT b.* FROM main.{name} b WHERE NOT EXISTS(SELECT 1 FROM main.publication WHERE phase!='committed') OR NOT EXISTS(SELECT 1 FROM main.publication_row r WHERE r.tbl='{name}' AND r.k={k}) UNION ALL SELECT {oldcols} FROM main.publication_row r WHERE r.tbl='{name}' AND r.old_json IS NOT NULL AND EXISTS(SELECT 1 FROM main.publication WHERE phase!='committed')",name=t.name,k=key(t,"b")))?;
    }
    Ok(())
}
impl Store {
    pub(crate) fn publication_request(
        &mut self,
        kind: &str,
        input: &impl serde::Serialize,
    ) -> Result<Option<String>> {
        use sha2::{Digest, Sha256};
        let mut hash = Sha256::new();
        hash.update(kind.as_bytes());
        hash.update([0]);
        hash.update(serde_json::to_vec(input)?);
        let request = format!("{:x}", hash.finalize());
        let retry:bool=self.conn.query_row("SELECT EXISTS(SELECT 1 FROM main.publication WHERE kind=? AND json_extract(args,'$.request')=?)",params![kind,request],|r|r.get(0))?;
        self.publication_ready()?;
        Ok((!retry).then_some(request))
    }
    pub(crate) fn publication_ready(&mut self) -> Result<()> {
        let result =
            resume(&mut self.conn).and_then(|()| crate::control_sweep::resume(&mut self.conn));
        if result.is_err() {
            self.cache_writes = false;
        }
        result
    }
    pub(crate) fn apply_publication(&mut self, kind: &str, args: Value) -> Result<()> {
        let result = (|| {
            if pending(&self.conn)? {
                resume(&mut self.conn)?;
            }
            validate(kind, &args, false)?;
            if small::publish(&mut self.conn, kind, &args)? {
                return Ok(());
            }
            crate::content_budget::publish_bounded(&mut self.conn, now(), |tx| {
                tx.execute(
                    "INSERT INTO publication(id,kind,args,phase) VALUES(1,?,?,'planning')",
                    params![kind, args.to_string()],
                )?;
                Ok(())
            })?;
            #[cfg(test)]
            point("begun");
            resume(&mut self.conn)
        })();
        if result.is_err() {
            self.cache_writes = false;
            protect(&self.conn)?;
        }
        result
    }
}
#[cfg(test)]
struct Plan {
    tbl: &'static str,
    sql: String,
}
#[cfg(test)]
fn plan(tbl: &'static str, sql: impl Into<String>) -> Plan {
    Plan {
        tbl,
        sql: sql.into(),
    }
}
#[cfg(test)]
fn a(args: &Value, name: &str) -> String {
    match args[name].as_str() {
        Some(s) => lit(s),
        None => args[name].to_string(),
    }
}
#[cfg(test)]
fn lit(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}
#[cfg(test)]
fn flags_plan() -> Plan {
    plan(
        "watch",
        "WITH ns AS (SELECT s.* FROM watch_source s WHERE NOT EXISTS(SELECT 1 FROM publication_row r WHERE r.tbl='watch_source' AND r.k=json_array(s.id,s.source)) UNION ALL SELECT json_extract(new_json,'$[0]'),json_extract(new_json,'$[1]') FROM publication_row WHERE tbl='watch_source' AND new_json IS NOT NULL), ids AS(SELECT id FROM watch UNION SELECT id FROM ns) SELECT json_array(id),json_array(id,EXISTS(SELECT 1 FROM ns WHERE ns.id=ids.id AND source='authored'),EXISTS(SELECT 1 FROM ns WHERE ns.id=ids.id AND (source='direct' OR source LIKE 'team:%')),EXISTS(SELECT 1 FROM ns WHERE ns.id=ids.id AND source='selected')),NULL FROM ids",
    )
}
#[cfg(test)]
fn health(k: &str, time: u64) -> Plan {
    plan(
        "resource_state",
        format!("SELECT json_array({k}),json_array({k},{time},1,NULL),NULL"),
    )
}
#[cfg(test)]
fn plans(kind: &str, x: &Value) -> Result<Vec<Plan>> {
    let time = x["time"].as_u64().unwrap();
    let mut out = vec![];
    match kind {
        "activity" | "activity_summary" => return activity::plans(kind, x),
        "source_page" => {
            let source = a(x, "source");
            out.push(plan("source_stage",format!("SELECT json_array({source},json_extract(value,'$.id')),json_array({source},json_extract(value,'$.id'),NULL),json_array('args',fullkey) FROM json_each(?2,'$.prs')")));
        }
        "source" => {
            let source = a(x, "source");
            out.push(plan("watch_source",format!("WITH ids AS(SELECT id FROM watch_source WHERE source={source} UNION SELECT id FROM source_stage WHERE source={source}) SELECT json_array(id,{source}),CASE WHEN EXISTS(SELECT 1 FROM source_stage s WHERE s.source={source} AND s.id=ids.id) THEN json_array(id,{source}) END,NULL FROM ids")));
            out.push(plan("watch_locator",format!("SELECT json_array(s.id),json_array(s.id,json_extract(s.json,'$.repo'),json_extract(s.json,'$.number'),COALESCE(json_extract(s.json,'$.repoId'),l.repo_id),COALESCE(l.state,'open'),l.review,l.head,l.merge_sha,COALESCE(l.epoch,0),COALESCE(l.due,0),l.last_success,l.error,COALESCE(l.purged_generation,0)),NULL FROM source_stage s LEFT JOIN watch_locator l ON l.id=s.id WHERE s.source={source}")));
            out.push(plan("content",format!("SELECT json_array('list:'||id),json_array('list:'||id,NULL,{time},{time}),json_array('source_stage',source,id) FROM source_stage WHERE source={source}")));
            out.push(flags_plan());
            out.push(health(&source, time));
            // Content application consumes source staging after all metadata plans exist.
        }
        "selection" => {
            if x["number"].as_u64().unwrap_or(0) > 0 {
                out.push(plan("progress","SELECT json_array('selection'),json_array('selection',json_array(json_extract(?2,'$.repo'),json_extract(?2,'$.number'))),NULL"));
            }
            let repo = a(x, "repo");
            let num = a(x, "number");
            let open = x["open"].as_bool().unwrap();
            out.push(plan("watch_source",format!("WITH ids AS(SELECT id FROM watch_source WHERE source='selected' UNION SELECT id FROM watch_locator WHERE repo={repo} AND number={num} AND state='open' AND {}) SELECT json_array(id,'selected'),CASE WHEN {} AND EXISTS(SELECT 1 FROM watch_locator l WHERE l.id=ids.id AND repo={repo} AND number={num} AND state='open') THEN json_array(id,'selected') END,NULL FROM ids",i32::from(open),i32::from(open))));
            out.push(flags_plan());
        }
        "selected" | "head" => {
            let p = &x["pr"];
            let id = lit(p["id"].as_str().unwrap());
            let closed = p["state"] != "open";

            if kind == "head" {
                let changed = x["changed"].as_bool().unwrap();
                if changed {
                    out.push(plan("check_context",format!("SELECT json_array(pr_id,commit_sha,id,staged),NULL,NULL FROM check_context WHERE pr_id={id} AND staged=1")));
                    out.push(plan("check_state",format!("SELECT json_array(pr_id,commit_sha),json_array(pr_id,commit_sha,fetched,0,'Old revision',accessed),NULL FROM check_state WHERE pr_id={id}")));
                }
                if closed {
                    out.push(plan(
                        "watch_source",
                        format!(
                            "SELECT json_array(id,source),NULL,NULL FROM watch_source WHERE id={id}"
                        ),
                    ));
                }
            } else {
                out.push(plan(
                    "watch_source",
                    format!(
                        "SELECT json_array({id},'selected'),{} ,NULL",
                        if x["open"] == true && !closed {
                            format!("json_array({id},'selected')")
                        } else {
                            "NULL".into()
                        }
                    ),
                ));
            }
            // Locator values are fixed at acceptance, including one incremented epoch.
            out.push(plan(
                "watch_locator",
                format!("SELECT json_array({id}),json_extract(?2,'$.locator'),NULL"),
            ));
            out.push(plan("content",format!("SELECT json_array('list:'||{id}),json_array('list:'||{id},NULL,{time},{time}),json_array('args','$.pr')")));
            if kind == "selected" || closed {
                out.push(flags_plan());
            }
        }
        "teams_page" => {
            if x["first"] == true {
                out.push(plan("teams_stage","SELECT json_array(id),NULL,NULL FROM teams_stage WHERE id NOT IN(SELECT json_extract(value,'$.node_id') FROM json_each(?2,'$.rows'))"));
            }
            out.push(plan("teams_stage","SELECT json_array(json_extract(value,'$.node_id')),json_array(json_extract(value,'$.node_id'),json_extract(value,'$.organization.login')||'/'||json_extract(value,'$.slug')),NULL FROM json_each(?2,'$.rows')"));
        }
        "teams" => {
            out.push(plan("watch_source","SELECT json_array(id,source),NULL,NULL FROM watch_source WHERE source LIKE 'team:%' AND substr(source,6) NOT IN(SELECT id FROM teams_stage)"));
            out.push(plan("pending","SELECT json_array(seq),NULL,NULL FROM pending WHERE key LIKE 'discover:team:%' AND substr(key,15) NOT IN(SELECT id FROM teams_stage)"));
            out.push(plan("teams","WITH ids AS(SELECT id FROM teams UNION SELECT id FROM teams_stage) SELECT json_array(id),(SELECT json_array(id,slug) FROM teams_stage s WHERE s.id=ids.id),NULL FROM ids"));
            out.push(flags_plan());
            out.push(health("'teams'", time));
            out.push(plan(
                "teams_stage",
                "SELECT json_array(id),NULL,NULL FROM teams_stage",
            ));
        }
        "checks" => {
            let id = a(x, "id");
            let sha = a(x, "commit");
            let first = x["first"] == true;
            let complete = x["complete"] == true;
            let incoming =
                "SELECT json_extract(value,'$.id') id,fullkey path FROM json_each(?2,'$.contexts')";
            let fresh = format!("SELECT id,json_array('args',path) ref FROM ({incoming})");
            let all = if first {
                fresh
            } else {
                format!(
                    "SELECT id,json_array('check_context',pr_id,commit_sha,id,1) ref FROM check_context WHERE pr_id={id} AND commit_sha={sha} AND staged=1 AND id NOT IN(SELECT id FROM ({incoming})) UNION ALL {fresh}"
                )
            };
            if complete {
                out.push(plan("check_state",format!("SELECT json_array({id},{sha}),json_array({id},{sha},{time},0,'Refreshing; publication incomplete',{time}),NULL")));
                out.push(plan("check_context",format!("WITH new AS({all}),ids AS(SELECT id FROM new UNION SELECT id FROM check_context WHERE pr_id={id} AND commit_sha={sha} AND staged=0) SELECT json_array({id},{sha},id,0),CASE WHEN EXISTS(SELECT 1 FROM new WHERE new.id=ids.id) THEN json_array({id},{sha},id,NULL,0) END,(SELECT ref FROM new WHERE new.id=ids.id) FROM ids")));
                out.push(plan("check_context",format!("SELECT json_array(pr_id,commit_sha,id,staged),NULL,NULL FROM check_context WHERE pr_id={id} AND commit_sha={sha} AND staged=1")));
            } else {
                if first {
                    out.push(plan("check_context",format!("SELECT json_array(pr_id,commit_sha,id,staged),NULL,NULL FROM check_context WHERE pr_id={id} AND commit_sha={sha} AND staged=1 AND id NOT IN(SELECT id FROM ({incoming}))")));
                }
                out.push(plan("check_context",format!("SELECT json_array({id},{sha},id,1),json_array({id},{sha},id,NULL,1),json_array('args',path) FROM ({incoming})")));
                out.push(plan("check_state",format!("SELECT json_array({id},{sha}),json_array({id},{sha},s.fetched,COALESCE(s.complete,0),'Refreshing; traversal incomplete',COALESCE(s.accessed,0)),NULL FROM (SELECT 1) LEFT JOIN check_state s ON s.pr_id={id} AND s.commit_sha={sha}")));
            }
        }
        _ => return Err(Error::Safe("Unknown publication.".into())),
    }
    Ok(out)
}

pub(crate) fn resume(c: &mut Connection) -> Result<()> {
    if !pending(c)? {
        return Ok(());
    }
    unprotect(c)?;
    let result = (|| {
        while pending(c)? {
            batch(c)?;
        }
        Ok(())
    })();
    match result {
        Ok(()) => Ok(()),
        Err(e) => {
            protect(c)?;
            Err(match e {
                Error::Storage(_) | Error::StorageUnavailable(_) => e,
                _ => Error::StorageUnavailable(format!(
                    "Interrupted publication requires storage repair: {e}"
                )),
            })
        }
    }
}
pub(crate) fn application_sql() -> String {
    format!(
        "SELECT seq,tbl,k,new_json,reference FROM publication_row INDEXED BY publication_unapplied WHERE applied=0 ORDER BY seq LIMIT {ROWS}"
    )
}
fn batch(c: &mut Connection) -> Result<usize> {
    admit(c)?;
    let (kind, args, phase, step, after, ceiling): (
        String,
        String,
        String,
        i64,
        Option<i64>,
        Option<i64>,
    ) = c.query_row(
        "SELECT kind,args,phase,step,scan_after,scan_ceiling FROM main.publication",
        [],
        |r| {
            Ok((
                r.get(0)?,
                r.get(1)?,
                r.get(2)?,
                r.get(3)?,
                r.get(4)?,
                r.get(5)?,
            ))
        },
    )?;
    let x: Value = serde_json::from_str(&args)?;
    validate(&kind, &x, phase != "planning")?;
    let before = c.total_changes();
    if !matches!(phase.as_str(), "planning" | "applying" | "committed") {
        return Err(Error::StorageUnavailable(
            "Invalid publication phase; original database preserved.".into(),
        ));
    }
    if phase == "planning" {
        if planning::finished(&kind, &x, step)? {
            if after.is_some() || ceiling.is_some() {
                return Err(Error::StorageUnavailable(
                    "Invalid completed planning cursor.".into(),
                ));
            }
            let tx = c.transaction()?;
            tx.execute("UPDATE publication SET phase='applying'", [])?;
            #[cfg(test)]
            record(&tx, before);
            crate::shared_storage::bounded(&tx, before)?;
            tx.commit()?;
        } else {
            crate::content_budget::publish_bounded(c, now(), |tx| {
                planning::prepare(tx, &kind, &x, step, after, ceiling)
            })?;
            #[cfg(test)]
            point("cursor_after");
        }
    } else if phase == "applying" {
        let rows = {
            let mut q = c.prepare(&application_sql())?;
            q.query_map([], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, Option<String>>(3)?,
                    r.get::<_, Option<String>>(4)?,
                ))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?
        };
        if rows.is_empty() {
            let tx = c.transaction()?;
            if kind == "checks" && x["complete"] == true {
                let count = crate::publication_completion::published_count(
                    &tx,
                    x["id"].as_str().unwrap(),
                    x["commit"].as_str().unwrap(),
                )?;
                tx.execute(
                    "UPDATE check_state SET complete=?,error=? WHERE pr_id=? AND commit_sha=?",
                    params![
                        count == x["total"].as_i64().unwrap(),
                        if count == x["total"].as_i64().unwrap() {
                            None
                        } else {
                            Some("Cached checks evicted")
                        },
                        x["id"].as_str(),
                        x["commit"].as_str()
                    ],
                )?;
            }
            tx.execute("UPDATE publication SET phase='committed'", [])?;
            bump(&tx)?;
            #[cfg(test)]
            record(&tx, before);
            #[cfg(test)]
            point("commit_before");
            crate::shared_storage::bounded(&tx, before)?;
            tx.commit()?;
            #[cfg(test)]
            point("commit_after");
        } else {
            crate::content_budget::publish_bounded(c, now(), |tx| {
                for (seq, tbl, k, n, reference) in &rows {
                    let t = table(tbl)?;
                    if let Some(n) = n {
                        let mut values: Value = serde_json::from_str(n)?;
                        if values.as_array().is_none_or(|a| a.len() != t.columns.len()) {
                            return Err(Error::Safe(
                                "Invalid publication row; original database preserved.".into(),
                            ));
                        }
                        if let Some(reference) = reference {
                            let rf: Value = serde_json::from_str(reference)?;
                            let body:String=match rf[0].as_str(){
        Some("activity_stage")=>tx.query_row("SELECT json_set(json,'$.seen',json(CASE WHEN json_extract(?4,'$.historical') OR json_extract(json,'$.occurred')<=json_extract(?4,'$.baseline') THEN 'true' ELSE 'false' END)) FROM activity_stage WHERE repo=?1 AND number=?2 AND id=?3",params![rf[1].as_str(),rf[2].as_i64(),rf[3].as_str(),args],|r|r.get(0))?,
        Some("source_stage")=>tx.query_row("SELECT json FROM source_stage WHERE source=? AND id=?",params![rf[1].as_str(),rf[2].as_str()],|r|r.get(0))?,
        Some("check_context")=>tx.query_row("SELECT json FROM check_context WHERE pr_id=? AND commit_sha=? AND id=? AND staged=1",params![rf[1].as_str(),rf[2].as_str(),rf[3].as_str()],|r|r.get(0))?,
        Some("args")=>tx.query_row("SELECT json_extract(?1,?2)",params![args,rf[1].as_str()],|r|r.get(0))?,
        _=>return Err(Error::Safe("Invalid publication reference.".into()))
       };
                            values[match tbl.as_str() {
                                "content" => 1,
                                "source_stage" => 2,
                                _ => 3,
                            }] = Value::String(body);
                        }
                        let select = (0..t.columns.len())
                            .map(|i| format!("json_extract(?1,'$[{i}]')"))
                            .collect::<Vec<_>>()
                            .join(",");
                        let updates = t
                            .columns
                            .iter()
                            .map(|col| format!("{col}=excluded.{col}"))
                            .collect::<Vec<_>>()
                            .join(",");
                        tx.execute(&format!("INSERT INTO {}({}) SELECT {select} WHERE 1 ON CONFLICT({}) DO UPDATE SET {updates}",t.name,t.columns.join(","),t.keys.iter().map(|i|t.columns[*i]).collect::<Vec<_>>().join(",")),[values.to_string()])?;
                        if let Some(reference) = reference {
                            let rf: Value = serde_json::from_str(reference)?;
                            if rf[0] == "args" {
                                tx.execute(
                                    "UPDATE publication SET args=json_set(args,?,NULL)",
                                    [rf[1].as_str()],
                                )?;
                            }
                            if rf[0] == "activity_stage" {
                                tx.execute(
                                    "DELETE FROM activity_stage WHERE repo=? AND number=? AND id=?",
                                    params![rf[1].as_str(), rf[2].as_i64(), rf[3].as_str()],
                                )?;
                            }
                            if rf[0] == "source_stage" {
                                tx.execute(
                                    "DELETE FROM source_stage WHERE source=? AND id=?",
                                    params![rf[1].as_str(), rf[2].as_str()],
                                )?;
                            } else if rf[0] == "check_context" {
                                tx.execute("DELETE FROM check_context WHERE pr_id=? AND commit_sha=? AND id=? AND staged=1",params![rf[1].as_str(),rf[2].as_str(),rf[3].as_str()])?;
                            }
                        }
                    } else {
                        tx.execute(
                            &format!("DELETE FROM {} AS b WHERE {}", t.name, pred(t, "b")),
                            [k],
                        )?;
                    }
                    tx.execute("UPDATE publication_row SET applied=1 WHERE seq=?", [seq])?;
                }
                Ok(())
            })?;
            #[cfg(test)]
            point("apply_after");
        }
    } else {
        let tx = c.transaction()?;
        let removed=tx.execute("DELETE FROM publication_row WHERE seq IN(SELECT seq FROM publication_row ORDER BY seq LIMIT 63)",[])?;
        if removed < 63 {
            tx.execute("DELETE FROM publication", [])?;
        }
        #[cfg(test)]
        {
            record(&tx, before);
            point(if removed < 63 {
                "finish_before"
            } else {
                "cleanup_before"
            });
        }
        crate::shared_storage::bounded(&tx, before)?;
        tx.commit()?;
        #[cfg(test)]
        point(if removed < 63 {
            "finish_after"
        } else {
            "cleanup_after"
        });
    }
    let changed = (c.total_changes() - before) as usize;
    // total_changes includes rolled-back admission attempts and independent LRU
    // batches; per-transaction accounting is tested separately at the helper seam.
    Ok(changed)
}
#[cfg(test)]
fn point(phase: &str) {
    if std::env::var("GITJUKE_TEST_PUBLICATION_PHASE").as_deref() == Ok(phase) {
        std::fs::write(
            std::env::var_os("GITJUKE_TEST_PUBLICATION_MARKER").unwrap(),
            phase,
        )
        .unwrap();
        loop {
            std::thread::park();
        }
    }
}

#[cfg(test)]
thread_local! {static COUNTS:std::cell::RefCell<Option<Vec<u64>>>=const {std::cell::RefCell::new(None)};}
#[cfg(test)]
pub(crate) fn record(c: &Connection, before: u64) {
    COUNTS.with(|v| {
        if let Some(counts) = v.borrow_mut().as_mut() {
            counts.push(c.total_changes() - before);
        }
    });
}
#[cfg(test)]
#[path = "publication_tests.rs"]
pub(crate) mod tests;

#[cfg(test)]
pub(crate) fn bounded_point(phase: &str) {
    point(phase);
}

pub(crate) fn discard_stage(c: &mut Connection, tbl: &str, args: &Value) -> Result<()> {
    // Incoming overlaps are native point deletes, not a filtered scan of every
    // stored context/source row. Bulk resets have exact indexed scope prefixes.
    let field = match tbl {
        "source_stage" => Some("prs"),
        "check_context" if args["first"] != true => Some("contexts"),
        "activity_stage" | "check_context" => None,
        _ => return Err(Error::Safe("Invalid staging cleanup table.".into())),
    };
    if let Some(field) = field {
        for chunk in args[field]
            .as_array()
            .ok_or_else(|| Error::Safe("Invalid staging inputs.".into()))?
            .chunks(64)
        {
            admit(c)?;
            let tx = c.transaction()?;
            let before = tx.total_changes();
            for item in chunk {
                if tbl == "source_stage" {
                    tx.execute(
                        "DELETE FROM source_stage WHERE source=? AND id=?",
                        params![args["source"].as_str(), item["id"].as_str()],
                    )?;
                } else {
                    tx.execute("DELETE FROM check_context WHERE pr_id=? AND commit_sha=? AND id=? AND staged=1",params![args["id"].as_str(),args["commit"].as_str(),item["id"].as_str()])?;
                }
            }
            crate::shared_storage::bounded(&tx, before)?;
            #[cfg(test)]
            record(&tx, before);
            tx.commit()?;
        }
        return Ok(());
    }
    loop {
        admit(c)?;
        let tx = c.transaction()?;
        let before = tx.total_changes();
        let removed = if tbl == "activity_stage" {
            tx.execute("DELETE FROM activity_stage WHERE rowid IN(SELECT rowid FROM activity_stage WHERE repo=? AND number=? ORDER BY id LIMIT 64)",params![args["repo"].as_str(),args["number"].as_i64()])?
        } else {
            tx.execute("DELETE FROM check_context WHERE rowid IN(SELECT rowid FROM check_context INDEXED BY check_staged_keys WHERE pr_id=? AND commit_sha=? AND staged=1 ORDER BY id LIMIT 64)",params![args["id"].as_str(),args["commit"].as_str()])?
        };
        crate::shared_storage::bounded(&tx, before)?;
        #[cfg(test)]
        {
            record(&tx, before);
            if tbl == "activity_stage" {
                crate::activity_crash::point("stage_reset_before_commit");
            }
        }
        tx.commit()?;
        #[cfg(test)]
        if tbl == "activity_stage" {
            crate::activity_crash::point("stage_reset_after_commit");
        }
        if removed < 64 {
            return Ok(());
        }
    }
}

fn validate(kind: &str, x: &Value, consumed: bool) -> Result<()> {
    let valid = x["time"].as_u64().is_some()
        && match kind {
            "activity" => {
                x["repo"].is_string()
                    && x["number"].as_u64().is_some()
                    && x["baseline"].as_u64().is_some()
                    && x["historical"].is_boolean()
                    && x["alerts"].is_boolean()
                    && x["boundary"].as_u64().is_some()
                    && x["mode"].is_string()
                    && x["intent"].as_i64().is_some()
            }
            "activity_summary" => x["intent"].as_i64().is_some(),
            "source" => x["source"].is_string(),
            "source_page" => x["source"].is_string() && x["prs"].is_array(),
            "selection" => {
                x["repo"].is_string() && x["number"].as_u64().is_some() && x["open"].is_boolean()
            }
            "selected" | "head" => {
                x["locator"].as_array().is_some_and(|a| a.len() == 13)
                    && x["locator"][0].is_string()
                    && ((consumed && x["pr"].is_null())
                        || (x["pr"]["id"].is_string()
                            && x["pr"]["state"].is_string()
                            && x["locator"][0] == x["pr"]["id"]))
                    && if kind == "head" {
                        x["changed"].is_boolean()
                    } else {
                        x["open"].is_boolean()
                    }
            }
            "teams" => true,
            "teams_page" => x["rows"].is_array() && x["first"].is_boolean(),
            "checks" => {
                x["id"].is_string()
                    && x["commit"].is_string()
                    && x["first"].is_boolean()
                    && x["complete"].is_boolean()
                    && x["total"].as_i64().is_some_and(|n| n >= 0)
                    && x["contexts"].is_array()
            }
            _ => false,
        };
    if valid {
        Ok(())
    } else {
        Err(Error::Safe(
            "Invalid publication input; original database preserved.".into(),
        ))
    }
}

#[path = "publication_small.rs"]
mod small;

#[path = "publication_planning.rs"]
mod planning;

#[cfg(test)]
#[path = "activity_publication.rs"]
mod activity;

#[cfg(test)]
pub(crate) fn start_counts() {
    COUNTS.with(|v| *v.borrow_mut() = Some(vec![]));
}
#[cfg(test)]
pub(crate) fn take_counts() -> Vec<u64> {
    COUNTS.with(|v| v.borrow_mut().take().unwrap())
}

#[cfg(test)]
#[path = "publication_window_tests.rs"]
mod window_tests;
