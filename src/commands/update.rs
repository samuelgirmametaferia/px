//! `px update` — self-update from GitHub releases + refresh recipes.
//!
//! Resolves the latest v-tag (releases/latest is the registry's rolling
//! release), downloads, verifies it runs, replaces the running binary
//! (Linux allows replacing a running executable's file), and force-fetches
//! fresh recipes so fundamental-command fixes ship without a reinstall.

use crate::app::App;
use crate::error::{PxError, PxResult};

const REPO: &str = "samuelgirmametaferia/px";

pub async fn run(app: App) -> PxResult<()> {
    let style = &app.style;
    println!("{}", style.banner());

    // current version
    let current = env!("CARGO_PKG_VERSION");

    // resolve the latest v-tag from the tags atom feed — no API rate
    // limits, no auth (releases/latest points at the registry's rolling
    // release, and the JSON API rate-limits unauthenticated clients)
    let feed = app
        .client
        .get(format!("https://github.com/{REPO}/tags.atom"))
        .send()
        .await?
        .text()
        .await?;
    // entry ids: tag:github.com,2008:Repository/…/v3.1.0 — the tag is
    // the path segment after the repo id
    let latest_version = feed
        .lines()
        .filter_map(|l| {
            let t = l.trim();
            let t = t.strip_prefix("<id>")?;
            let tag = t.strip_suffix("</id>")?;
            tag.rsplit('/').next()
        })
        .find(|t| t.starts_with('v') && t[1..].chars().next().is_some_and(|c| c.is_ascii_digit()))
        .unwrap_or_default()
        .trim_start_matches('v')
        .to_string();
    if latest_version.is_empty() {
        return Err(PxError::User(
            "could not resolve the latest px version".into(),
        ));
    }
    let download_url = format!("https://github.com/{REPO}/releases/download/v{latest_version}/px");

    println!(
        "  {} current: {}   latest: {}",
        style.dim("·"),
        style.bold(current),
        style.bold(&latest_version)
    );

    if version_lte(&latest_version, current) {
        println!("  {} px is up to date", style.ok("✔"));
    } else {
        println!("  {} updating…", style.dim("↓"));
        let bytes = app
            .client
            .get(&download_url)
            .send()
            .await?
            .error_for_status()?
            .bytes()
            .await?;

        // find the running binary
        let exe = std::env::current_exe()
            .map_err(|e| PxError::User(format!("cannot locate the px binary: {e}")))?;
        let tmp = exe.with_extension("new");
        std::fs::write(&tmp, &bytes)
            .map_err(|e| PxError::User(format!("cannot write update: {e}")))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(&tmp)
                .map_err(|e| PxError::User(format!("stat: {e}")))?
                .permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(&tmp, perms)
                .map_err(|e| PxError::User(format!("chmod: {e}")))?;
        }
        // verify the new binary actually runs before replacing
        let out = tokio::process::Command::new(&tmp)
            .arg("--version")
            .output()
            .await;
        match out {
            Ok(o) if o.status.success() => {}
            _ => {
                let _ = std::fs::remove_file(&tmp);
                return Err(PxError::User(
                    "the downloaded binary failed verification — not updating".into(),
                ));
            }
        }
        // replace (Linux allows replacing a running binary's file)
        std::fs::rename(&tmp, &exe)
            .map_err(|e| PxError::User(format!("cannot replace {}: {e}", exe.display())))?;
        println!(
            "  {} updated to {} — new version applies on next run",
            style.ok("✔"),
            style.bold(&latest_version)
        );
    }

    // refresh recipes from the web (fundamental-command fixes ship in recipes)
    let source = crate::registry::source_from_config(&app.config);
    if let Some(_reg_source) = source {
        println!("  {} refreshing recipes…", style.dim("·"));
    }
    // recipes re-fetch: clear cache + reload is handled by --refresh; do it directly
    let recipe_id = app.recipe().meta.id.clone();
    let _ =
        std::fs::remove_file(crate::recipe::load::cache_dir().join(format!("{recipe_id}.toml")));
    let _ = app
        .client
        .get(format!(
            "https://raw.githubusercontent.com/{}/{}/recipes/{recipe_id}.toml",
            app.config.recipe_repo, app.config.recipe_branch
        ))
        .send()
        .await;
    println!("  {} recipes will be fresh on next run", style.ok("✔"));

    Ok(())
}

/// Semver-aware a <= b. String equality fails on 3.10.0 vs 3.9.0 ("3.10"
/// sorts before "3.9" lexically), which would hide real updates.
fn version_lte(a: &str, b: &str) -> bool {
    let parse = |v: &str| -> Vec<u64> {
        v.split('.')
            .map(|p| {
                p.chars()
                    .take_while(|c| c.is_ascii_digit())
                    .collect::<String>()
                    .parse::<u64>()
                    .unwrap_or(0)
            })
            .collect()
    };
    let (av, bv) = (parse(a), parse(b));
    let n = av.len().max(bv.len());
    let av: Vec<u64> = (0..n).map(|i| av.get(i).copied().unwrap_or(0)).collect();
    let bv: Vec<u64> = (0..n).map(|i| bv.get(i).copied().unwrap_or(0)).collect();
    av <= bv
}

/// `px update <pkg>...` — per-package update: re-run install resolution
/// for each (already-installed packages get refreshed to latest).
pub async fn run_pkgs(app: App, pkgs: &[String]) -> PxResult<()> {
    let style = &app.style;
    let providers = app.providers();
    let mut outdated: Vec<String> = Vec::new();
    for pkg in pkgs {
        let mut installed = false;
        for p in &providers {
            if p.is_installed(pkg).await.unwrap_or(false) {
                installed = true;
                break;
            }
        }
        let installed = installed;
        if !installed {
            println!(
                "  {} {} is not installed — use `px install {pkg}`",
                style.warn("○"),
                style.bold(pkg)
            );
            continue;
        }
        outdated.push(pkg.clone());
    }
    if outdated.is_empty() {
        return Ok(());
    }
    // reuse the install machinery: it resolves to the latest version and
    // skips if the resolved version is already present
    super::install::run(app, &outdated).await
}

#[cfg(test)]
mod tests {
    use super::version_lte;

    #[test]
    fn semver_ordering() {
        assert!(version_lte("3.1.0", "3.1.0"));
        assert!(version_lte("3.1.0", "3.10.0")); // the lexical trap
        assert!(!version_lte("3.10.0", "3.9.0"));
        assert!(version_lte("3.9.0", "3.10.0"));
        assert!(version_lte("2.99.0", "3.0.0"));
        assert!(!version_lte("3.1.0", "3.0.9"));
    }
}
