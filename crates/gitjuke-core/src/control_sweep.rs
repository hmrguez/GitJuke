//! Durable, idempotent control-plane sweeps. No published payload or durable
//! Activity history is removed. The marker blocks scheduling until completion.
use crate::{Error, Result, store::storage_healthy};
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::Value;
use std::path::Path;

fn admit(c: &Connection) -> Result<()> {
    storage_healthy(
        c.path()
            .and_then(|p| Path::new(p).parent())
            .unwrap_or(Path::new("/")),
    )
}
pub(crate) fn pending(c: &Connection) -> Result<bool> {
    Ok(
        c.query_row("SELECT EXISTS(SELECT 1 FROM control_sweep)", [], |r| {
            r.get(0)
        })?,
    )
}
pub(crate) fn run(c: &mut Connection, kind: &str, args: Value) -> Result<()> {
    resume(c)?;
    // Persist the exact scope before touching any rows. Repeating a failed call
    // can finish the old scope before starting the requested scope.
    admit(c)?;
    #[cfg(test)]
    let before = c.total_changes();
    c.execute(
        "INSERT INTO control_sweep VALUES(1,?,?)",
        params![kind, args.to_string()],
    )?;
    #[cfg(test)]
    crate::publication::record(c, before);
    resume(c)
}
pub(crate) fn resume(c: &mut Connection) -> Result<()> {
    loop {
        let work: Option<(String, String)> = c
            .query_row("SELECT kind,args FROM control_sweep WHERE id=1", [], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .optional()?;
        let Some((kind, args)) = work else {
            return Ok(());
        };
        let scope: Value = serde_json::from_str(&args).map_err(|_| {
            Error::StorageUnavailable(
                "Invalid control sweep scope; original database preserved.".into(),
            )
        })?;
        if kind == "activity" && !(scope["repo"].is_string() && scope["number"].as_u64().is_some())
        {
            return Err(Error::StorageUnavailable(
                "Invalid Activity cleanup scope; original database preserved.".into(),
            ));
        }
        let sql = match kind.as_str() {
            "heads" => {
                "DELETE FROM pending WHERE rowid IN(SELECT rowid FROM pending WHERE json_type(job,'$.Heads') IS NOT NULL ORDER BY rowid LIMIT 127)"
            }
            "activity" => {
                "DELETE FROM activity_stage WHERE rowid IN(SELECT rowid FROM activity_stage WHERE repo=json_extract(?1,'$.repo') AND number=json_extract(?1,'$.number') ORDER BY rowid LIMIT 63)"
            }
            "manual" => {
                "UPDATE watch_locator SET due=0 WHERE rowid IN(SELECT rowid FROM watch_locator WHERE due!=0 AND id IN(SELECT id FROM watch_source) ORDER BY rowid LIMIT 127)"
            }
            _ => {
                return Err(Error::StorageUnavailable(
                    "Unknown control sweep; original database preserved.".into(),
                ));
            }
        };
        admit(c)?;
        let tx = c.transaction()?;
        let before = tx.total_changes();
        let changed = if kind == "activity" {
            tx.execute(sql, [args])?
        } else {
            tx.execute(sql, [])?
        };
        if changed < if kind == "activity" { 63 } else { 127 } {
            tx.execute("DELETE FROM control_sweep WHERE id=1", [])?;
        }
        if tx.total_changes() - before > 128 {
            return Err(Error::StorageUnavailable(
                "Control sweep mutation allowance exceeded.".into(),
            ));
        }
        #[cfg(test)]
        crate::publication::record(&tx, before);
        #[cfg(test)]
        point("sweep_before");
        tx.commit()?;
        #[cfg(test)]
        point("sweep_after");
    }
}
#[cfg(test)]
pub(crate) fn point(phase: &str) {
    if std::env::var("GITJUKE_TEST_MAINTENANCE_PHASE").as_deref() == Ok(phase) {
        thread_local! { static HITS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) }; }
        let hit = HITS.with(|v| {
            let n = v.get() + 1;
            v.set(n);
            n
        });
        if hit
            != std::env::var("GITJUKE_TEST_MAINTENANCE_HIT")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(1)
        {
            return;
        }
        std::fs::write(
            std::env::var_os("GITJUKE_TEST_MAINTENANCE_MARKER").unwrap(),
            phase,
        )
        .unwrap();
        loop {
            std::thread::park();
        }
    }
}
