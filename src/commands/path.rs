//! `px path <pkg>` — where a package lives: installed?, which binaries it
//! provides and where, package-manager file list summary.

use crate::app::App;
use crate::error::PxResult;

pub async fn run(app: App, name: &str) -> PxResult<()> {
    let style = &app.style;
    println!("{}", style.banner());

    // the ledger knows universal installs (query/binary/project handles)
    let ledger = crate::ledger::Ledger::load();
    if let Some(entry) = ledger.find(name) {
        println!(
            "  {} {} (installed by px via {})",
            style.ok("◆"),
            style.bold(&entry.name),
            entry.source
        );
        if let Some(p) = &entry.binary_path {
            println!("    binary path: {}", style.bold(p));
            if let Ok(meta) = std::fs::metadata(p) {
                println!(
                    "    size:        {}",
                    crate::maintenance::human_size(meta.len())
                );
            }
        }
        if let Some(b) = &entry.binary {
            if let Ok(found) = which::which(b) {
                println!(
                    "    on PATH:     {} → {}",
                    style.bold(b),
                    style.dim(&found.to_string_lossy())
                );
            } else {
                println!(
                    "    {}:       {} is not on PATH",
                    style.warn("⚠"),
                    style.bold(b)
                );
            }
        }
        if let Some(project) = &entry.project {
            println!("    project:     {}", style.dim(project));
        }
        println!(
            "    installed:   {}",
            style.dim(&entry.installed_at.to_rfc3339())
        );
        return Ok(());
    }

    // native package: the recipe's installed check + file list
    let providers = app.providers();
    let mut installed = false;
    for p in &providers {
        if p.is_installed(name).await.unwrap_or(false) {
            installed = true;
            println!(
                "  {} {} is installed (via {})",
                style.ok("◆"),
                style.bold(name),
                style.dim(p.label())
            );
            break;
        }
    }
    if !installed {
        println!(
            "  {} {} is not installed",
            style.warn("○"),
            style.bold(name)
        );
        println!("    {} try `px install {name}`", style.dim("·"));
        return Ok(());
    }

    // files + binaries
    let files = crate::maintenance::package_files(&app.exec, app.recipe(), name)
        .await
        .unwrap_or_default();
    let cmds = crate::maintenance::commands_from_files(&files);
    if cmds.is_empty() {
        println!("    no standard-bin files found");
        return Ok(());
    }
    let mut names: Vec<String> = cmds
        .iter()
        .map(|c| c.rsplit('/').next().unwrap_or(c).to_string())
        .collect();
    names.sort();
    names.dedup();
    println!(
        "    commands:    {}",
        style.bold(&names.iter().take(8).cloned().collect::<Vec<_>>().join(", "))
    );
    if names.len() > 8 {
        println!(
            "                {} +{} more",
            style.dim("·"),
            names.len() - 8
        );
    }
    let mut dirs: Vec<String> = cmds
        .iter()
        .filter_map(|c| c.rsplit_once('/').map(|(d, _)| d.to_string()))
        .collect();
    dirs.sort();
    dirs.dedup();
    for d in &dirs {
        let on_path = crate::maintenance::dir_on_path(d);
        println!(
            "    {}  {} {}",
            if on_path {
                style.ok("on PATH ")
            } else {
                style.warn("off PATH")
            },
            style.bold(d),
            if on_path {
                String::new()
            } else {
                style.dim("(add it to use the commands)")
            }
        );
    }
    Ok(())
}
