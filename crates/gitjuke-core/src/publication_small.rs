//! Proven-small publications retain a single transaction. Preflight uses capped
//! counts/identity lists; larger operations use the durable publication journal.
use super::*;
use rusqlite::Transaction;
pub(super) fn count(c: &Connection, t: &str, filter: &str, args: &str) -> Result<i64> {
    // Every small-path threshold is below 128. Saturation is safe for empty
    // replacements too, and all scoped predicates have native prefix indexes.
    let mut q = c.prepare(&format!(
        "SELECT COUNT(*) FROM(SELECT 1 FROM {t} WHERE {filter} AND ?1 IS NOT NULL LIMIT 128)"
    ))?;
    let out = q.query_row([args], |r| r.get(0))?;
    crate::publication_completion::observe(&q, 1);
    Ok(out)
}

fn ids(c: &Connection, sql: &str, args: &str) -> Result<Vec<String>> {
    let mut q = c.prepare(&format!("SELECT id FROM ({sql}) LIMIT 17"))?;
    let out = q
        .query_map([args], |r| r.get(0))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    crate::publication_completion::observe(&q, out.len());
    Ok(out)
}
fn flags(tx: &Transaction<'_>, ids: &[String]) -> Result<()> {
    for id in ids {
        tx.execute("INSERT INTO watch SELECT ?1,EXISTS(SELECT 1 FROM watch_source WHERE id=?1 AND source='authored'),(EXISTS(SELECT 1 FROM watch_source WHERE id=?1 AND source='direct') OR EXISTS(SELECT 1 FROM watch_source WHERE id=?1 AND source LIKE 'team:%')),EXISTS(SELECT 1 FROM watch_source WHERE id=?1 AND source='selected') WHERE EXISTS(SELECT 1 FROM watch WHERE id=?1) OR EXISTS(SELECT 1 FROM watch_source WHERE id=?1) ON CONFLICT(id) DO UPDATE SET authored=excluded.authored,requested=excluded.requested,selected=excluded.selected",[id])?;
    }
    Ok(())
}
fn locator(tx: &Transaction<'_>, x: &Value) -> Result<()> {
    tx.execute("INSERT INTO watch_locator(id,repo,number,repo_id,state,review,head,merge_sha,epoch,due,last_success,error,purged_generation) SELECT json_extract(?1,'$[0]'),json_extract(?1,'$[1]'),json_extract(?1,'$[2]'),json_extract(?1,'$[3]'),json_extract(?1,'$[4]'),json_extract(?1,'$[5]'),json_extract(?1,'$[6]'),json_extract(?1,'$[7]'),json_extract(?1,'$[8]'),json_extract(?1,'$[9]'),json_extract(?1,'$[10]'),json_extract(?1,'$[11]'),json_extract(?1,'$[12]') WHERE 1 ON CONFLICT(id) DO UPDATE SET repo=excluded.repo,number=excluded.number,repo_id=excluded.repo_id,state=excluded.state,review=excluded.review,head=excluded.head,merge_sha=excluded.merge_sha,epoch=excluded.epoch,due=excluded.due,last_success=excluded.last_success,error=excluded.error,purged_generation=excluded.purged_generation",[x["locator"].to_string()])?;
    Ok(())
}
fn content(tx: &Transaction<'_>, x: &Value) -> Result<()> {
    tx.execute("INSERT INTO content VALUES(?,?,?,?) ON CONFLICT(key) DO UPDATE SET json=excluded.json,fetched=excluded.fetched,accessed=excluded.accessed",params![format!("list:{}",x["pr"]["id"].as_str().unwrap()),x["pr"].to_string(),x["time"].as_i64(),x["time"].as_i64()])?;
    Ok(())
}
fn health(tx: &Transaction<'_>, key: &str, time: u64) -> Result<()> {
    tx.execute("INSERT INTO resource_state VALUES(?,?,1,NULL) ON CONFLICT(key) DO UPDATE SET fetched=excluded.fetched,complete=1,error=NULL",params![key,time as i64])?;
    Ok(())
}
// These operations change no disposable payloads. Keep cache expiry attached to
// payload admission/maintenance rather than making cached selection evict data.
fn metadata(c: &mut Connection, write: impl FnOnce(&Transaction<'_>) -> Result<()>) -> Result<()> {
    admit(c)?;
    let tx = c.transaction()?;
    let before = tx.total_changes();
    write(&tx)?;
    if tx.total_changes() - before > 128 {
        return Err(Error::Safe(
            "Publication mutation allowance exceeded.".into(),
        ));
    }
    #[cfg(test)]
    record(&tx, before);
    tx.commit()?;
    Ok(())
}
pub(super) fn publish(c: &mut Connection, kind: &str, x: &Value) -> Result<bool> {
    #[cfg(test)]
    if FORCE.with(|v| v.get()) {
        return Ok(false);
    }
    let args = x.to_string();
    let time = x["time"].as_u64().unwrap();
    let mut affected = vec![];
    match kind {
        "source_page" => {
            if x["prs"].as_array().unwrap().len() > 60 {
                return Ok(false);
            }
        }
        "source" => {
            if count(
                c,
                "watch_source",
                "source=json_extract(?1,'$.source')",
                &args,
            )? > 8
                || count(
                    c,
                    "source_stage",
                    "source=json_extract(?1,'$.source')",
                    &args,
                )? > 8
            {
                return Ok(false);
            }
            affected = ids(
                c,
                "SELECT id FROM watch_source WHERE source=json_extract(?1,'$.source') UNION SELECT id FROM source_stage WHERE source=json_extract(?1,'$.source')",
                &args,
            )?;
        }
        "selection" => {
            if count(c, "watch_source", "source='selected'", &args)? > 16
                || count(
                    c,
                    "watch_locator",
                    "repo=json_extract(?1,'$.repo') AND number=json_extract(?1,'$.number')",
                    &args,
                )? > 16
            {
                return Ok(false);
            }
            affected = ids(
                c,
                "SELECT id FROM watch_source WHERE source='selected' UNION SELECT id FROM watch_locator WHERE repo=json_extract(?1,'$.repo') AND number=json_extract(?1,'$.number') AND state='open' AND json_extract(?1,'$.open')",
                &args,
            )?;
            if affected.len() > 16 {
                return Ok(false);
            }
        }
        "selected" => affected.push(x["pr"]["id"].as_str().unwrap().into()),
        "head" => {
            let mut changes = 0;
            if x["changed"] == true {
                changes += count(
                    c,
                    "check_context",
                    "pr_id=json_extract(?1,'$.pr.id') AND staged=1",
                    &args,
                )?;
                changes += count(c, "check_state", "pr_id=json_extract(?1,'$.pr.id')", &args)?;
            }
            if x["pr"]["state"] != "open" {
                changes += count(c, "watch_source", "id=json_extract(?1,'$.pr.id')", &args)?;
                affected.push(x["pr"]["id"].as_str().unwrap().into());
            }
            if changes > 24 {
                return Ok(false);
            }
        }
        "checks" => {
            let staged = count(
                c,
                "check_context",
                "pr_id=json_extract(?1,'$.id') AND commit_sha=json_extract(?1,'$.commit') AND staged=1",
                &args,
            )?;
            let incoming = x["contexts"].as_array().unwrap().len() as i64;
            let mut changes = incoming + 2 + if x["first"] == true { staged } else { 0 };
            if x["complete"] == true {
                changes += count(
                    c,
                    "check_context",
                    "pr_id=json_extract(?1,'$.id') AND commit_sha=json_extract(?1,'$.commit') AND staged=0",
                    &args,
                )?;
                changes += if x["first"] == true {
                    incoming
                } else {
                    staged + incoming
                };
            }
            if changes > 28 {
                return Ok(false);
            }
        }
        "teams_page" => {
            let incoming = x["rows"].as_array().unwrap().len() as i64;
            let old = if x["first"] == true {
                count(c, "teams_stage", "1", &args)?
            } else {
                0
            };
            if incoming + old + 1 > 112 {
                return Ok(false);
            }
        }
        "teams" => {
            if count(c, "teams", "1", &args)? > 8 || count(c, "teams_stage", "1", &args)? > 8 {
                return Ok(false);
            }
            if count(c, "watch_source", "source LIKE 'team:%'", &args)? > 16
                || count(c, "pending", "key LIKE 'discover:team:%'", &args)? > 16
            {
                return Ok(false);
            }
            affected = ids(
                c,
                "SELECT DISTINCT id FROM watch_source WHERE source LIKE 'team:%' AND substr(source,6) NOT IN(SELECT id FROM teams_stage) AND ?1 IS NOT NULL",
                &args,
            )?;
        }
        _ => return Ok(false),
    }
    let write = |tx: &Transaction<'_>| {
        match kind {
            "source_page" => {
                for p in x["prs"].as_array().unwrap() {
                    tx.execute("INSERT INTO source_stage VALUES(?,?,?) ON CONFLICT(source,id) DO UPDATE SET json=excluded.json",params![x["source"].as_str(),p["id"].as_str(),p.to_string()])?;
                }
            }
            "source" => {
                let source = x["source"].as_str().unwrap();
                tx.execute("DELETE FROM watch_source WHERE source=?", [source])?;
                tx.execute(
                    "INSERT INTO watch_source SELECT id,source FROM source_stage WHERE source=?",
                    [source],
                )?;
                tx.execute("INSERT INTO watch_locator(id,repo,number,repo_id) SELECT id,json_extract(json,'$.repo'),json_extract(json,'$.number'),json_extract(json,'$.repoId') FROM source_stage WHERE source=? ON CONFLICT(id) DO UPDATE SET repo=excluded.repo,number=excluded.number,repo_id=COALESCE(excluded.repo_id,repo_id)",[source])?;
                tx.execute("INSERT INTO content SELECT 'list:'||id,json,?,? FROM source_stage WHERE source=? ON CONFLICT(key) DO UPDATE SET json=excluded.json,fetched=excluded.fetched,accessed=excluded.accessed",params![time as i64,time as i64,source])?;
                flags(tx, &affected)?;
                health(tx, source, time)?;
                tx.execute("DELETE FROM source_stage WHERE source=?", [source])?;
            }
            "selection" => {
                if x["number"].as_u64().unwrap() > 0 {
                    tx.execute("INSERT INTO progress VALUES('selection',?) ON CONFLICT(key) DO UPDATE SET value=excluded.value",[json!([x["repo"],x["number"]]).to_string()])?;
                }
                tx.execute("DELETE FROM watch_source WHERE source='selected'", [])?;
                if x["open"] == true {
                    tx.execute("INSERT INTO watch_source SELECT id,'selected' FROM watch_locator WHERE repo=? AND number=? AND state='open'",params![x["repo"].as_str(),x["number"].as_i64()])?;
                }
                flags(tx, &affected)?;
            }
            "selected" | "head" => {
                let id = x["pr"]["id"].as_str().unwrap();
                locator(tx, x)?;
                content(tx, x)?;
                if kind == "head" {
                    if x["changed"] == true {
                        tx.execute("DELETE FROM check_context WHERE pr_id=? AND staged=1", [id])?;
                        tx.execute(
                            "UPDATE check_state SET complete=0,error='Old revision' WHERE pr_id=?",
                            [id],
                        )?;
                    }
                    if x["pr"]["state"] != "open" {
                        tx.execute("DELETE FROM watch_source WHERE id=?", [id])?;
                    }
                } else {
                    tx.execute(
                        "DELETE FROM watch_source WHERE id=? AND source='selected'",
                        [id],
                    )?;
                    if x["open"] == true && x["pr"]["state"] == "open" {
                        tx.execute("INSERT INTO watch_source VALUES(?,'selected')", [id])?;
                    }
                }
                flags(tx, &affected)?;
            }
            "checks" => {
                let id = x["id"].as_str().unwrap();
                let sha = x["commit"].as_str().unwrap();
                if x["first"] == true {
                    tx.execute(
                        "DELETE FROM check_context WHERE pr_id=? AND commit_sha=? AND staged=1",
                        params![id, sha],
                    )?;
                }
                for ctx in x["contexts"].as_array().unwrap() {
                    tx.execute("INSERT INTO check_context VALUES(?,?,?,?,1) ON CONFLICT(pr_id,commit_sha,id,staged) DO UPDATE SET json=excluded.json",params![id,sha,ctx["id"].as_str(),ctx.to_string()])?;
                }
                if x["complete"] == true {
                    let actual:i64=tx.query_row("SELECT COUNT(*) FROM check_context WHERE pr_id=? AND commit_sha=? AND staged=1",params![id,sha],|r|r.get(0))?;
                    if actual != x["total"].as_i64().unwrap() {
                        return Err(Error::Safe("Check contexts changed during pagination; previous successful results are retained.".into()));
                    }
                    tx.execute(
                        "DELETE FROM check_context WHERE pr_id=? AND commit_sha=? AND staged=0",
                        params![id, sha],
                    )?;
                    tx.execute("UPDATE check_context SET staged=0 WHERE pr_id=? AND commit_sha=? AND staged=1",params![id,sha])?;
                    tx.execute("INSERT INTO check_state VALUES(?,?,?,1,NULL,?) ON CONFLICT(pr_id,commit_sha) DO UPDATE SET fetched=excluded.fetched,accessed=excluded.accessed,complete=1,error=NULL",params![id,sha,time as i64,time as i64])?;
                } else {
                    tx.execute("INSERT INTO check_state(pr_id,commit_sha,fetched,complete,error) VALUES(?,?,NULL,0,'Refreshing; traversal incomplete') ON CONFLICT(pr_id,commit_sha) DO UPDATE SET error=excluded.error",params![id,sha])?;
                }
            }
            "teams_page" => {
                if x["first"] == true {
                    tx.execute("DELETE FROM teams_stage", [])?;
                }
                for t in x["rows"].as_array().unwrap() {
                    tx.execute("INSERT INTO teams_stage VALUES(?,?) ON CONFLICT(id) DO UPDATE SET slug=excluded.slug",params![t["node_id"].as_str(),format!("{}/{}",t["organization"]["login"].as_str().unwrap(),t["slug"].as_str().unwrap())])?;
                }
            }
            "teams" => {
                tx.execute("DELETE FROM watch_source WHERE source LIKE 'team:%' AND substr(source,6) NOT IN(SELECT id FROM teams_stage)",[])?;
                tx.execute("DELETE FROM pending WHERE key LIKE 'discover:team:%' AND substr(key,15) NOT IN(SELECT id FROM teams_stage)",[])?;
                tx.execute("DELETE FROM teams", [])?;
                tx.execute("INSERT INTO teams SELECT * FROM teams_stage", [])?;
                tx.execute("DELETE FROM teams_stage", [])?;
                flags(tx, &affected)?;
                health(tx, "teams", time)?;
            }
            _ => unreachable!(),
        }
        bump(tx)?;
        Ok(())
    };
    if matches!(kind, "selection" | "teams" | "teams_page") {
        metadata(c, write)?;
    } else {
        crate::content_budget::publish_bounded(c, time, write)?;
    }
    Ok(true)
}
use serde_json::json;
#[cfg(test)]
thread_local! {pub(super) static FORCE:std::cell::Cell<bool>=const{std::cell::Cell::new(false)};}
