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
    if app.cli.dry_run {
        println!("  would check for a newer px release and refresh the active recipe");
        return Ok(());
    }

    // resolve the latest v-tag from the tags atom feed — no API rate
    // limits, no auth (releases/latest points at the registry's rolling
    // release, and the JSON API rate-limits unauthenticated clients)
    let feed = app
        .client
        .get(format!("https://github.com/{REPO}/tags.atom"))
        .send()
        .await?
        .error_for_status()?
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
    let asset = release_asset(std::env::consts::OS, std::env::consts::ARCH)?;
    let release_url = format!("https://github.com/{REPO}/releases/download/v{latest_version}");

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
        let response = app
            .client
            .get(format!("{release_url}/{asset}"))
            .send()
            .await?;
        let response = if response.status() == reqwest::StatusCode::NOT_FOUND
            && std::env::consts::ARCH == "x86_64"
        {
            app.client.get(format!("{release_url}/px")).send().await?
        } else {
            response
        };
        let bytes = response.error_for_status()?.bytes().await?;

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
        let mut verification = tokio::process::Command::new(&tmp);
        verification.arg("--version").kill_on_drop(true);
        let out =
            tokio::time::timeout(std::time::Duration::from_secs(10), verification.output()).await;
        match out {
            Ok(Ok(o))
                if o.status.success()
                    && String::from_utf8_lossy(&o.stdout).trim()
                        == format!("px {latest_version}") => {}
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

    if app.cli.recipe.is_none() {
        println!("  {} refreshing recipes…", style.dim("·"));
        let recipe_id = &app.recipe().meta.id;
        let text =
            crate::recipe::load::fetch_remote(&app.client, &app.config.recipe_repo(), recipe_id)
                .await?;
        let recipe = crate::recipe::schema::Recipe::parse_str(&text).map_err(PxError::Recipe)?;
        if recipe.meta.id != *recipe_id {
            return Err(PxError::Recipe(
                "downloaded recipe has an unexpected id".into(),
            ));
        }
        crate::recipe::load::write_cache(recipe_id, &text)?;
        println!("  {} recipe refreshed", style.ok("✔"));
    }

    Ok(())
}

fn release_asset(os: &str, arch: &str) -> PxResult<&'static str> {
    match (os, arch) {
        ("linux", "x86_64") => Ok("px-linux-x86_64"),
        ("linux", "aarch64") => Ok("px-linux-aarch64"),
        _ => Err(PxError::User(format!(
            "no prebuilt px update for {os}/{arch}; update from source"
        ))),
    }
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

    #[tokio::test]
    async fn dry_run_does_not_contact_the_update_server() {
        use clap::Parser;
        use std::sync::Arc;
        // A listening proxy records any attempted HTTP request without
        // contacting GitHub. Dry-run must return before it is used.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let client = reqwest::Client::builder()
            .proxy(
                reqwest::Proxy::all(format!("http://{}", listener.local_addr().unwrap())).unwrap(),
            )
            .timeout(std::time::Duration::from_millis(100))
            .build()
            .unwrap();
        let app = crate::app::App {
            cli: crate::cli::Cli::parse_from(["px", "--dry-run", "update"]),
            config: crate::config::Config::default(),
            style: crate::ui::style::Style::new(false),
            recipe: crate::recipe::load::LoadedRecipe {
                recipe: Arc::new(
                    crate::recipe::schema::Recipe::parse_str(crate::recipe::load::bundled::ARCH)
                        .unwrap(),
                ),
                source: crate::recipe::load::RecipeSource::Bundled,
            },
            client,
            exec: Arc::new(crate::exec::RealExecutor::new()),
        };
        super::run(app).await.unwrap();
        assert_eq!(
            listener.accept().unwrap_err().kind(),
            std::io::ErrorKind::WouldBlock
        );
    }

    #[test]
    fn updates_select_native_assets() {
        assert_eq!(
            super::release_asset("linux", "x86_64").unwrap(),
            "px-linux-x86_64"
        );
        assert_eq!(
            super::release_asset("linux", "aarch64").unwrap(),
            "px-linux-aarch64"
        );
        assert!(super::release_asset("macos", "aarch64").is_err());
        assert!(super::release_asset("linux", "riscv64").is_err());
    }

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
