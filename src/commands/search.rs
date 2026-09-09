//! `px search <term>` — every active source in parallel, one pretty table.

use crate::app::App;
use crate::error::PxResult;
use crate::ui::spinner::Spinners;
use std::sync::Arc;

pub async fn run(app: App, term: &str) -> PxResult<()> {
    let style = &app.style;
    let started = std::time::Instant::now();
    println!("{}", style.banner());

    let providers = app.providers();
    if providers.is_empty() {
        return Err(crate::error::PxError::User(
            "no usable package sources on this machine (see `px doctor`)".into(),
        ));
    }

    let spinners = Spinners::new(!app.cli.no_color);
    let mut handles = Vec::new();
    for p in &providers {
        let p: Arc<dyn crate::backend::Provider> = Arc::clone(p);
        let term = term.to_string();
        let pb = spinners.add(&format!("searching {}", style.bold(p.label())));
        handles.push(tokio::spawn(async move {
            let hits = p.search(&term).await.unwrap_or_default();
            (p, hits, pb)
        }));
    }

    let mut all = Vec::new();
    let mut per_source: Vec<(String, usize)> = Vec::new();
    for h in handles {
        let (p, hits, pb) = h.await.expect("search task panicked");
        let label = p.label().to_string();
        if hits.is_empty() {
            crate::ui::spinner::finish_warn(&pb, format!("{label}: no matches"));
        } else {
            crate::ui::spinner::finish_ok(&pb, format!("{label}: {} matches", hits.len()));
        }
        per_source.push((label, hits.len()));
        all.extend(hits);
    }

    // Rank by fuzzy score against the term; drop duplicates (same name from
    // multiple sources keeps its highest-priority source, which came first).
    for hit in &mut all {
        hit.score = crate::resolver::fuzzy::score(term, &hit.name);
    }
    all.sort_by_key(|h| std::cmp::Reverse(h.score));
    all.dedup_by(|a, b| a.name == b.name);
    // everything is shown — the table wraps to the terminal, not to a cap

    if all.is_empty() {
        // Nothing matched the full term — build a did-you-mean pool with
        // shorter-prefix searches ("firefxo" → "firef" → firefox).
        let mut pool: Vec<String> = Vec::new();
        for len in [6usize, 5, 4, 3] {
            if term.len() <= len {
                continue;
            }
            let prefix = &term[..len];
            let mut prefix_handles = Vec::new();
            for p in &providers {
                let p = std::sync::Arc::clone(p);
                let prefix = prefix.to_string();
                prefix_handles.push(tokio::spawn(async move {
                    p.search(&prefix).await.unwrap_or_default()
                }));
            }
            for h in prefix_handles {
                if let Ok(hits) = h.await {
                    pool.extend(hits.into_iter().map(|hit| hit.name));
                }
            }
            if !pool.is_empty() {
                break;
            }
        }
        pool.sort();
        pool.dedup();
        let near = crate::resolver::fuzzy::near_misses(term, &pool, 5);
        if near.is_empty() {
            println!("{} nothing matching '{term}' anywhere", style.err("✘"));
        } else {
            println!(
                "{} nothing matching '{term}' — did you mean: {}?",
                style.warn("⚠"),
                near.iter()
                    .map(|n| style.bold(n))
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }
        return Err(crate::error::PxError::NotFound(term.to_string()));
    }

    println!();
    println!("{}", crate::ui::table::hits_table(style, &all));

    // Stats footer: totals, per-source breakdown, wall time.
    let breakdown = per_source
        .iter()
        .filter(|(_, n)| *n > 0)
        .map(|(label, n)| format!("{label} {n}"))
        .collect::<Vec<_>>()
        .join(" · ");
    println!(
        "\n{} {} result(s) · {} · {:.1}s",
        style.dim("·"),
        all.len(),
        if breakdown.is_empty() {
            "no source matched".to_string()
        } else {
            breakdown
        },
        started.elapsed().as_secs_f64()
    );
    Ok(())
}
