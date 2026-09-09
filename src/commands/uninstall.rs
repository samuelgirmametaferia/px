//! `px uninstall <pkgs...>` — removes through the installer that put it
//! there. System packages go through the distro's own tools; universal
//! installs (cargo/npm/pipx/go/gem/brew/release/script) route to their
//! own uninstallers, using the install database.

use crate::app::App;
use crate::error::{PxError, PxResult};
use crate::ledger::Ledger;

pub async fn run(app: App, specs: &[String]) -> PxResult<()> {
    crate::backend::elevate::refuse_root()?;
    let style = &app.style;
    println!("{}", style.banner());

    let providers = app.providers();

    for spec in specs {
        // 1. universal install in the database? route by its method.
        if let Some(entry) = Ledger::load().find(spec) {
            let method = entry.source.as_str();
            if !matches!(method, "repo" | "aur" | "pacman" | "apt" | "dnf" | "zypper") {
                remove_universal(&app, spec, method).await?;
                continue;
            }
            // fall through: it's a system package, remove below under its
            // recorded package name
        }

        let name = Ledger::load()
            .find(spec)
            .map(|e| e.name.clone())
            .unwrap_or_else(|| spec.clone());

        // is it actually installed as a system package?
        let mut installed = false;
        for p in &providers {
            if p.is_installed(&name).await.unwrap_or(false) {
                installed = true;
                break;
            }
        }
        if !installed {
            println!(
                "  {} {} is not installed here",
                style.warn("⚠"),
                style.bold(spec)
            );
            continue;
        }

        let to_remove = vec![name];
        let info = crate::maintenance::installed_info(&app.exec, app.recipe(), &to_remove)
            .await
            .unwrap_or_default();
        let total: u64 = info.iter().map(|p| p.size).sum();
        if total > 0 {
            println!(
                "  {} {} — free ~{}",
                style.dim("·"),
                style.bold(&to_remove[0]),
                style.bold(&crate::maintenance::human_size(total))
            );
        }
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
                crate::ui::spinner::finish_ok(&pb, format!("removed {}", to_remove.join(", ")));
                let mut ledger = Ledger::load();
                ledger.entries.retain(|e| e.name != to_remove[0]);
                ledger.save();
            }
            Err(e) => {
                crate::ui::spinner::finish_err(&pb, "removal failed".into());
                return Err(e);
            }
        }
    }
    Ok(())
}

/// Route a removal through the method that installed it.
async fn remove_universal(app: &App, spec: &str, method: &str) -> PxResult<()> {
    let style = &app.style;
    let ledger = Ledger::load();
    let Some(entry) = ledger.find(spec) else {
        return Ok(());
    };
    let name = entry.name.clone();

    let argv: Vec<String> = match method {
        "cargo" => vec!["cargo".into(), "uninstall".into(), name.clone()],
        "npm" => vec!["npm".into(), "uninstall".into(), "-g".into(), name.clone()],
        "pipx" => vec!["pipx".into(), "uninstall".into(), name.clone()],
        "gem" => vec!["gem".into(), "uninstall".into(), name.clone()],
        "brew" => vec!["brew".into(), "uninstall".into(), name.clone()],
        // release/script/source installs: px placed a binary — remove it
        "release" | "script" | "source" | "github" => {
            let bin = entry
                .binary_path
                .clone()
                .or_else(|| {
                    entry
                        .binary
                        .as_ref()
                        .and_then(|b| which::which(b).ok())
                        .map(|p| p.to_string_lossy().into_owned())
                })
                .ok_or_else(|| {
                    PxError::User(format!(
                        "no recorded binary for {spec} — remove it manually"
                    ))
                })?;
            println!(
                "  {} removing {} ({})",
                style.dim("·"),
                style.bold(&bin),
                style.dim(method)
            );
            if !app.cli.dry_run {
                std::fs::remove_file(&bin)
                    .map_err(|e| PxError::User(format!("cannot remove {bin}: {e}")))?;
            }
            println!("  {} removed {}", style.ok("✔"), style.bold(spec));
            let mut ledger = Ledger::load();
            ledger.entries.retain(|e| e.name != name);
            ledger.save();
            return Ok(());
        }
        other => {
            return Err(PxError::User(format!(
                "don't know how to remove a '{other}' install"
            )));
        }
    };

    println!(
        "  {} {} → {}",
        style.dim("·"),
        style.bold(spec),
        style.dim(&argv.join(" "))
    );
    let pb = crate::ui::spinner::one(&format!("removing via {method}…"));
    let out = app.exec.run(&argv, crate::exec::RunOpts::default()).await?;
    if out.success() {
        crate::ui::spinner::finish_ok(&pb, format!("removed {spec}"));
        let mut ledger = Ledger::load();
        ledger.entries.retain(|e| e.name != name);
        ledger.save();
        Ok(())
    } else {
        crate::ui::spinner::finish_err(&pb, format!("{method} uninstall failed"));
        let combined = format!("{}{}", out.stdout, out.stderr);
        if !combined.trim().is_empty() {
            let lines: Vec<&str> = combined.lines().collect();
            let start = lines.len().saturating_sub(10);
            for line in &lines[start..] {
                eprintln!("  {line}");
            }
        }
        Err(PxError::Command {
            cmd: argv.join(" "),
            stderr: "(see output above)".into(),
        })
    }
}
