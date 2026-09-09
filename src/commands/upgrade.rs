//! `px upgrade` — full system upgrade through the distro's own tools,
//! recipe-driven like everything else.

use crate::app::App;
use crate::error::{PxError, PxResult};

pub async fn run(app: App) -> PxResult<()> {
    crate::backend::elevate::refuse_root()?;
    let style = &app.style;
    println!("{}", style.banner());

    let Some(def) = app.recipe().maintenance.upgrade.clone() else {
        return Err(PxError::User(
            "this recipe defines no upgrade command".into(),
        ));
    };

    // What's pending, if the recipe can tell us.
    let pending = crate::maintenance::updates_available(&app.exec, app.recipe())
        .await
        .unwrap_or_default();
    if pending.is_empty() {
        println!("  {} everything is already up to date", style.ok("✔"));
        return Ok(());
    }
    println!(
        "  {} {} package(s) have updates available",
        style.warn("ℹ"),
        pending.len()
    );
    let shown = pending
        .iter()
        .take(8)
        .cloned()
        .collect::<Vec<_>>()
        .join(", ");
    let more = if pending.len() > 8 {
        format!(" +{} more", pending.len() - 8)
    } else {
        String::new()
    };
    println!("  {} {shown}{more}", style.dim("→"));

    if !app.cli.yes && !app.cli.dry_run && !crate::ui::prompt::confirm("upgrade now?", false)? {
        return Err(PxError::Cancelled);
    }

    // sudo before the spinner — the prompt must never hide behind drawing.
    if def.elevated && !app.cli.dry_run {
        crate::backend::elevate::preflight().await?;
    }

    // argv[1] is the package manager itself (sudo pacman -Syu ...)
    let pm_name = def.argv.get(1).cloned().unwrap_or_else(|| "system".into());
    let pb = crate::ui::spinner::one(&format!("upgrading via {pm_name}…"));
    let pkg = String::new();
    let argv = crate::exec::expand_argv(&def.argv, &pkg, &[], None, None);
    let out = app
        .exec
        .run(
            &argv,
            crate::exec::RunOpts {
                inherit: app.cli.verbose > 0,
                dry_run: app.cli.dry_run,
                ..Default::default()
            },
        )
        .await;
    match out {
        Ok(o) if o.success() => {
            crate::ui::spinner::finish_ok(&pb, format!("upgraded {} packages", pending.len()));
            Ok(())
        }
        Ok(o) => {
            crate::ui::spinner::finish_err(&pb, "upgrade failed".into());
            let combined = format!("{}{}", o.stdout, o.stderr);
            if !combined.trim().is_empty() {
                let lines: Vec<&str> = combined.lines().collect();
                let start = lines.len().saturating_sub(25);
                eprintln!("  ── last lines of: {} ──", argv.join(" "));
                for line in &lines[start..] {
                    eprintln!("  {line}");
                }
            }
            Err(PxError::Command {
                cmd: argv.join(" "),
                stderr: "(see output above)".into(),
            })
        }
        Err(e) => {
            crate::ui::spinner::finish_err(&pb, "upgrade failed".into());
            Err(e)
        }
    }
}
