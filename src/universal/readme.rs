//! Upstream documentation parsing: fetch a repo's README and extract the
//! install commands it documents. A documented command (confidence 90) is
//! worth far more than finding a random install.sh in the tree (60).

use regex::Regex;

use super::{Candidate, Method};
use crate::error::PxResult;

/// Fetch the repo's README (tries main then master) and extract documented
/// install methods. Best-effort: network failures return empty.
pub async fn discover(client: &reqwest::Client, repo: &str) -> Vec<Candidate> {
    let mut candidates = Vec::new();

    let readme = fetch_readme(client, repo).await;
    let Some(readme) = readme else {
        return candidates;
    };

    // cargo install <crate>
    if let Some(c) = find(&readme, r"cargo\s+install\s+([A-Za-z0-9_-]+)") {
        candidates.push(Candidate {
            method: Method::Cargo {
                crate_name: c.clone(),
            },
            confidence: 90,
            note: format!("documented in README (cargo install {c})"),
            pinned_sha: None,
        });
    }

    // brew install <tap/formula or formula>
    if let Some(b) = find(&readme, r"brew\s+install\s+([A-Za-z0-9_/@-]+)") {
        candidates.push(Candidate {
            method: Method::Brew { tap: b.clone() },
            confidence: 90,
            note: format!("documented in README (brew install {b})"),
            pinned_sha: None,
        });
    }

    // npm install -g <pkg> / npm i -g <pkg>
    if let Some(n) = find(&readme, r"npm\s+(?:install|i)\s+-g\s+(@?[A-Za-z0-9/.-]+)") {
        candidates.push(Candidate {
            method: Method::Npm { package: n.clone() },
            confidence: 90,
            note: format!("documented in README (npm install -g {n})"),
            pinned_sha: None,
        });
    }

    // pipx install <pkg>
    if let Some(p) = find(&readme, r"pipx\s+install\s+([A-Za-z0-9_.-]+)") {
        candidates.push(Candidate {
            method: Method::Pipx { package: p.clone() },
            confidence: 90,
            note: format!("documented in README (pipx install {p})"),
            pinned_sha: None,
        });
    }

    // go install <module>@version
    if let Some(g) = find(&readme, r"go\s+install\s+([A-Za-z0-9./_-]+)@") {
        candidates.push(Candidate {
            method: Method::Go { module: g.clone() },
            confidence: 90,
            note: format!("documented in README (go install {g})"),
            pinned_sha: None,
        });
    }

    // curl -fsSL <url> | sh|bash  (or wget -qO- <url> | sh)
    let script_re = Regex::new(
        r#"(?:curl\s+-?f?s?S?L?\s+\S*|wget\s+-qO-\s+\S*)['"]?(https?://[^'"|\s]+)['"]?\s*\|\s*(?:sudo\s+)?(?:ba|z|da)?sh"#,
    )
    .unwrap();
    if let Some(caps) = script_re.captures(&readme)
        && let Some(url) = caps.get(1) {
            candidates.push(Candidate {
                method: Method::Script {
                    url: url.as_str().to_string(),
                },
                confidence: 60,
                note: "installer script referenced in README".into(),
                pinned_sha: None,
            });
        }

    candidates
}

fn find(text: &str, pattern: &str) -> Option<String> {
    let re = Regex::new(pattern).ok()?;
    re.captures(text)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().to_string())
}

async fn fetch_readme(client: &reqwest::Client, repo: &str) -> Option<String> {
    for branch in ["main", "master"] {
        for name in ["README.md", "README.MD", "readme.md", "README"] {
            let url = format!("https://raw.githubusercontent.com/{repo}/{branch}/{name}");
            if let Ok(resp) = client.get(&url).send().await
                && resp.status().is_success()
                    && let Ok(text) = resp.text().await {
                        return Some(text);
                    }
        }
    }
    None
}

/// Test hook: parse methods out of already-fetched README text.
pub fn parse_readme_text(readme: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    if let Some(c) = find(readme, r"cargo\s+install\s+([A-Za-z0-9_-]+)") {
        out.push(("cargo".into(), c));
    }
    if let Some(b) = find(readme, r"brew\s+install\s+([A-Za-z0-9_/@-]+)") {
        out.push(("brew".into(), b));
    }
    if let Some(n) = find(readme, r"npm\s+(?:install|i)\s+-g\s+(@?[A-Za-z0-9/.-]+)") {
        out.push(("npm".into(), n));
    }
    out
}

// keep PxResult import used even if signature set changes
#[allow(dead_code)]
fn _unused(_: PxResult<()>) {}
