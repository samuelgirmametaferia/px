//! Release-binary installs: instead of piping someone's install.sh into a
//! shell, px reproduces what a good installer does — pick the right asset
//! from the latest GitHub release, verify its checksum when one is
//! published, unpack, and place the executable in ~/.local/bin.

use serde::Deserialize;
use std::path::PathBuf;

use crate::app::App;
use crate::error::{PxError, PxResult};

#[derive(Debug, Deserialize)]
pub struct GhAsset {
    pub name: String,
    pub browser_download_url: String,
    #[serde(default)]
    #[allow(dead_code)]
    pub size: u64,
}

#[derive(Debug, Deserialize)]
pub struct GhRelease {
    pub tag_name: String,
    #[serde(default)]
    pub assets: Vec<GhAsset>,
}

/// The asset we'd install for this machine, if any.
#[derive(Debug)]
pub struct PickedAsset {
    pub tag: String,
    pub name: String,
    pub url: String,
    pub checksum_url: Option<String>,
}

/// Pick the right release asset for linux on this arch.
/// Scored: name must mention linux or be arch-neutral; must mention the
/// right arch (x86_64/amd64 or aarch64/arm64); archive suffixes preferred.
pub fn pick_asset(release: &GhRelease, arch: &str) -> Option<PickedAsset> {
    let arch_names: &[&str] = if arch == "aarch64" {
        &["aarch64", "arm64"]
    } else {
        &["x86_64", "x64", "amd64"]
    };

    let mut best: Option<(u32, &GhAsset)> = None;
    for asset in &release.assets {
        let name = asset.name.to_lowercase();
        // skip checksums/signatures/source archives — they describe assets
        if name.ends_with(".sha256")
            || name.ends_with(".sha256sum")
            || name.ends_with(".sig")
            || name.contains("checksum")
            || name.ends_with(".txt")
            || name.ends_with(".deb")
            || name.ends_with(".rpm")
            || name.ends_with(".exe")
            || name.contains("windows")
            || name.contains("darwin")
            || name.contains("macos")
            || name.contains(".appimage")
        {
            continue;
        }
        let mut score = 0u32;
        if name.contains("linux") {
            score += 10;
        }
        if arch_names.iter().any(|a| name.contains(a)) {
            score += 20;
        }
        if name.ends_with(".tar.gz") || name.ends_with(".tgz") || name.ends_with(".zip") {
            score += 5;
        }
        if score < 20 {
            continue; // no arch match — never install a wrong-arch binary
        }
        if best.as_ref().is_none_or(|(s, _)| score > *s) {
            best = Some((score, asset));
        }
    }

    let (_, asset) = best?;
    let checksum_url = release
        .assets
        .iter()
        .find(|a| {
            let n = a.name.to_lowercase();
            n.contains("checksum") || n.ends_with(".sha256") || n.ends_with(".sha256sum")
        })
        .map(|a| a.browser_download_url.clone());
    Some(PickedAsset {
        tag: release.tag_name.clone(),
        name: asset.name.clone(),
        url: asset.browser_download_url.clone(),
        checksum_url,
    })
}

async fn latest_release(client: &reqwest::Client, repo: &str) -> PxResult<GhRelease> {
    let url = format!("https://api.github.com/repos/{repo}/releases/latest");
    let release: GhRelease = client
        .get(url)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    Ok(release)
}

fn arch() -> &'static str {
    if cfg!(target_arch = "aarch64") {
        "aarch64"
    } else {
        "x86_64"
    }
}

/// Full release-binary flow: latest release → right asset → download →
/// checksum-verify → unpack → install executable to ~/.local/bin.
/// Returns the installed binary name.
pub async fn install(app: &App, repo: &str) -> PxResult<Option<String>> {
    let release = latest_release(&app.client, repo).await?;
    let Some(asset) = pick_asset(&release, arch()) else {
        return Err(PxError::User(format!(
            "no release asset for linux/{} in {}'s latest release ({})",
            arch(),
            repo,
            release.tag_name
        )));
    };

    println!(
        "    {} latest release: {} → {}",
        app.style.ok("✓"),
        app.style.bold(&asset.tag),
        app.style.dim(&asset.name)
    );

    // download into the px cache
    let dir = crate::cache::cache_root()
        .join("releases")
        .join(repo.replace('/', "__"));
    std::fs::create_dir_all(&dir)
        .map_err(|e| PxError::User(format!("cannot create release cache: {e}")))?;
    let archive_path = dir.join(&asset.name);
    let bytes = app
        .client
        .get(&asset.url)
        .send()
        .await?
        .error_for_status()?
        .bytes()
        .await?;
    std::fs::write(&archive_path, &bytes)
        .map_err(|e| PxError::User(format!("cannot write archive: {e}")))?;

    // verify checksum when the release publishes one
    if let Some(checksum_url) = &asset.checksum_url {
        let sums = app.client.get(checksum_url).send().await?.text().await?;
        if let Some(expected) = expected_sha(&sums, &asset.name) {
            let actual = sha256_hex(&bytes);
            if actual == expected {
                println!("    {} checksum verified", app.style.ok("✓"));
            } else {
                return Err(PxError::User(format!(
                    "checksum MISMATCH for {name}: expected {expected}, got {actual}",
                    name = asset.name
                )));
            }
        }
    } else {
        println!(
            "    {} no published checksum — proceeding without verification",
            app.style.warn("⚠")
        );
    }

    // unpack and find the executable
    let unpack_dir = dir.join("unpacked");
    let _ = std::fs::remove_dir_all(&unpack_dir);
    std::fs::create_dir_all(&unpack_dir)
        .map_err(|e| PxError::User(format!("cannot create unpack dir: {e}")))?;
    unpack(&archive_path, &unpack_dir)?;

    // the executable: a file matching the repo's binary name, else the
    // single executable file in the archive
    let repo_bin = repo.rsplit('/').next().unwrap_or("").to_string();
    let binary = find_executable(&unpack_dir, &repo_bin)
        .ok_or_else(|| PxError::User("no executable found in the release archive".into()))?;

    // install to ~/.local/bin
    let bin_dir = dirs::home_dir()
        .map(|h| h.join(".local/bin"))
        .ok_or_else(|| PxError::User("no home directory".into()))?;
    std::fs::create_dir_all(&bin_dir)
        .map_err(|e| PxError::User(format!("cannot create ~/.local/bin: {e}")))?;
    let dest = bin_dir.join(binary.file_name().unwrap_or_default());
    make_executable(&binary)?;
    std::fs::copy(&binary, &dest)
        .map_err(|e| PxError::User(format!("cannot install binary: {e}")))?;
    println!(
        "    {} installed {} → {}",
        app.style.ok("✓"),
        binary
            .file_name()
            .map(|n| n.to_string_lossy())
            .unwrap_or_default(),
        dest.display()
    );

    Ok(binary.file_name().map(|n| n.to_string_lossy().into_owned()))
}

fn expected_sha(sums: &str, asset_name: &str) -> Option<String> {
    for line in sums.lines() {
        let mut parts = line.split_whitespace();
        let hash = parts.next()?;
        let name = parts.next_back()?.trim_matches('*');
        if name == asset_name && hash.len() == 64 {
            return Some(hash.to_lowercase());
        }
    }
    None
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(bytes);
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

fn unpack(archive: &std::path::Path, dest: &std::path::Path) -> PxResult<()> {
    let name = archive
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let status = if name.ends_with(".zip") {
        std::process::Command::new("unzip")
            .arg("-oq")
            .arg(archive)
            .arg("-d")
            .arg(dest)
            .status()
    } else {
        std::process::Command::new("tar")
            .arg("xf")
            .arg(archive)
            .arg("-C")
            .arg(dest)
            .status()
    }
    .map_err(|e| PxError::User(format!("cannot run unpacker: {e}")))?;
    if status.success() {
        Ok(())
    } else {
        Err(PxError::User("failed to unpack the release archive".into()))
    }
}

fn find_executable(dir: &std::path::Path, preferred: &str) -> Option<PathBuf> {
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;
    let mut preferred_hit: Option<PathBuf> = None;
    let mut any_exec: Option<PathBuf> = None;
    for entry in walkdir::WalkDir::new(dir)
        .max_depth(3)
        .into_iter()
        .flatten()
    {
        let Ok(meta) = entry.metadata() else { continue };
        if !meta.is_file() {
            continue;
        }
        let name = entry.file_name().to_string_lossy();
        #[cfg(unix)]
        let exec_bit = meta.permissions().mode() & 0o111 != 0;
        #[cfg(not(unix))]
        let exec_bit = true;
        if !exec_bit && !name.contains(preferred) {
            continue;
        }
        if name.contains(preferred) && preferred_hit.is_none() {
            preferred_hit = Some(entry.into_path());
        } else if any_exec.is_none() {
            any_exec = Some(entry.into_path());
        }
    }
    preferred_hit.or(any_exec)
}

fn make_executable(path: &std::path::Path) -> PxResult<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(path)
            .map_err(|e| PxError::User(format!("stat: {e}")))?
            .permissions();
        perms.set_mode(perms.mode() | 0o755);
        std::fs::set_permissions(path, perms).map_err(|e| PxError::User(format!("chmod: {e}")))?;
    }
    Ok(())
}
