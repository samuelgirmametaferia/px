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
    let style = &app.style;

    println!("{}", style.banner());

    // Interrupted install? Offer to resume exactly what's left.
    let mut specs: Vec<String> = specs.to_vec();
    if let Some(journal) = crate::state::read_journal()
        && !journal.remaining.is_empty() && !journal.done.is_empty() {
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
                if crate::apps::registry_lookup(&app, &spec).is_some() {
                    spinner::finish_warn(&pb, format!("{spec} → known app, trying app channel"));
                    if try_app_install(&app, &spec).await? {
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
                // Apps with their own install channels (npm globals,
                // install.sh) come before the GitHub fallback.
                if try_app_install(&app, &spec).await? {
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
                // Last resort: a source build from GitHub. Always a
                // dedicated confirmation, even under --yes.
                try_source_build(&app, &spec).await?;
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
    let width = plan.iter().map(|l| l.len()).max().unwrap_or(30).min(80);
    println!();
    println!("{}", crate::ui::table::panel("install plan", &plan, width));
    println!();

    if !app.cli.yes && !app.cli.dry_run && !crate::ui::prompt::confirm("proceed?", false)? {
        return Err(PxError::Cancelled);
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
    let total_steps: usize = by_source.iter().map(|(_, p)| p.len()).sum();
    let bar =
        crate::ui::progress::install_bar(bar_style, total_steps.max(1), &crate::ui::jokes::next());

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
                        bar.inc(pkgs.len() as u64);
                    }
                    Err(e) => {
                        failures.push(format!("{source}: {e}"));
                    }
                }
            }
            None => failures.push(format!("{source}: no installer configured")),
        }
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
    if failures.is_empty() {
        println!("{}", style.ok("done"));
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

/// App-channel installs (npm globals, install.sh). Returns true when the
/// spec was handled here. npm apps go through the suspicious-package check
/// first — px asks whether to investigate before anything touches npm.
async fn try_app_install(app: &App, spec: &str) -> PxResult<bool> {
    let style = &app.style;

    let install = match crate::apps::registry_lookup(app, spec) {
        Some(i) => Some(i),
        None => crate::apps::npm_lookup(&app.client, spec).await?,
    };
    let Some(install) = install else {
        return Ok(false);
    };

    match &install {
        crate::apps::AppInstall::Npm { package, label } => {
            println!(
                "  {} {} installs via npm: {}",
                style.bold("app"),
                label,
                style.bold(&format!("npm install -g {package}"))
            );

            if app.cli.dry_run {
                println!(
                    "  {} would run: npm install -g {package}",
                    style.dim("[dry-run]")
                );
                return Ok(true);
            }

            // Suspicious-package gate: analyze first (cheap, metadata only),
            // ask to investigate when anything looks off.
            let meta = crate::apps::npm_meta(&app.client, package).await;
            let flags = meta
                .as_ref()
                .map(crate::security::analyze_npm)
                .unwrap_or_default();
            if !flags.is_empty() && crate::ui::interactive() {
                println!(
                    "  {} {}",
                    style.warn("⚠"),
                    flags
                        .iter()
                        .map(|f| f.reason.as_str())
                        .collect::<Vec<_>>()
                        .join("; ")
                );
                if crate::ui::prompt::confirm("want me to investigate before installing?", true)? {
                    let report = crate::security::investigate(app, package).await?;
                    println!("{report}");
                    if !crate::ui::prompt::confirm("install it anyway?", false)? {
                        println!("  {} skipped", style.dim("ok"));
                        return Ok(true);
                    }
                }
            }

            if !app.cli.yes
                && !crate::ui::prompt::confirm(&format!("install {package} via npm?"), false)?
            {
                println!("  {} skipped", style.dim("ok"));
                return Ok(true);
            }
        }
        crate::apps::AppInstall::Script { url, label } => {
            println!(
                "  {} {} installs via installer script: {}",
                style.bold("app"),
                label,
                style.dim(&format!("curl -fsSL {url} | sh"))
            );
            if app.cli.dry_run {
                println!(
                    "  {} would run: curl -fsSL {url} | sh (after confirmation)",
                    style.dim("[dry-run]")
                );
                return Ok(true);
            }
            // Script installs ALWAYS get their dedicated confirmation.
            if !crate::ui::interactive()
                || !crate::ui::prompt::confirm(
                    &format!("download and run the {label} installer from {url}?"),
                    false,
                )?
            {
                println!("  {} skipped", style.dim("ok"));
                return Ok(true);
            }
        }
    }

    let pb = spinner::one(&format!("installing {spec}…"));
    match crate::apps::install(app, &install, app.cli.dry_run).await {
        Ok(()) => {
            spinner::finish_ok(&pb, format!("installed {spec}"));
            if !app.cli.dry_run {
                let mut ledger = Ledger::load();
                ledger.record(spec, "app");
                ledger.save();
            }
            Ok(true)
        }
        Err(e) => {
            spinner::finish_err(&pb, format!("{spec} install failed"));
            Err(e)
        }
    }
}

/// GitHub source-build fallback: search, show what's there, and ask the
/// dedicated question. Never runs without an explicit yes — --yes does NOT
/// cover source builds.
async fn try_source_build(app: &App, spec: &str) -> PxResult<()> {
    let style = &app.style;
    let github_cfg = &app.recipe().github;
    if !github_cfg.enabled || app.cli.no_source || !app.config.source_enabled {
        return Ok(());
    }

    let pb = spinner::one(&format!("searching GitHub for {spec}…"));
    let hits = crate::github::search(&app.client, spec, github_cfg.min_stars).await?;
    if hits.is_empty() {
        spinner::finish_warn(&pb, format!("no repo with ≥{} stars", github_cfg.min_stars));
        return Ok(());
    }
    spinner::finish_ok(&pb, format!("{} candidate(s)", hits.len()));

    // Show the candidates with their stars.
    for hit in hits.iter().take(5) {
        println!(
            "  {} {} {} {}",
            style.src("github"),
            style.bold(&hit.full_name),
            style.dim(&format!("★{}", hit.stars)),
            style.dim(hit.description.as_deref().unwrap_or(""))
        );
    }

    if app.cli.dry_run {
        println!(
            "  {} would offer to clone and build {} (needs confirmation)",
            style.dim("[dry-run]"),
            hits[0].full_name
        );
        return Ok(());
    }

    if !crate::ui::interactive() {
        println!(
            "  {} run px in a terminal to confirm a source build",
            style.dim("(non-interactive)")
        );
        return Ok(());
    }

    let pick = if hits.len() == 1 {
        0
    } else {
        let items: Vec<String> = hits
            .iter()
            .map(|h| {
                format!(
                    "{} ★{} {}",
                    h.full_name,
                    h.stars,
                    h.description.clone().unwrap_or_default()
                )
            })
            .collect();
        crate::ui::prompt::select("which repo should I build?", &items)?
    };
    let repo = &hits[pick];

    // THE dedicated confirmation.
    let confirmed = crate::ui::prompt::confirm(
        &format!(
            "only a source build is available ({}). do you want me to download and build it for you?",
            repo.url
        ),
        false,
    )?;
    if !confirmed {
        println!("  {} skipped source build", style.dim("ok"));
        return Ok(());
    }

    crate::backend::elevate::refuse_root()?;
    let pb = spinner::one(&format!("cloning {}…", repo.full_name));
    let dir = crate::github::clone(repo).await?;
    let strategy = crate::github::detect_strategy(&dir).ok_or_else(|| {
        PxError::User(format!(
            "no recognized build system in {} (supported: cargo, go, cmake, make, meson)",
            repo.full_name
        ))
    })?;
    spinner::finish_ok(&pb, format!("builds with {}", strategy.label()));

    let pb = spinner::one(&format!("building {}…", repo.full_name));
    match crate::github::build_and_install(repo, &strategy, false).await {
        Ok(()) => {
            spinner::finish_ok(&pb, format!("built and installed {}", repo.full_name));
            let mut ledger = Ledger::load();
            ledger.record(&repo.full_name, "github");
            ledger.save();
        }
        Err(e) => {
            spinner::finish_err(&pb, "build failed".into());
            return Err(e);
        }
    }
    Ok(())
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
