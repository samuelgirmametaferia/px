//! --local mode: bootstrap a language-level environment in the project.
//! Nothing here needs sudo; everything stays inside the project directory.

use std::path::Path;

use crate::error::{PxError, PxResult};
use crate::forfile::detector::Ecosystem;

/// Install local deps with the ecosystem's own tooling.
pub async fn bootstrap(
    root: &Path,
    ecosystem: Ecosystem,
    deps: &[String],
    dry_run: bool,
) -> PxResult<()> {
    let deps = deps.to_vec();
    if deps.is_empty() {
        return Ok(());
    }
    match ecosystem {
        Ecosystem::Python => python_venv(root, &deps, dry_run).await,
        Ecosystem::Node => npm_install(root, dry_run).await,
        Ecosystem::Go => go_mod(root, dry_run).await,
        Ecosystem::Ruby => bundle(root, dry_run).await,
        _ => {
            println!(
                "  ({} deps are resolved by that ecosystem's own tooling)",
                ecosystem.id()
            );
            Ok(())
        }
    }
}

async fn python_venv(root: &Path, deps: &[String], dry_run: bool) -> PxResult<()> {
    let venv = root.join(".venv");
    if !venv.exists() {
        println!("  creating {} …", venv.display());
        run(&["python3", "-m", "venv", ".venv"], root, dry_run).await?;
    }
    let pip = venv.join("bin").join("pip");
    let pip_str = pip.to_string_lossy().into_owned();
    let mut argv = vec![pip_str.as_str(), "install"];
    let dep_refs: Vec<&str> = deps.iter().map(|s| s.as_str()).collect();
    argv.extend(dep_refs);
    run(&argv, root, dry_run).await
}

async fn npm_install(root: &Path, dry_run: bool) -> PxResult<()> {
    // npm install without a package.json is an ENOENT error — loose .js
    // files have nothing for npm to resolve; only bootstrap a real project
    if !root.join("package.json").exists() {
        println!("  · no package.json — nothing for npm to install");
        return Ok(());
    }
    run(&["npm", "install"], root, dry_run).await
}

async fn go_mod(root: &Path, dry_run: bool) -> PxResult<()> {
    run(&["go", "mod", "download"], root, dry_run).await
}

async fn bundle(root: &Path, dry_run: bool) -> PxResult<()> {
    run(&["bundle", "install"], root, dry_run).await
}

async fn run(argv: &[&str], cwd: &Path, dry_run: bool) -> PxResult<()> {
    // a missing tool is a confusing ENOENT spawn error — check first and
    // tell the user which tool to install (with the px command to get it)
    if let Some(bin) = argv.first()
        && which::which(bin).is_err()
    {
        return Err(crate::error::PxError::User(format!(
            "'{bin}' is not installed — get it with: px install {bin}",
        )));
    }
    let argv: Vec<String> = argv.iter().map(|s| s.to_string()).collect();
    if dry_run {
        println!("[dry-run] (cwd {}) {}", cwd.display(), argv.join(" "));
        return Ok(());
    }
    let out = tokio::process::Command::new(&argv[0])
        .args(&argv[1..])
        .current_dir(cwd)
        .output()
        .await
        .map_err(|e| PxError::Command {
            cmd: argv.join(" "),
            stderr: e.to_string(),
        })?;
    if !out.status.success() {
        return Err(PxError::Command {
            cmd: argv.join(" "),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        });
    }
    Ok(())
}
