//! Last-resort source builds from GitHub. This is px's own fallback (not a
//! package manager!): when no source on the machine has a package, find the
//! project on GitHub and build it with its own build system — but only ever
//! after an explicit, dedicated confirmation.

use serde::Deserialize;
use std::path::{Path, PathBuf};

use crate::error::{PxError, PxResult};

#[derive(Debug, Clone, Deserialize)]
struct GhRepo {
    full_name: String,
    html_url: String,
    clone_url: String,
    stargazers_count: u64,
    #[serde(default)]
    description: Option<String>,
}

#[derive(Debug, Clone)]
pub struct RepoHit {
    pub full_name: String,
    pub url: String,
    pub clone_url: String,
    pub stars: u64,
    pub description: Option<String>,
}

/// How a repo builds. Detected from its files after cloning.
#[derive(Debug, Clone, PartialEq)]
pub enum Strategy {
    /// cargo install --path . (or --git without cloning ourselves)
    Cargo,
    /// go install ./...
    Go,
    /// cmake -B build && make -C build && sudo make -C build install
    CMake,
    /// make && sudo make install
    Make,
    /// meson setup build && ninja -C build && sudo ninja -C build install
    Meson,
}

impl Strategy {
    pub fn label(&self) -> &'static str {
        match self {
            Strategy::Cargo => "cargo",
            Strategy::Go => "go",
            Strategy::CMake => "cmake",
            Strategy::Make => "make",
            Strategy::Meson => "meson",
        }
    }
}

const GITHUB_API: &str = "https://api.github.com";

pub async fn search(
    client: &reqwest::Client,
    term: &str,
    min_stars: u64,
) -> PxResult<Vec<RepoHit>> {
    let url = format!("{GITHUB_API}/search/repositories");
    let resp = client
        .get(url)
        .query(&[("q", term), ("sort", "stars"), ("per_page", "5")])
        .header("Accept", "application/vnd.github+json")
        .send()
        .await?;
    if !resp.status().is_success() {
        // Rate limits are expected unauthenticated; degrade silently.
        tracing::debug!("github search failed: {}", resp.status());
        return Ok(Vec::new());
    }
    #[derive(Deserialize)]
    struct SearchResponse {
        #[serde(default)]
        items: Vec<GhRepo>,
    }
    let body: SearchResponse = resp.json().await?;
    Ok(body
        .items
        .into_iter()
        .filter(|r| r.stargazers_count >= min_stars)
        .map(|r| RepoHit {
            full_name: r.full_name,
            url: r.html_url,
            clone_url: r.clone_url,
            stars: r.stargazers_count,
            description: r.description,
        })
        .collect())
}

pub fn builds_dir() -> PathBuf {
    crate::cache::cache_root().join("builds")
}

fn repo_dir(name: &str) -> PathBuf {
    builds_dir().join(name.replace('/', "__"))
}

/// Clone (or update) a repo into the px build cache.
pub async fn clone(repo: &RepoHit) -> PxResult<PathBuf> {
    let dir = repo_dir(&repo.full_name);
    if dir.join(".git").exists() {
        let out = tokio::process::Command::new("git")
            .args(["pull", "--ff-only"])
            .current_dir(&dir)
            .output()
            .await
            .map_err(|e| PxError::User(format!("git pull failed: {e}")))?;
        if !out.status.success() {
            // A stale clone is not fatal — rebuild from what's there.
            tracing::warn!("git pull in {} failed, using existing clone", dir.display());
        }
        return Ok(dir);
    }
    std::fs::create_dir_all(builds_dir())
        .map_err(|e| PxError::User(format!("cannot create builds dir: {e}")))?;
    let out = tokio::process::Command::new("git")
        .args(["clone", "--depth", "1", &repo.clone_url])
        .arg(&dir)
        .output()
        .await
        .map_err(|e| PxError::User(format!("git clone failed: {e}")))?;
    if !out.status.success() {
        return Err(PxError::Command {
            cmd: format!("git clone {}", repo.clone_url),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        });
    }
    Ok(dir)
}

/// Detect how a cloned repo builds, from its files.
pub fn detect_strategy(dir: &Path) -> Option<Strategy> {
    let has = |name: &str| dir.join(name).exists();
    if has("Cargo.toml") {
        Some(Strategy::Cargo)
    } else if has("go.mod") {
        Some(Strategy::Go)
    } else if has("CMakeLists.txt") {
        Some(Strategy::CMake)
    } else if has("meson.build") {
        Some(Strategy::Meson)
    } else if has("Makefile") || has("makefile") || has("GNUmakefile") {
        Some(Strategy::Make)
    } else {
        None
    }
}

async fn run_in(dir: &Path, argv: &[&str], dry_run: bool) -> PxResult<()> {
    if dry_run {
        let prefix = if std::io::IsTerminal::is_terminal(&std::io::stdout()) {
            "\x1b[1;33m[dry-run]\x1b[0m"
        } else {
            "[dry-run]"
        };
        println!("{prefix} (cwd {}) {}", dir.display(), argv.join(" "));
        return Ok(());
    }
    crate::ui::prompt::flush();
    let status = tokio::process::Command::new(argv[0])
        .args(&argv[1..])
        .current_dir(dir)
        .status()
        .await
        .map_err(|e| PxError::Command {
            cmd: argv.join(" "),
            stderr: e.to_string(),
        })?;
    if status.success() {
        Ok(())
    } else {
        Err(PxError::Command {
            cmd: argv.join(" "),
            stderr: "build step failed (output above)".into(),
        })
    }
}

/// Build + install a cloned repo with its own build system. The dedicated
/// confirmation happened before this is called — never inside.
pub async fn build_and_install(repo: &RepoHit, strategy: &Strategy, dry_run: bool) -> PxResult<()> {
    let dir = clone(repo).await?;
    if !dry_run {
        let detected = detect_strategy(&dir);
        let strategy = match &detected {
            Some(s) if s == strategy => strategy,
            Some(s) => {
                // Cloned HEAD builds differently than the search suggested.
                println!(
                    "  note: repo builds with {} (expected {})",
                    s.label(),
                    strategy.label()
                );
                s
            }
            None => {
                return Err(PxError::User(format!(
                    "no recognized build system in {} (supported: cargo, go, cmake, make, meson)",
                    dir.display()
                )));
            }
        };
        build_with(&dir, strategy, dry_run).await
    } else {
        build_with(&dir, strategy, dry_run).await
    }
}

async fn build_with(dir: &Path, strategy: &Strategy, dry_run: bool) -> PxResult<()> {
    match strategy {
        Strategy::Cargo => {
            run_in(dir, &["cargo", "build", "--release"], dry_run).await?;
            run_in(
                dir,
                &["cargo", "install", "--path", ".", "--force"],
                dry_run,
            )
            .await
        }
        Strategy::Go => {
            run_in(dir, &["go", "build", "./..."], dry_run).await?;
            run_in(dir, &["go", "install", "./..."], dry_run).await
        }
        Strategy::CMake => {
            run_in(dir, &["cmake", "-B", "build"], dry_run).await?;
            run_in(dir, &["make", "-C", "build"], dry_run).await?;
            run_in(dir, &["sudo", "make", "-C", "build", "install"], dry_run).await
        }
        Strategy::Make => {
            run_in(dir, &["make"], dry_run).await?;
            run_in(dir, &["sudo", "make", "install"], dry_run).await
        }
        Strategy::Meson => {
            run_in(dir, &["meson", "setup", "build"], dry_run).await?;
            run_in(dir, &["ninja", "-C", "build"], dry_run).await?;
            run_in(dir, &["sudo", "ninja", "-C", "build", "install"], dry_run).await
        }
    }
}
