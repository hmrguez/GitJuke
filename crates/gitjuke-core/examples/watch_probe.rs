//! Actual stored-auth runtime probe; output excludes repository names, titles, bodies and team slugs.
use gitjuke_core::{Result, runtime::Runtime};
use std::time::{Duration, Instant};
#[tokio::main]
async fn main() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let runtime = Runtime::start(dir.path().join("probe.sqlite3"), |_| {})?;
    let started = Instant::now();
    loop {
        let s = runtime.status()?;
        if !s.locked
            && s.discovery.len() >= 3
            && s.discovery.iter().all(|r| r.complete && r.error.is_none())
        {
            break;
        }
        if started.elapsed() > Duration::from_secs(45) {
            println!("Discovery incomplete: {}", s.health);
            for r in s.discovery {
                println!("resource complete={} error={:?}", r.complete, r.error);
            }
            runtime.shutdown().await;
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let s = runtime.status()?;
    println!(
        "Discovery complete: sources={}, active watches={}, elapsed_ms={}",
        s.discovery.len(),
        s.watch_count,
        started.elapsed().as_millis()
    );
    let args: Vec<_> = std::env::args().collect();
    if args.len() == 3 {
        let repo = args[1].clone();
        let number = args[2].parse().expect("PR number");
        runtime.select(repo.clone(), number)?;
        let start = Instant::now();
        loop {
            let checks = runtime.checks(&repo, number, 0)?;
            if checks.head.is_some()
                && !checks.commits.is_empty()
                && checks
                    .commits
                    .iter()
                    .all(|r| r.complete && r.error.is_none())
            {
                println!(
                    "Current check snapshot: contexts_loaded={}, more={}, commits={}, actions_attempts={}, elapsed_ms={}",
                    checks.items.len(),
                    checks.next_offset.is_some(),
                    checks.commits.len(),
                    checks.items.iter().filter(|c| c.attempt.is_some()).count(),
                    start.elapsed().as_millis()
                );
                break;
            }
            if start.elapsed() > Duration::from_secs(35) {
                println!("Check snapshot incomplete: {}", runtime.status()?.health);
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        let before = runtime
            .checks(&repo, number, 0)?
            .commits
            .iter()
            .filter_map(|c| c.fetched_at)
            .max()
            .unwrap_or(0);
        let refresh_start = Instant::now();
        loop {
            let latest = runtime
                .checks(&repo, number, 0)?
                .commits
                .iter()
                .filter_map(|c| c.fetched_at)
                .max()
                .unwrap_or(0);
            if latest > before {
                println!(
                    "Next automatic selected-head/check refresh observed after_ms={}",
                    refresh_start.elapsed().as_millis()
                );
                break;
            }
            if refresh_start.elapsed() > Duration::from_secs(30) {
                println!(
                    "Next automatic refresh not observed within probe window; verify quota/deadlines."
                );
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        runtime.reader_closed()?;
        println!(
            "After close: active watches={}",
            runtime.status()?.watch_count
        );
    }
    for b in runtime.status()?.budgets {
        println!(
            "Budget {}: requests={}, payload_bytes={}, cost={}, remaining={}",
            b.resource, b.requests, b.payload_bytes, b.cost, b.remaining
        );
    }
    runtime.shutdown().await;
    println!("Runtime shut down; original gh identity/credentials and GitHub content unchanged.");
    Ok(())
}
