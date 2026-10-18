//! Authorized read-only probe. Emits only counts, deadlines and safe status categories.
use gitjuke_core::{Result, runtime::Runtime};
use std::time::{Duration, Instant};
#[tokio::main]
async fn main() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let runtime = Runtime::start(dir.path().join("probe.sqlite3"), |_| {})?;
    let start = Instant::now();
    while runtime.status()?.locked {
        if start.elapsed() > Duration::from_secs(30) {
            println!("Identity validation incomplete");
            runtime.shutdown().await;
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    runtime.refresh_activity("cli/cli", 14719)?;
    loop {
        let page = runtime.activity(0)?;
        let db = rusqlite::Connection::open(dir.path().join("probe.sqlite3"))?;
        let public_done: bool = db.query_row(
            "SELECT EXISTS(SELECT 1 FROM progress WHERE key='activity:pr:cli/cli:14719')",
            [],
            |r| r.get(0),
        )?;
        if public_done && page.next_poll_at > 0 || start.elapsed() > Duration::from_secs(100) {
            let sources: i64 = db.query_row(
                "SELECT COUNT(*) FROM content WHERE key LIKE 'api:repos/cli/cli/%'",
                [],
                |r| r.get(0),
            )?;
            let errors:i64=db.query_row("SELECT COUNT(*) FROM resource_state WHERE key LIKE 'activity:%' AND error IS NOT NULL",[],|r|r.get(0))?;
            println!(
                "public_hydration_complete={public_done} cached_public_pages={sources} baseline={} recovery_incomplete={} pending_prs={} activity_loaded={} source_errors={} poll_deadline_in_s={} elapsed_ms={}",
                page.baseline,
                page.recovery.is_some(),
                page.pending,
                page.items.len(),
                errors,
                page.next_poll_at.saturating_sub(gitjuke_core::now()),
                start.elapsed().as_millis()
            );
            for b in runtime.status()?.budgets {
                println!(
                    "budget={} requests={} bytes={} remaining={}",
                    b.resource, b.requests, b.payload_bytes, b.remaining
                );
            }
            break;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    runtime.shutdown().await;
    Ok(())
}
