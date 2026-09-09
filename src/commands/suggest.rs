//! `px suggest` — find unused/unneeded packages and offer to remove them
//! to free space. Orphans first (safe, nothing depends on them), then the
//! biggest explicitly-installed packages with how long they've sat there.

use crate::app::App;
use crate::error::{PxError, PxResult};
use crate::ui::spinner;

pub async fn run(app: App) -> PxResult<()> {
    crate::backend::elevate::refuse_root()?;
    let style = &app.style;
    println!("{}", style.banner());

    let exec = &app.exec;
    let recipe = app.recipe();

    let orphans = crate::maintenance::orphans(exec, recipe).await?;
    let mut candidates: Vec<crate::maintenance::InstalledPkg> = Vec::new();

    if !orphans.is_empty() {
        let pb = crate::ui::spinner::one(&format!("sizing {} orphans…", orphans.len()));
        let info = crate::maintenance::installed_info(exec, recipe, &orphans)
            .await
            .unwrap_or_default();
        spinner::finish_ok(&pb, format!("{} orphans", orphans.len()));
        candidates.extend(info);
    } else {
        println!(
            "  {} no orphans — nothing installed that nothing needs",
            style.ok("✔")
        );
    }

    // Biggest explicit packages: only the top few, sized in one call.
    let explicit = crate::maintenance::explicit_pkgs(exec, recipe).await?;
    let big: Vec<crate::maintenance::InstalledPkg> = if !explicit.is_empty() {
        // cap the query: 250 names max keeps pacman -Qi fast
        let query: Vec<String> = explicit.iter().take(250).cloned().collect();
        let pb = crate::ui::spinner::one("sizing your biggest packages…");
        let info = crate::maintenance::installed_info(exec, recipe, &query)
            .await
            .unwrap_or_default();
        spinner::finish_ok(&pb, format!("{} packages", info.len()));
        let mut sorted = info;
        sorted.sort_by_key(|p| std::cmp::Reverse(p.size));
        sorted.into_iter().take(12).collect()
    } else {
        Vec::new()
    };

    // ---- report -------------------------------------------------------------
    let mut suggestions: Vec<crate::maintenance::InstalledPkg> = Vec::new();

    if !candidates.is_empty() {
        let total: u64 = candidates.iter().map(|p| p.size).sum();
        println!();
        println!(
            "  {} orphans — nothing depends on these, removing is safe",
            style.header("safe to remove")
        );
        let mut sorted = candidates.clone();
        sorted.sort_by_key(|p| std::cmp::Reverse(p.size));
        for p in sorted.iter().take(15) {
            println!(
                "    {} {}",
                style.bold(&p.name),
                style.dim(&crate::maintenance::human_size(p.size))
            );
        }
        println!(
            "    {} {}",
            style.dim("≈"),
            style.bold(&format!("free {}", crate::maintenance::human_size(total)))
        );
        suggestions.extend(sorted);
    }

    if !big.is_empty() {
        println!();
        println!(
            "  {} biggest packages you explicitly installed — take a look:",
            style.header("space hogs")
        );
        for p in &big {
            let age = p
                .installed
                .as_deref()
                .filter(|d| *d != "None")
                .map(|d| format!(", installed {}", d))
                .unwrap_or_default();
            println!(
                "    {} {}{}",
                style.bold(&p.name),
                style.dim(&crate::maintenance::human_size(p.size)),
                style.dim(&age)
            );
        }
    }

    if suggestions.is_empty() {
        println!();
        println!(
            "  {} nothing obvious to clean — your system is tidy",
            style.ok("✔")
        );
        return Ok(());
    }

    // ---- pick + remove -----------------------------------------------------
    if app.cli.dry_run || !crate::ui::interactive() {
        println!();
        println!(
            "  {} run {} in a terminal to pick and remove",
            style.dim("tip"),
            style.bold("px suggest")
        );
        return Ok(());
    }

    println!();
    let items: Vec<String> = suggestions
        .iter()
        .take(20)
        .map(|p| format!("{:<32} {}", p.name, crate::maintenance::human_size(p.size)))
        .collect();
    // The multiselect owns the terminal — suspend live drawing around it.
    crate::ui::spinner::suspend_all();
    let picks = dialoguer::MultiSelect::with_theme(&dialoguer::theme::ColorfulTheme::default())
        .with_prompt("select packages to uninstall (space = toggle)")
        .items(&items)
        .interact_on(&dialoguer::console::Term::stderr())
        .map_err(|_| PxError::Cancelled);
    crate::ui::spinner::resume_all();
    let picks = picks?;
    if picks.is_empty() {
        println!("  {} nothing selected", style.dim("ok"));
        return Ok(());
    }
    let chosen: Vec<String> = picks
        .into_iter()
        .map(|i| suggestions[i].name.clone())
        .collect();

    let freed: u64 = chosen
        .iter()
        .filter_map(|n| suggestions.iter().find(|p| &p.name == n).map(|p| p.size))
        .sum();
    if !app.cli.yes {
        println!();
        println!(
            "  {} about to remove {} — free ~{}",
            style.warn("⚠"),
            style.bold(&chosen.join(", ")),
            style.bold(&crate::maintenance::human_size(freed))
        );
        if !crate::ui::prompt::confirm("proceed?", false)? {
            return Err(PxError::Cancelled);
        }
    }

    // sudo before the spinner starts — a password prompt behind a live
    // spinner is invisible and looks exactly like a hang.
    if !app.cli.dry_run {
        crate::backend::elevate::preflight().await?;
    }

    let pb = crate::ui::spinner::one(&format!("removing {}…", chosen.join(", ")));
    match crate::maintenance::uninstall(
        &app.exec,
        app.recipe(),
        &chosen,
        app.cli.dry_run,
        app.cli.verbose > 0,
    )
    .await
    {
        Ok(()) => {
            spinner::finish_ok(
                &pb,
                format!("freed ~{}", crate::maintenance::human_size(freed)),
            );
            let mut ledger = crate::ledger::Ledger::load();
            ledger.entries.retain(|e| !chosen.contains(&e.name));
            ledger.save();
            Ok(())
        }
        Err(e) => {
            crate::ui::spinner::finish_err(&pb, "removal failed".into());
            Err(e)
        }
    }
}
