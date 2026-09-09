//! `px install <specs...>` — resolve every spec across all sources in
//! parallel, confirm one plan, install.

use std::sync::Arc;

use crate::app::App;
use crate::backend::{Installer, PackageHit, Provider};
use crate::error::{PxError, PxResult};
use crate::ledger::Ledger;
use crate::resolver::{self, Resolution};
use crate::ui::spinner::{self, Spinners};

pub async fn run(app: App, specs: &[String]) -> PxResult<()> {
    crate::backend::elevate::refuse_root()?;
    let started = std::time::Instant::now();
    let style = &app.style;

    println!("{}", style.banner());

    // Interrupted install? Offer to resume exactly what's left.
    let mut specs: Vec<String> = specs.to_vec();
    if let Some(journal) = crate::state::read_journal()
        && !journal.remaining.is_empty()
        && !journal.done.is_empty()
    {
        println!(
            "{} px was interrupted mid-install — left to do: {}",
            style.warn("⚠"),
            style.bold(&journal.remaining.join(", "))
        );
        if crate::ui::interactive()
            && !app.cli.dry_run
            && crate::ui::prompt::confirm("resume that install first?", true)?
        {
            specs = journal.remaining.clone();
        } else if crate::ui::interactive() && !app.cli.dry_run {
            // explicit decline — drop the stale plan
            crate::state::clear_journal();
        }
        // non-interactive / dry-run: keep the journal untouched; it may
        // still be resumable from a terminal later.
    }

    let providers = app.providers();
    let installers = app.installers();
    if providers.is_empty() {
        return Err(PxError::User(
            "no usable package sources on this machine (see `px doctor`)".into(),
        ));
    }

    // Resolve all specs in parallel.
    let assume_exists = app.cli.dry_run && app.cli.recipe.is_some();
    let spinners = Spinners::new(!app.cli.no_color);
    let mut handles = Vec::new();
    for spec in specs {
        let provs: Vec<Arc<dyn Provider>> = providers.clone();
        let spec = spec.clone();
        let pb = spinners.add(&format!("resolving {}", style.bold(&spec)));
        handles.push(tokio::spawn(async move {
            let resolution =
                resolver::resolve_spec(&spec, &provs, resolver::ResolveOpts { assume_exists })
                    .await;
            (spec, resolution, pb)
        }));
    }

    let mut exacts: Vec<PackageHit> = Vec::new();
    let mut problems: Vec<String> = Vec::new();
    for handle in handles {
        let (spec, resolution, pb) = handle.await.map_err(|e| PxError::User(e.to_string()))?;
        match resolution {
            Resolution::Exact(hit) => {
                spinner::finish_ok(
                    &pb,
                    format!("{spec} → {} ({})", style.bold(&hit.name), hit.source),
                );
                exacts.push(hit);
            }
            Resolution::Candidates(candidates) => {
                spinner::finish_warn(&pb, format!("{spec} → {} candidates", candidates.len()));
                // A curated app match ("codex" = the OpenAI CLI) beats fuzzy
                // package candidates (AUR's codex-app-electron-port-bin).
                if crate::universal::registry::lookup(&spec).is_some() {
                    spinner::finish_warn(&pb, format!("{spec} → known app, resolving project"));
                    if crate::universal::try_install(&app, &spec).await? {
                        continue;
                    }
                }
                // Let the user pick.
                if crate::ui::interactive() && !app.cli.yes {
                    let items: Vec<String> = candidates
                        .iter()
                        .take(10)
                        .map(|c| {
                            let d = c
                                .description
                                .as_deref()
                                .unwrap_or("")
                                .chars()
                                .take(48)
                                .collect::<String>();
                            format!("{:<32} {:<6} {}", c.name, c.source, d)
                        })
                        .collect();
                    match crate::ui::prompt::select(&format!("pick a package for '{spec}'"), &items)
                    {
                        Ok(i) => exacts.push(candidates[i].clone()),
                        Err(_) => problems.push(format!("{spec}: skipped")),
                    }
                } else {
                    // Non-interactive: take the best-scoring candidate.
                    exacts.push(candidates[0].clone());
                }
            }
            Resolution::NotFound { near_misses } => {
                spinner::finish_err(&pb, format!("{spec} → not found"));
                // Universal resolver: identify the PROJECT, then offer its
                // install methods with confidence scores. Handles cargo,
                // brew, verified npm, release binaries, installer scripts,
                // and source builds — identity-checked, never name-matched.
                if crate::universal::try_install(&app, &spec).await? {
                    continue;
                }
                if near_misses.is_empty() {
                    problems.push(format!("{spec}: not found in any source"));
                } else {
                    problems.push(format!(
                        "{spec}: not found — did you mean {}?",
                        near_misses.join(", ")
                    ));
                }
            }
        }
    }

    // Skip what's already installed.
    let mut to_install: Vec<PackageHit> = Vec::new();
    let mut already: Vec<String> = Vec::new();
    for hit in exacts {
        let is_installed = match providers.iter().find(|p| p.source_id() == hit.source) {
            Some(p) => p.is_installed(&hit.name).await.unwrap_or(false),
            None => false,
        };
        if is_installed {
            already.push(hit.name.clone());
        } else {
            to_install.push(hit);
        }
    }

    if !already.is_empty() {
        println!(
            "{} {}",
            style.ok("already installed, skipping:"),
            style.dim(&already.join(", "))
        );
    }

    if to_install.is_empty() {
        if problems.is_empty() {
            println!("{}", style.ok("nothing to do"));
        }
        for p in &problems {
            println!("{} {}", style.err("✘"), p);
        }
        return if problems.is_empty() {
            Ok(())
        } else {
            Err(PxError::NotFound(problems.join("; ")))
        };
    }

    // One plan, one confirmation.
    let plan: Vec<String> = to_install
        .iter()
        .map(|h| resolver::plan_line(style, h))
        .collect();
    let width = plan
        .iter()
        .map(|l| crate::ui::table::visible_len(l))
        .max()
        .unwrap_or(30)
        .min(80);
    println!();
    println!("{}", crate::ui::table::panel("install plan", &plan, width));
    println!();

    if !app.cli.yes && !app.cli.dry_run && !crate::ui::prompt::confirm("proceed?", false)? {
        return Err(PxError::Cancelled);
    }

    // Ask for sudo NOW, before any spinner or bar starts drawing — the
    // password prompt must never race live output for the terminal. Only
    // when the planned commands actually need elevation (AUR helpers and
    // plain downloads don't).
    let needs_elevation = app.recipe().sources.iter().any(|s| {
        to_install.iter().any(|h| h.source == s.id)
            && s.install.as_ref().is_some_and(|c| c.elevated)
    });
    if !app.cli.dry_run && needs_elevation {
        crate::backend::elevate::preflight().await?;
    }

    // One joke per install. Non-negotiable.
    println!(
        "{} {}",
        style.dim("while you wait:"),
        style.dim(&crate::ui::jokes::next())
    );
    println!();

    // Install, grouped by source (one command per source).
    let mut by_source: Vec<(String, Vec<String>)> = Vec::new();
    for hit in &to_install {
        match by_source.iter_mut().find(|(s, _)| *s == hit.source) {
            Some((_, pkgs)) => pkgs.push(hit.name.clone()),
            None => by_source.push((hit.source.clone(), vec![hit.name.clone()])),
        }
    }

    // Journal the plan so an interrupted px can resume the remainder.
    let mut journal = crate::state::Journal {
        created: chrono::Utc::now().to_rfc3339(),
        recipe: app.recipe().meta.id.clone(),
        remaining: to_install.iter().map(|h| h.name.clone()).collect(),
        done: Vec::new(),
    };
    if !app.cli.dry_run {
        crate::state::write_journal(&journal);
    }

    // A joke for the road, shown under the chosen progress bar.
    let bar_style = crate::ui::progress::BarStyle::parse(&app.cli.bar)
        .unwrap_or(crate::ui::progress::BarStyle::Shades);

    // Download sizes for a REAL percentage: package metadata knows the
    // bytes; the network-flow monitor (netmon) watches /proc/net/dev while
    // the package manager runs. No output parsing, no guessing. AUR builds
    // report no size — there the monitor still runs, open-ended, so the
    // byte counter visibly climbs during the download.
    let mut info_handles = Vec::new();
    for hit in &to_install {
        let provs: Vec<Arc<dyn Provider>> = providers.clone();
        let name = hit.name.clone();
        info_handles.push(tokio::spawn(async move {
            for p in &provs {
                if let Ok(Some(h)) = p.info(&name).await {
                    return h.download_size;
                }
            }
            None
        }));
    }
    let mut total_bytes: u64 = 0;
    for h in info_handles {
        if let Ok(Some(size)) = h.await {
            total_bytes += size;
        }
    }

    let bar = if total_bytes > 0 {
        crate::ui::progress::bytes_bar(
            bar_style,
            total_bytes,
            &format!(
                "downloading ~{}",
                crate::maintenance::human_size(total_bytes)
            ),
        )
    } else {
        crate::ui::progress::flow_bar(bar_style, "installing (size unknown — watching the wire)")
    };

    // Watch the interface counters for the whole install; aborted after.
    // `credited` is shared with the monitor so the summary can report the
    // real downloaded total.
    let credited = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let monitor = crate::netmon::total_rx_bytes().map(|baseline| {
        crate::netmon::spawn_monitor(
            bar.clone(),
            if total_bytes > 0 {
                Some(total_bytes)
            } else {
                None
            },
            baseline,
            std::sync::Arc::clone(&credited),
        )
    });

    let mut ledger = Ledger::load();
    let mut failures = Vec::new();
    for (source, pkgs) in &by_source {
        bar.set_message(format!("installing {} via {source}", pkgs.join(", ")));
        // Installers are ordered like providers; find by source id.
        let idx = providers.iter().position(|p| p.source_id() == *source);
        let installer: Option<&Arc<dyn Installer>> = idx.and_then(|i| installers.get(i));
        match installer {
            Some(installer) => {
                let ctx = crate::backend::InstallCtx {
                    assume_yes: app.cli.yes,
                    dry_run: app.cli.dry_run,
                    verbose: app.cli.verbose > 0,
                };
                match installer.install(pkgs, ctx).await {
                    Ok(()) => {
                        if !app.cli.dry_run {
                            for p in pkgs {
                                ledger.record(p, source);
                                // the "not installed" cache answer is now wrong
                                crate::cache::delete(
                                    "provider",
                                    &format!("installed:{source}:{p}"),
                                );
                                crate::state::journal_step(&mut journal, p);
                            }
                        }
                    }
                    Err(e) => {
                        failures.push(format!("{source}: {e}"));
                    }
                }
            }
            None => failures.push(format!("{source}: no installer configured")),
        }
    }
    if let Some(monitor) = monitor {
        monitor.abort();
    }
    bar.finish_and_clear();
    if !app.cli.dry_run {
        ledger.save();
        // Only clear the journal when everything installed — a failed or
        // interrupted run leaves its remainder for the resume offer.
        if failures.is_empty() {
            crate::state::clear_journal();
        }
    }

    // Passive update notice: px mentions what's outdated while it has you.
    if !app.cli.dry_run {
        notice_updates(&app).await;
    }

    for f in &failures {
        println!("{} {}", style.err("✘"), f);
    }

    // Summary stats: what happened, through which sources, how long.
    let installed_count: usize = to_install.len() - failures.len();
    let per_source = by_source
        .iter()
        .map(|(s, p)| format!("{s} {}", p.len()))
        .collect::<Vec<_>>()
        .join(" · ");
    let downloaded = credited.load(std::sync::atomic::Ordering::Relaxed);
    let downloaded_note = if downloaded > 0 {
        format!(
            " · downloaded {}",
            crate::maintenance::human_size(downloaded)
        )
    } else {
        String::new()
    };
    println!(
        "\n{} {} installed{} · {} already present · {} failed{} · {:.1}s",
        if failures.is_empty() {
            style.ok("✔")
        } else {
            style.warn("⚠")
        },
        installed_count,
        if per_source.is_empty() {
            String::new()
        } else {
            format!(" ({per_source})")
        },
        already.len(),
        failures.len(),
        downloaded_note,
        started.elapsed().as_secs_f64()
    );
    if failures.is_empty() {
        Ok(())
    } else {
        Err(PxError::User(failures.join("; ")))
    }
}

/// Passive update notice after installs — informational, never blocking.
async fn notice_updates(app: &App) {
    let style = &app.style;
    let updates = crate::maintenance::updates_available(&app.exec, app.recipe())
        .await
        .unwrap_or_default();
    if updates.is_empty() {
        return;
    }
    let shown = updates
        .iter()
        .take(8)
        .cloned()
        .collect::<Vec<_>>()
        .join(", ");
    let more = if updates.len() > 8 {
        format!(" +{} more", updates.len() - 8)
    } else {
        String::new()
    };
    println!(
        "  {} {} package(s) have updates available: {}{more} — update with your package manager when ready",
        style.warn("ℹ"),
        updates.len(),
        style.dim(&shown)
    );
}

/// `px -i` with nothing else: ask what to install.
pub async fn interactive(app: App) -> PxResult<()> {
    let style = &app.style;
    println!("{}", style.banner());
    if !crate::ui::interactive() {
        return Err(PxError::User(
            "interactive mode needs a terminal — try `px install <pkg>`".into(),
        ));
    }
    let answer = crate::ui::prompt::input("package name, or project path (for 'install for')")?;
    if answer.trim().is_empty() {
        return Ok(());
    }
    let path = std::path::Path::new(&answer);
    if path.exists() && (path.is_dir() || crate::forfile::looks_like_project_file(path)) {
        super::install_for::run(app, &answer).await
    } else {
        let specs = answer
            .split_whitespace()
            .map(|s| s.to_string())
            .collect::<Vec<_>>();
        run(app, &specs).await
    }
}
