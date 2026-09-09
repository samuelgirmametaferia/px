//! Method executors: each install method runs quietly with output shown on
//! failure, cargo/npm/source builds run inside the px sandbox (build
//! scripts are arbitrary code), and every install verifies the expected
//! executable afterwards.

use crate::app::App;
use crate::error::{PxError, PxResult};

use super::Method;

/// Execute an install method. Returns the expected binary name on success
/// (Some when the method installs a predictable executable).
pub async fn execute(app: &App, method: &Method) -> PxResult<Option<String>> {
    match method {
        Method::Native => Err(PxError::User(
            "native packages are handled before the universal resolver".into(),
        )),
        Method::Release { repo } => {
            let bin = super::releasebin::install(app, repo).await?;
            Ok(Some(bin.unwrap_or_else(|| repo_name(repo))))
        }
        Method::Cargo { crate_name } => {
            let argv = vec!["cargo".to_string(), "install".into(), crate_name.clone()];
            run_sandboxed_quiet(app, &argv).await?;
            // cargo installs the binary named after the crate (or [[bin]]
            // name — crate name is the best guess, verified by the caller)
            Ok(Some(crate_name.clone()))
        }
        Method::Npm { package } => {
            let argv = vec![
                "npm".to_string(),
                "install".into(),
                "-g".into(),
                package.clone(),
            ];
            run_sandboxed_quiet(app, &argv).await?;
            Ok(Some(
                package.rsplit('/').next().unwrap_or(package).to_string(),
            ))
        }
        Method::Pipx { package } => {
            let argv = vec!["pipx".to_string(), "install".into(), package.clone()];
            run_plain_quiet(app, &argv).await?;
            Ok(Some(package.clone()))
        }
        Method::Go { module } => {
            let argv = vec![
                "go".to_string(),
                "install".into(),
                format!("{module}@latest"),
            ];
            run_sandboxed_quiet(app, &argv).await?;
            Ok(Some(
                module.rsplit('/').next().unwrap_or(module).to_string(),
            ))
        }
        Method::Gem { gem } => {
            let argv = vec!["gem".to_string(), "install".into(), gem.clone()];
            run_plain_quiet(app, &argv).await?;
            Ok(Some(gem.clone()))
        }
        Method::Brew { tap } => {
            let argv = vec!["brew".to_string(), "install".into(), tap.clone()];
            run_plain_quiet(app, &argv).await?;
            Ok(Some(tap.rsplit('/').next().unwrap_or(tap).to_string()))
        }
        Method::Script { url } => {
            super::exec_script_sandboxed(app, url).await?;
            Ok(None) // binary unknown — verified by caller when possible
        }
        Method::Source { repo } => {
            source_build(app, repo).await?;
            Ok(Some(repo_name(repo)))
        }
    }
}

fn repo_name(repo: &str) -> String {
    repo.rsplit('/').next().unwrap_or(repo).to_string()
}

/// cargo/go/npm: build scripts are arbitrary code → sandbox when enabled.
async fn run_sandboxed_quiet(app: &App, argv: &[String]) -> PxResult<()> {
    let out = crate::sandbox::run_sandboxed(app, argv, &[]).await?;
    if out.success() {
        return Ok(());
    }
    print_tail(argv, &out);
    Err(PxError::Command {
        cmd: argv.join(" "),
        stderr: "(see output above)".into(),
    })
}

/// pipx/gem/brew: no build scripts, plain quiet run.
async fn run_plain_quiet(app: &App, argv: &[String]) -> PxResult<()> {
    let out = app.exec.run(argv, crate::exec::RunOpts::default()).await?;
    if out.success() {
        return Ok(());
    }
    print_tail(argv, &out);
    Err(PxError::Command {
        cmd: argv.join(" "),
        stderr: "(see output above)".into(),
    })
}

fn print_tail(argv: &[String], out: &crate::exec::ExecOutput) {
    let combined = format!("{}{}", out.stdout, out.stderr);
    if combined.trim().is_empty() {
        return;
    }
    let lines: Vec<&str> = combined.lines().collect();
    let start = lines.len().saturating_sub(15);
    eprintln!("  ── last lines of: {} ──", argv.join(" "));
    for line in &lines[start..] {
        eprintln!("  {line}");
    }
}

/// Verify the expected binary now exists on PATH and executes.
/// Returns its resolved path.
pub async fn verify_binary(bin: &str) -> PxResult<String> {
    let path = which::which(bin)
        .map_err(|_| PxError::User(format!("'{bin}' not found on PATH after install")))?;
    // does it run? (some binaries need args; --version is the convention)
    let out = tokio::process::Command::new(&path)
        .arg("--version")
        .output()
        .await;
    match out {
        Ok(o) if o.status.success() => Ok(path.to_string_lossy().into_owned()),
        Ok(_) => Ok(path.to_string_lossy().into_owned()), // exists+runs; --version unsupported is fine
        Err(e) => Err(PxError::User(format!(
            "'{bin}' exists but cannot execute: {e}"
        ))),
    }
}

/// Source build through the existing github module, sandboxed.
async fn source_build(app: &App, repo: &str) -> PxResult<()> {
    let repo_hit = crate::github::RepoHit {
        full_name: repo.to_string(),
        url: format!("https://github.com/{repo}"),
        clone_url: format!("https://github.com/{repo}.git"),
        stars: 0,
        description: None,
    };
    let dir = crate::github::clone(&repo_hit).await?;
    let strategy = crate::github::detect_strategy(&dir).ok_or_else(|| {
        PxError::User(format!(
            "no recognized build system in {repo} (supported: cargo, go, cmake, make, meson)"
        ))
    })?;
    println!(
        "    {} builds with {}",
        app.style.dim("·"),
        strategy.label()
    );
    crate::github::build_and_install(
        repo_hit_ref(&repo_hit),
        &strategy,
        false,
        crate::sandbox::enabled(app),
    )
    .await
}

fn repo_hit_ref(hit: &crate::github::RepoHit) -> &crate::github::RepoHit {
    hit
}
