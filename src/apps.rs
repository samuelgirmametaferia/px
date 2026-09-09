//! Smart app installs: some applications don't live in distro repos — they
//! install via npm (claude-code, codex) or a curl|sh installer (bun, deno).
//! The recipe's [[apps]] registry is data; the npm registry is checked
//! generically when npm exists. Checked after package sources, before the
//! GitHub fallback. Every app install shows exactly what will run.

use serde::Deserialize;

use crate::app::App;
use crate::error::{PxError, PxResult};
use crate::recipe::schema::AppDef;

/// What px will do to install an app.
#[derive(Debug, Clone)]
pub enum AppInstall {
    /// `npm install -g <package>` (after investigation for suspicious ones)
    Npm { package: String, label: String },
    /// `curl -fsSL <url> | sh` — shown verbatim, dedicated confirmation.
    Script { url: String, label: String },
}

/// Look up a spec in the recipe's [[apps]] registry.
pub fn registry_lookup(app: &App, spec: &str) -> Option<AppInstall> {
    let spec_lc = spec.to_lowercase();
    let hit: &AppDef = app
        .recipe()
        .apps
        .iter()
        .find(|a| a.match_.iter().any(|m| m.to_lowercase() == spec_lc))?;
    build_install(hit)
}

/// Generic npm fallback: does this name exist on the npm registry?
pub async fn npm_lookup(client: &reqwest::Client, spec: &str) -> PxResult<Option<AppInstall>> {
    if which::which("npm").is_err() {
        return Ok(None); // no npm, no npm installs
    }
    let url = format!("https://registry.npmjs.org/{}", spec.replace('/', "%2f"));
    let resp = match client.head(&url).send().await {
        Ok(r) => r,
        Err(_) => return Ok(None),
    };
    if resp.status().is_success() {
        Ok(Some(AppInstall::Npm {
            package: spec.to_string(),
            label: spec.to_string(),
        }))
    } else {
        Ok(None)
    }
}

fn build_install(def: &AppDef) -> Option<AppInstall> {
    // required binaries present?
    if !def.require_any.is_empty() && !def.require_any.iter().any(|b| which::which(b).is_ok()) {
        return None;
    }
    match def.method.as_str() {
        "npm" => def.package.clone().map(|package| AppInstall::Npm {
            package,
            label: def.label.clone(),
        }),
        "script" => def.url.clone().map(|url| AppInstall::Script {
            url,
            label: def.label.clone(),
        }),
        other => {
            tracing::warn!("unknown app method '{other}' for {}", def.label);
            None
        }
    }
}

/// Execute an app install. `force` adds npm's `--force` (used when a
/// previous install left the binary in place and the user chose to
/// overwrite). Script installs show their pipe command verbatim.
pub async fn install(_app: &App, install: &AppInstall, dry_run: bool, force: bool) -> PxResult<()> {
    match install {
        AppInstall::Npm { package, .. } => {
            let mut argv: Vec<&str> = vec!["npm", "install", "-g"];
            if force {
                argv.push("--force");
            }
            argv.push(package);
            run(&argv, dry_run).await
        }
        AppInstall::Script { url, .. } => {
            // Show exactly what runs — curl | sh is a trust decision.
            let curl = crate::exec::resolve_bin("curl");
            let sh = crate::exec::resolve_bin("sh");
            if dry_run {
                println!("[dry-run] {curl} -fsSL {url} | {sh}");
                return Ok(());
            }
            crate::ui::prompt::flush();
            let mut curl_proc = tokio::process::Command::new(&curl)
                .args(["-fsSL", url])
                .stdout(std::process::Stdio::piped())
                .spawn()
                .map_err(|e| PxError::User(format!("cannot run curl: {e}")))?;
            let curl_stdout = curl_proc
                .stdout
                .take()
                .ok_or_else(|| PxError::User("curl produced no output pipe".into()))?;
            let fd = curl_stdout
                .into_owned_fd()
                .map_err(|e| PxError::User(format!("pipe error: {e}")))?;
            let sh_proc = tokio::process::Command::new(&sh)
                .stdin(std::process::Stdio::from(fd))
                .status()
                .await
                .map_err(|e| PxError::User(format!("cannot run sh: {e}")))?;
            let _ = curl_proc.wait().await;
            if sh_proc.success() {
                Ok(())
            } else {
                Err(PxError::User("installer script failed".into()))
            }
        }
    }
}

async fn run(argv: &[&str], dry_run: bool) -> PxResult<()> {
    if dry_run {
        println!("[dry-run] {}", argv.join(" "));
        return Ok(());
    }
    crate::ui::prompt::flush();
    let status = tokio::process::Command::new(argv[0])
        .args(&argv[1..])
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
            stderr: "install failed (output above)".into(),
        })
    }
}

/// npm registry metadata for investigation reports (flattened from the
/// packument: the interesting bits — scripts, dist — live per-version, so
/// we pull them from the latest).
#[derive(Debug, Default)]
pub struct NpmMeta {
    pub name: String,
    pub description: Option<String>,
    pub unpacked_size: Option<u64>,
    pub versions: usize,
    pub created: Option<String>,
    pub modified: Option<String>,
    pub maintainers: Vec<NpmUser>,
    pub scripts: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct NpmUser {
    #[serde(default)]
    pub name: String,
}

pub async fn npm_meta(client: &reqwest::Client, package: &str) -> PxResult<NpmMeta> {
    let url = format!("https://registry.npmjs.org/{}", package.replace('/', "%2f"));
    let doc: serde_json::Value = client
        .get(url)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;

    let latest = doc
        .get("dist-tags")
        .and_then(|t| t.get("latest"))
        .and_then(|v| v.as_str());

    let latest_doc = latest.and_then(|v| doc.get("versions").and_then(|vs| vs.get(v)));

    let scripts = latest_doc
        .and_then(|v| v.get("scripts"))
        .and_then(|s| s.as_object())
        .map(|o| o.keys().cloned().collect())
        .unwrap_or_default();

    let unpacked_size = latest_doc
        .and_then(|v| v.get("dist"))
        .and_then(|d| d.get("unpackedSize"))
        .and_then(|s| s.as_u64());

    let maintainers = doc
        .get("maintainers")
        .and_then(|m| m.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|u| serde_json::from_value::<NpmUser>(u.clone()).ok())
                .collect()
        })
        .unwrap_or_default();

    Ok(NpmMeta {
        name: doc
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string(),
        description: doc
            .get("description")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        unpacked_size,
        versions: doc
            .get("versions")
            .and_then(|v| v.as_object())
            .map(|o| o.len())
            .unwrap_or(0),
        created: doc
            .get("time")
            .and_then(|t| t.get("created"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        modified: doc
            .get("time")
            .and_then(|t| t.get("modified"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        maintainers,
        scripts,
    })
}
