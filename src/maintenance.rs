//! Recipe-driven maintenance: uninstall, update notices, orphan detection,
//! installed sizes. Same rules as everything else — px runs the distro's
//! own commands, defined as data.

use std::sync::Arc;

use crate::error::{PxError, PxResult};
use crate::exec::{Executor, RunOpts, expand_argv};
use crate::recipe::schema::{CommandDef, Recipe};

fn expand_no_pkg(def: &CommandDef) -> Vec<String> {
    expand_argv(&def.argv, "", &[], None, None)
}

async fn run(exec: &Arc<dyn Executor>, argv: &[String]) -> PxResult<String> {
    // Maintenance queries get a generous but real ceiling — a wedged
    // package-manager call must fail loudly, not hang px.
    let out = exec
        .run(
            argv,
            RunOpts {
                timeout: Some(std::time::Duration::from_secs(60)),
                ..Default::default()
            },
        )
        .await?;
    Ok(out.stdout)
}

/// Packages with updates available (for the passive notice after installs).
pub async fn updates_available(exec: &Arc<dyn Executor>, recipe: &Recipe) -> PxResult<Vec<String>> {
    let Some(def) = &recipe.maintenance.updates else {
        return Ok(Vec::new());
    };
    let stdout = run(exec, &expand_no_pkg(def)).await?;
    let parse = def.parse.as_deref().unwrap_or("name_version_lines");
    let mut names = crate::backend::parsers::parse_maintenance_names(parse, &stdout);
    names.sort();
    names.dedup();
    Ok(names)
}

/// Orphan packages — installed, unneeded, prime uninstall candidates.
pub async fn orphans(exec: &Arc<dyn Executor>, recipe: &Recipe) -> PxResult<Vec<String>> {
    let Some(def) = &recipe.maintenance.orphans else {
        return Ok(Vec::new());
    };
    let stdout = run(exec, &expand_no_pkg(def)).await?;
    let parse = def.parse.as_deref().unwrap_or("names_lines");
    Ok(crate::backend::parsers::parse_maintenance_names(
        parse, &stdout,
    ))
}

/// Explicitly user-installed package names.
pub async fn explicit_pkgs(exec: &Arc<dyn Executor>, recipe: &Recipe) -> PxResult<Vec<String>> {
    let Some(def) = &recipe.maintenance.explicit else {
        return Ok(Vec::new());
    };
    let stdout = run(exec, &expand_no_pkg(def)).await?;
    let parse = def.parse.as_deref().unwrap_or("names_lines");
    Ok(crate::backend::parsers::parse_maintenance_names(
        parse, &stdout,
    ))
}

/// One installed package: name, on-disk size, install date (if known).
#[derive(Debug, Clone)]
pub struct InstalledPkg {
    pub name: String,
    pub size: u64,
    pub installed: Option<String>,
}

/// Sizes + install dates for many packages in ONE subprocess — the recipes'
/// installed_info commands all accept multiple names ({pkgs...}).
pub async fn installed_info(
    exec: &Arc<dyn Executor>,
    recipe: &Recipe,
    names: &[String],
) -> PxResult<Vec<InstalledPkg>> {
    let Some(def) = &recipe.maintenance.installed_info else {
        return Ok(Vec::new());
    };
    if names.is_empty() {
        return Ok(Vec::new());
    }
    let pkg = names[0].clone();
    let argv = expand_argv(&def.argv, &pkg, names, None, None);
    let stdout = run(exec, &argv).await?;
    let parse = def.parse.as_deref().unwrap_or("pacman_qi");

    match parse {
        // pacman -Qi blocks: Name / Installed Size / Install Date per package
        "pacman_qi" => {
            let mut out = Vec::new();
            let mut current: Vec<(String, String)> = Vec::new();
            for line in stdout.lines().chain(std::iter::once("")) {
                if line.trim().is_empty() {
                    if !current.is_empty() {
                        if let Some(p) = qi_block(&current) {
                            out.push(p);
                        }
                        current.clear();
                    }
                    continue;
                }
                if let Some((k, v)) = line.split_once(':') {
                    let k = k.trim();
                    // pacman -Qi keys can contain spaces ("Installed Size");
                    // the continuation lines of Description are indented and
                    // contain no ':' so they're skipped naturally.
                    if !k.is_empty() && k.len() < 40 && !line.starts_with(' ') {
                        current.push((k.to_string(), v.trim().to_string()));
                    }
                }
            }
            Ok(out)
        }
        // "SIZE\tNAME" lines; dpkg reports KiB, rpm reports bytes.
        "dpkg_size" => Ok(crate::backend::parsers::size_lines(&stdout)
            .into_iter()
            .map(|(name, size)| InstalledPkg {
                name,
                size: size * 1024,
                installed: None,
            })
            .collect()),
        other => {
            tracing::warn!("unknown installed_info parser '{other}'");
            Ok(Vec::new())
        }
    }
}

fn qi_block(block: &[(String, String)]) -> Option<InstalledPkg> {
    let get = |key: &str| {
        block
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(key))
            .map(|(_, v)| v.clone())
    };
    let name = get("Name")?;
    let size = get("Installed Size")
        .and_then(|s| crate::backend::parsers::human_size_bytes(&s))
        .unwrap_or(0);
    Some(InstalledPkg {
        name,
        size,
        installed: get("Install Date"),
    })
}

/// Uninstall packages through the recipe's uninstall command.
pub async fn uninstall(
    exec: &Arc<dyn Executor>,
    recipe: &Recipe,
    pkgs: &[String],
    dry_run: bool,
    verbose: bool,
) -> PxResult<()> {
    let Some(def) = &recipe.maintenance.uninstall else {
        return Err(PxError::User(
            "this recipe defines no uninstall command".into(),
        ));
    };
    if def.elevated && !dry_run {
        crate::backend::elevate::preflight().await?;
    }
    let pkg = pkgs.first().cloned().unwrap_or_default();
    let argv = expand_argv(&def.argv, &pkg, pkgs, None, None);
    // Captured and quiet by default (-v streams it through).
    let out = exec
        .run(
            &argv,
            RunOpts {
                inherit: verbose,
                dry_run,
                ..Default::default()
            },
        )
        .await?;
    if !out.success() {
        let combined = format!("{}{}", out.stdout, out.stderr);
        if !combined.trim().is_empty() {
            let lines: Vec<&str> = combined.lines().collect();
            let start = lines.len().saturating_sub(25);
            eprintln!("  ── last lines of: {} ──", argv.join(" "));
            for line in &lines[start..] {
                eprintln!("  {line}");
            }
        }
        return Err(PxError::Command {
            cmd: argv.join(" "),
            stderr: "(see output above)".into(),
        });
    }
    // The "installed" cache answers are now wrong.
    for p in pkgs {
        crate::cache::delete("provider", &format!("installed:repo:{p}"));
        crate::cache::delete("provider", &format!("installed:aur:{p}"));
    }
    Ok(())
}

/// Human-readable size for display.
pub fn human_size(bytes: u64) -> String {
    let units = ["B", "KiB", "MiB", "GiB"];
    let mut size = bytes as f64;
    let mut unit = 0;
    while size >= 1024.0 && unit < units.len() - 1 {
        size /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} {}", units[unit])
    } else {
        format!("{size:.1} {}", units[unit])
    }
}

/// Files owned by an installed package — used to discover the commands a
/// native install actually provides (metasploit installs msfconsole, not
/// metasploit). Returns paths.
pub async fn package_files(
    exec: &Arc<dyn Executor>,
    recipe: &Recipe,
    pkg: &str,
) -> PxResult<Vec<String>> {
    let Some(def) = &recipe.maintenance.files else {
        return Ok(Vec::new());
    };
    let argv = expand_argv(&def.argv, pkg, &[pkg.to_string()], None, None);
    let out = exec
        .run(
            &argv,
            RunOpts {
                timeout: Some(std::time::Duration::from_secs(15)),
                ..Default::default()
            },
        )
        .await?;
    Ok(out
        .stdout
        .lines()
        .filter_map(|l| {
            l.split_whitespace()
                .nth(1)
                .map(|p| p.to_string())
                .or_else(|| {
                    let t = l.trim();
                    if t.starts_with('/') {
                        Some(t.to_string())
                    } else {
                        None
                    }
                })
        })
        .collect())
}

/// The standard executable directories (bin-dirs) a package's commands
/// can land in.
/// A file is a command if it sits DIRECTLY in a bin dir, or exactly at
/// /opt/<vendor>/bin/<file> — segment arithmetic, airtight where prefix
/// matching over-matched metasploit's 24k vendored files.
fn in_bin_dir(path: &str) -> bool {
    let mut segs: Vec<&str> = path.trim_start_matches('/').split('/').collect();
    let _file = match segs.pop() {
        Some(f) if !f.is_empty() => f,
        _ => return false,
    };
    match segs.as_slice() {
        ["usr", "bin"] | ["usr", "local", "bin"] | ["bin"] | ["usr", "sbin"] => true,
        ["opt", vendor, "bin"] if !vendor.is_empty() => true,
        _ => false,
    }
}

/// Extract command-like paths (executables in bin dirs) from a file list.
pub fn commands_from_files(files: &[String]) -> Vec<String> {
    files
        .iter()
        .filter(|f| in_bin_dir(f))
        .filter(|f| !f.ends_with('/'))
        .cloned()
        .collect()
}

/// Is a path on the user's PATH (by directory)?
pub fn dir_on_path(dir: &str) -> bool {
    match std::env::var("PATH") {
        Ok(p) => p.split(':').any(|d| d == dir),
        Err(_) => false,
    }
}
