//! Transactional accounting and snapshot-level shared LRU frontiers. Trigger
//! mutations are included in total_changes and in every covered batch allowance.
use crate::Result;
use rusqlite::{Connection, functions::FunctionFlags};

pub(crate) fn register(c: &Connection) -> Result<()> {
    c.create_scalar_function(
        "gitjuke_input_bytes",
        2,
        FunctionFlags::SQLITE_UTF8 | FunctionFlags::SQLITE_DETERMINISTIC,
        |args| {
            let kind = args.get::<String>(0)?;
            let input = args.get::<String>(1)?;
            // Keep malformed persisted inputs observable as storage protection,
            // including databases opened only for cached navigation.
            Ok(crate::admission_query::input_bytes(&kind, &input).ok())
        },
    )?;
    // REPLACE must fire delete triggers as well as insert triggers.
    c.pragma_update(None, "recursive_triggers", true)?;
    Ok(())
}

/// Worst total mutations per affected row for bounded cleanup statements.
/// A published context changes a charge and one frontier; state time changes
/// touch one frontier. Zero deltas may cost less, never more.
pub(crate) fn cost(sql: &str) -> usize {
    let table = sql.split_whitespace().nth(2).unwrap_or("");
    match table {
        "check_context" => 3,
        "content" | "stage" | "source_stage" | "activity_stage" | "publication_row"
        | "publication" | "check_state" => 2,
        _ if sql.starts_with("UPDATE check_state ") => 2,
        _ => 1,
    }
}

pub(crate) fn bounded(c: &Connection, before: u64) -> Result<()> {
    if c.total_changes() - before > 128 {
        return Err(crate::Error::StorageUnavailable(
            "Shared storage mutation allowance exceeded; transaction rolled back.".into(),
        ));
    }
    Ok(())
}
