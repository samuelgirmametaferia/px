//! `px install for <path>` — analyze, plan, confirm, install.

use std::collections::BTreeMap;

use crate::app::App;
use crate::error::{PxError, PxResult};
use crate::forfile::{self, Analysis, detector::Ecosystem};
use crate::ledger::Ledger;
use crate::ui::spinner::{self};

pub async fn run(app: App, path: &str) -> PxResult<()> {
    crate::backend::elevate::refuse_root()?;
    let style = &app.style;
    let path = std::path::PathBuf::from(path.trim());
    println!("{}", style.banner());

    if !path.exists() {
        return Err(PxError::User(format!("path not found: {}", path.display())));
    }

    // ---- analyze ----------------------------------------------------------
    let pb = spinner::one(&format!(
        "analyzing {}…",
        style.bold(&path.display().to_string())
    ));
    let mut analysis = match forfile::analyze(&path, app.recipe()) {
        Ok(a) => a,
        Err(e) => {
            spinner::finish_err(&pb, "analysis failed".into());
            return Err(e);
        }
    };
    let eco_names: Vec<String> = analysis
        .ecosystems
        .iter()
        .map(|e| e.id().to_string())
        .collect();
    spinner::finish_ok(
        &pb,
        format!(
            "{} project ({})",
            eco_names.join(" + "),
            analysis.evidence.join(", ")
        ),
    );

    // ---- validate + filter -------------------------------------------------
    let providers = app.providers();
    let installers = app.installers();
    let pb = spinner::one("checking what's already installed…");
    forfile::filter_installed_and_validate(&mut analysis, &providers, app.cli.dry_run).await?;
    spinner::finish_ok(&pb, "done".into());

    if analysis.system.is_empty()
        && analysis.local.is_empty()
        && analysis.tools.is_empty()
        && analysis.unmapped.is_empty()
    {
        println!(
            "{}",
            style.ok("everything this project needs is already here")
        );
        return Ok(());
    }

    // ---- plan ---------------------------------------------------------------
    println!();
    let mut plan_lines: Vec<String> = Vec::new();
    let mut plan_items: Vec<(String, String)> = Vec::new(); // (pkg, source) — filled per mode

    if !analysis.tools.is_empty() {
        for tool in &analysis.tools {
            plan_lines.push(format!("{}  {}", style.warn("tool"), style.bold(tool)));
            plan_items.push((tool.clone(), "repo".into()));
        }
    }
    for dep in &analysis.system {
        let target = &dep.candidates[0];
        let extra = if dep.candidates.len() > 1 {
            style.dim(&format!(" (or {})", dep.candidates[1..].join(", ")))
        } else {
            String::new()
        };
        plan_lines.push(format!(
            "{} {} → {}{}",
            style.bold(dep.ecosystem.id()),
            style.dim(&dep.import),
            style.bold(target),
            extra
        ));
        plan_items.push((target.clone(), "repo".into()));
    }

    let width = plan_lines
        .iter()
        .map(|l| l.len())
        .max()
        .unwrap_or(30)
        .min(90);
    if !plan_lines.is_empty() {
        println!(
            "{}",
            crate::ui::table::panel("system packages", &plan_lines, width)
        );
    }

    // Grouped local deps summary.
    let mut local_by_eco: BTreeMap<Ecosystem, Vec<String>> = BTreeMap::new();
    for dep in &analysis.local {
        local_by_eco
            .entry(dep.ecosystem)
            .or_default()
            .push(dep.name.clone());
    }
    for (eco, deps) in &local_by_eco {
        println!(
            "  {} {} deps (handled by {}'s own tooling): {}",
            style.bold(eco.id()),
            deps.len(),
            eco.id(),
            style.dim(&deps.iter().take(8).cloned().collect::<Vec<_>>().join(", "))
        );
    }

    if !analysis.unmapped.is_empty() {
        println!();
        println!(
            "  {} couldn't map (may be vendored or private): {}",
            style.warn("⚠"),
            style.dim(&analysis.unmapped.join(", "))
        );
    }
    println!();

    if analysis.system.is_empty() && analysis.tools.is_empty() {
        // Only local deps.
        if let Some(mode) = pick_mode(&app, &analysis)
            && (mode == Mode::Local || mode == Mode::Both)
        {
            bootstrap_locals(&analysis, app.cli.dry_run).await?;
        }
        return Ok(());
    }

    // ---- mode -----------------------------------------------------------------
    let mode = pick_mode(&app, &analysis).unwrap_or(Mode::Global);
    let do_global = matches!(mode, Mode::Global | Mode::Both);
    let do_local = matches!(mode, Mode::Local | Mode::Both);

    // ---- confirm -----------------------------------------------------------------
    if !app.cli.yes
        && !app.cli.dry_run
        && !crate::ui::prompt::confirm("install these system packages?", false)?
    {
        return Err(PxError::Cancelled);
    }

    // ---- install --------------------------------------------------------------------
    if do_global {
        // Group by source via resolver; simplest: install through first
        // provider that has each package (providers are priority-ordered).
        let mut ledger = Ledger::load();
        let pb = spinner::one("installing…");
        let mut installed: Vec<String> = Vec::new();
        let mut failures: Vec<String> = Vec::new();

        // Dedupe plan items.
        let mut seen = std::collections::BTreeSet::new();
        let items: Vec<(String, String)> = plan_items
            .into_iter()
            .filter(|(pkg, _)| seen.insert(pkg.clone()))
            .collect();

        for (pkg, _) in &items {
            // Under dry-run (distro simulation) skip validation — the target
            // source's tools may not exist on this machine by design.
            if app.cli.dry_run && !installers.is_empty() {
                let ctx = crate::backend::InstallCtx {
                    assume_yes: true,
                    dry_run: true,
                };
                match installers[0].install(std::slice::from_ref(pkg), ctx).await {
                    Ok(()) => installed.push(pkg.clone()),
                    Err(e) => failures.push(format!("{pkg}: {e}")),
                }
                continue;
            }
            let mut done = false;
            for (i, p) in providers.iter().enumerate() {
                if p.info(pkg).await.ok().flatten().is_none() {
                    continue;
                }
                let ctx = crate::backend::InstallCtx {
                    assume_yes: app.cli.yes,
                    dry_run: app.cli.dry_run,
                };
                match installers[i].install(std::slice::from_ref(pkg), ctx).await {
                    Ok(()) => {
                        installed.push(pkg.clone());
                        ledger.record(pkg, p.source_id());
                        done = true;
                        break;
                    }
                    Err(e) => {
                        failures.push(format!("{pkg}: {e}"));
                        done = true;
                        break;
                    }
                }
            }
            if !done {
                failures.push(format!("{pkg}: not found in any source"));
            }
        }
        if !app.cli.dry_run {
            ledger.save();
        }
        if installed.is_empty() && failures.is_empty() {
            spinner::finish_ok(&pb, "nothing to do".into());
        } else if failures.is_empty() {
            spinner::finish_ok(&pb, format!("installed {}", installed.join(", ")));
        } else {
            spinner::finish_err(&pb, "some installs failed".into());
            for f in &failures {
                println!("  {} {}", style.err("✘"), f);
            }
        }
    }

    if do_local {
        bootstrap_locals(&analysis, app.cli.dry_run).await?;
    }

    println!("{}", style.ok("done"));
    Ok(())
}

#[derive(PartialEq)]
enum Mode {
    Global,
    Local,
    Both,
}

fn pick_mode(app: &App, _analysis: &Analysis) -> Option<Mode> {
    if app.cli.local {
        Some(Mode::Local)
    } else if app.cli.global {
        Some(Mode::Global)
    } else {
        match app.config.default_mode.as_str() {
            "global" => Some(Mode::Global),
            "local" => Some(Mode::Local),
            _ => {
                if crate::ui::interactive() && !app.cli.yes {
                    let items = vec![
                        "system packages (distro-wide, needs sudo)".to_string(),
                        "local environment (project-only, no sudo)".to_string(),
                        "both".to_string(),
                    ];
                    match crate::ui::prompt::select("how should I install these?", &items) {
                        Ok(0) => Some(Mode::Global),
                        Ok(1) => Some(Mode::Local),
                        Ok(2) => Some(Mode::Both),
                        _ => None,
                    }
                } else {
                    Some(Mode::Global)
                }
            }
        }
    }
}

async fn bootstrap_locals(analysis: &Analysis, dry_run: bool) -> PxResult<()> {
    use std::collections::BTreeMap;
    let mut by_eco: BTreeMap<Ecosystem, Vec<String>> = BTreeMap::new();
    for dep in &analysis.local {
        by_eco
            .entry(dep.ecosystem)
            .or_default()
            .push(dep.name.clone());
    }
    for (eco, deps) in by_eco {
        println!("  setting up {} environment…", eco.id());
        forfile::localenv::bootstrap(&analysis.root, eco, &deps, dry_run).await?;
    }
    Ok(())
}
