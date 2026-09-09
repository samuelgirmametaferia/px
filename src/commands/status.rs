//! `px status` — the running px instance (lock), any interrupted install
//! that can be resumed (journal), and cache health.

use crate::app::App;
use crate::error::PxResult;

pub async fn run(app: App) -> PxResult<()> {
    let style = &app.style;
    println!("{}", style.banner());
    println!();

    println!("{}", style.header("instances"));
    match crate::state::read_lock() {
        Some(lock) => {
            let mine = lock.pid == std::process::id();
            println!(
                "  {} pid {} — {}{}",
                if mine {
                    style.ok("● this px")
                } else {
                    style.warn("● running")
                },
                style.bold(&lock.pid.to_string()),
                style.dim(&lock.command),
                if mine {
                    String::new()
                } else {
                    style.dim(" (a second px instance)")
                }
            );
            println!("    {}", style.dim(&format!("started {}", lock.started)));
        }
        None => println!("  {} no px instance running", style.ok("○")),
    }
    println!();

    println!("{}", style.header("interrupted work"));
    match crate::state::read_journal() {
        Some(j) if !j.remaining.is_empty() => {
            println!(
                "  {} an install was interrupted with {} left to install:",
                style.warn("⚠"),
                j.remaining.len()
            );
            println!(
                "    {} {}",
                style.dim("pending:"),
                style.bold(&j.remaining.join(", "))
            );
            if !j.done.is_empty() {
                println!(
                    "    {} {}",
                    style.dim("done:"),
                    style.dim(&j.done.join(", "))
                );
            }
            println!(
                "    {} px will offer to resume these next time you install",
                style.dim("tip:")
            );
        }
        _ => println!("  {} nothing pending", style.ok("○")),
    }
    println!();

    println!("{}", style.header("caches"));
    let root = crate::cache::cache_root();
    for ns in ["recipes", "search", "provider", "fun", "builds"] {
        let dir = root.join(ns);
        let count = std::fs::read_dir(&dir).map(|d| d.count()).unwrap_or(0);
        if count > 0 {
            println!("  {:<10} {}", ns, style.value(&format!("{count} entries")));
        }
    }
    let total: u64 = ["recipes", "search", "provider", "fun", "builds"]
        .iter()
        .map(|ns| dir_size(&root.join(ns)))
        .sum();
    println!(
        "  {:<10} {}",
        "on disk",
        style.dim(&crate::maintenance::human_size(total))
    );
    Ok(())
}

fn dir_size(dir: &std::path::Path) -> u64 {
    std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .flatten()
                .filter_map(|e| e.metadata().ok())
                .map(|m| m.len())
                .sum()
        })
        .unwrap_or(0)
}
