//! `px uninstall <pkgs...>` — through the distro's own tools, with a
//! confirmed plan and ledger cleanup.

use crate::app::App;
use crate::error::{PxError, PxResult};
use crate::ledger::Ledger;
use crate::ui::spinner;

pub async fn run(app: App, specs: &[String]) -> PxResult<()> {
    crate::backend::elevate::refuse_root()?;
    let style = &app.style;
    println!("{}", style.banner());

    // What's actually installed?
    let providers = app.providers();
    let mut to_remove: Vec<String> = Vec::new();
    let mut missing: Vec<String> = Vec::new();
    for spec in specs {
        let mut installed = false;
        for p in &providers {
            if p.is_installed(spec).await.unwrap_or(false) {
                installed = true;
                break;
            }
        }
        if installed {
            to_remove.push(spec.clone());
        } else {
            missing.push(spec.clone());
        }
    }

    if !missing.is_empty() {
        println!(
            "{} not installed here: {}",
            style.warn("⚠"),
            style.dim(&missing.join(", "))
        );
    }
    if to_remove.is_empty() {
        println!("{}", style.ok("nothing to remove"));
        return Ok(());
    }

    // Sizes, when the recipe can produce them.
    let info = crate::maintenance::installed_info(&app.exec, app.recipe(), &to_remove)
        .await
        .unwrap_or_default();
    let total: u64 = info.iter().map(|p| p.size).sum();

    let plan: Vec<String> = to_remove
        .iter()
        .map(|name| {
            let size = info
                .iter()
                .find(|p| &p.name == name)
                .map(|p| crate::maintenance::human_size(p.size))
                .unwrap_or_else(|| "?".into());
            format!("{}  {}", style.bold(name), style.dim(&size))
        })
        .collect();
    let width = plan
        .iter()
        .map(|l| crate::ui::table::visible_len(l))
        .max()
        .unwrap_or(30)
        .min(80);
    println!();
    println!("{}", crate::ui::table::panel("remove plan", &plan, width));
    if total > 0 {
        println!(
            "  {} freeing about {}",
            style.dim("≈"),
            style.bold(&crate::maintenance::human_size(total))
        );
    }
    println!();

    if !app.cli.yes
        && !app.cli.dry_run
        && !crate::ui::prompt::confirm("remove these packages?", false)?
    {
        return Err(PxError::Cancelled);
    }

    // sudo before the spinner starts, so the prompt is never hidden.
    if !app.cli.dry_run && app.recipe().maintenance.uninstall.is_some() {
        crate::backend::elevate::preflight().await?;
    }

    let pb = crate::ui::spinner::one(&format!("removing {}…", to_remove.join(", ")));
    match crate::maintenance::uninstall(
        &app.exec,
        app.recipe(),
        &to_remove,
        app.cli.dry_run,
        app.cli.verbose > 0,
    )
    .await
    {
        Ok(()) => {
            spinner::finish_ok(&pb, format!("removed {}", to_remove.join(", ")));
            if !app.cli.dry_run {
                let mut ledger = Ledger::load();
                ledger.entries.retain(|e| !to_remove.contains(&e.name));
                ledger.save();
            }
            Ok(())
        }
        Err(e) => {
            crate::ui::spinner::finish_err(&pb, "removal failed".into());
            Err(e)
        }
    }
}
