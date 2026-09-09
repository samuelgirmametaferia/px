//! Universal project identity resolution.
//!
//! The rule this module enforces: **a matching name in a registry proves
//! nothing about project identity**. npm's `agent-code` (no repo, no
//! versions) is not avala-ai's `agent-code` (48 crate releases). Resolution
//! therefore establishes a canonical PROJECT first — curated registry →
//! crates.io repository link → GitHub search → README evidence — and only
//! then offers install methods, each carrying a confidence score. Same-name
//! packages with no identity link top out at LOW confidence and are never
//! installed silently.

pub mod exec;
pub mod readme;
pub mod registry;
pub mod releasebin;

use serde::Deserialize;

use crate::app::App;
use crate::error::{PxError, PxResult};

/// A canonical project identity.
#[derive(Debug, Clone)]
pub struct Project {
    /// "avala-ai/agent-code" (github) — the identity anchor.
    pub canonical: String,
    pub description: Option<String>,
    /// Executable the project installs, when known (package ≠ binary!).
    pub binary: Option<String>,
    /// How the identity was established.
    pub evidence: String,
}

/// Every way a project can be installed, with a confidence score.
/// Confidence (per the spec):
///   100 officially documented (curated registry entry)
///    90 README installation section references it
///    80 official release with a checksum we can verify
///    60 installer exists but is poorly documented
///    20 same-name package with NO identity link — never silent
#[derive(Debug, Clone)]
pub struct Candidate {
    pub method: Method,
    pub confidence: u32,
    pub note: String,
    /// Registry-pinned installer sha256: when set, the live installer must
    /// hash to exactly this or the install STOPS.
    pub pinned_sha: Option<String>,
}

#[derive(Debug, Clone)]
pub enum Method {
    /// Distro-native package (repo/aur/...) — handled before universal.
    Native,
    /// Verified upstream release binary: download, checksum, unpack.
    Release { repo: String },
    /// cargo install <crate> (build scripts run — sandboxed).
    Cargo { crate_name: String },
    /// npm install -g <pkg> — only with a verified repository link.
    Npm { package: String },
    /// pipx install <pkg>
    Pipx { package: String },
    /// go install <module>@latest
    Go { module: String },
    /// gem install <gem>
    Gem { gem: String },
    /// brew install <tap> (brew only exists on macOS/Linuxbrew boxes)
    Brew { tap: String },
    /// upstream installer script — static-scanned + sandboxed, last resort
    Script { url: String },
    /// source build from the repo
    Source { repo: String },
}

impl Method {
    /// Rank for install preference (1 = best). Native > release binary >
    /// language package managers > brew > script > source.
    pub fn rank(&self) -> u8 {
        match self {
            Method::Native => 1,
            Method::Release { .. } => 2,
            Method::Cargo { .. }
            | Method::Npm { .. }
            | Method::Pipx { .. }
            | Method::Go { .. }
            | Method::Gem { .. } => 3,
            Method::Brew { .. } => 4,
            Method::Script { .. } => 5,
            Method::Source { .. } => 6,
        }
    }

    pub fn label(&self) -> String {
        match self {
            Method::Native => "system package".into(),
            Method::Release { repo } => format!("release binary ({repo})"),
            Method::Cargo { crate_name } => format!("cargo install {crate_name}"),
            Method::Npm { package } => format!("npm install -g {package}"),
            Method::Pipx { package } => format!("pipx install {package}"),
            Method::Go { module } => format!("go install {module}@latest"),
            Method::Gem { gem } => format!("gem install {gem}"),
            Method::Brew { tap } => format!("brew install {tap}"),
            Method::Script { url } => format!("installer ({url})"),
            Method::Source { repo } => format!("source build ({repo})"),
        }
    }

    /// Tools this method needs on PATH.
    pub fn requires(&self) -> Vec<&'static str> {
        match self {
            Method::Cargo { .. } => vec!["cargo"],
            Method::Npm { .. } => vec!["npm"],
            Method::Pipx { .. } => vec!["pipx"],
            Method::Go { .. } => vec!["go"],
            Method::Gem { .. } => vec!["gem"],
            Method::Brew { .. } => vec!["brew"],
            Method::Script { .. } => vec!["curl"],
            Method::Release { .. } => vec![],
            Method::Source { .. } => vec![],
            Method::Native => vec![],
        }
    }
}

/// Candidates sorted best-first: rank first, confidence second.
pub fn rank_candidates(mut candidates: Vec<Candidate>) -> Vec<Candidate> {
    candidates.retain(|c| {
        // a method is only a candidate if its tools exist on this machine
        c.method.requires().iter().all(|t| which::which(t).is_ok())
    });
    candidates.sort_by(|a, b| {
        a.method
            .rank()
            .cmp(&b.method.rank())
            .then(b.confidence.cmp(&a.confidence))
    });
    candidates
}

// ---------------------------------------------------------- identity sources

/// Resolve a spec into a canonical project, with candidates.
/// Order of strength: curated registry (confidence 100) → crates.io repo →
/// GitHub search → npm (identity-checked only).
pub async fn resolve(app: &App, spec: &str) -> PxResult<Option<(Project, Vec<Candidate>)>> {
    // 1. curated registry — strongest signal, methods pre-scored
    if let Some(entry) = registry::lookup(spec) {
        let project = Project {
            canonical: entry.project.clone(),
            description: Some(entry.description.clone()),
            binary: entry.binary.clone(),
            evidence: "curated registry (officially documented methods)".into(),
        };
        let candidates: Vec<Candidate> = entry
            .methods
            .iter()
            .map(|m| Candidate {
                method: m.to_method(),
                confidence: m.confidence,
                note: m.note.clone(),
                pinned_sha: None,
            })
            .collect();
        return Ok(Some((project, candidates)));
    }

    // 1b. the upstream fallback registry: deterministic, sharded, pinned
    //     installer hashes. Consulted BEFORE ad-hoc discovery so verified
    //     records beat guesses.
    if let Some(source) = crate::registry::source_from_config(&app.config) {
        match crate::registry::lookup(&app.client, &source, spec).await {
            Ok(Some(record)) if !record.is_dead() => {
                let project = Project {
                    canonical: record.canonical_id.clone(),
                    description: Some(record.description.clone()),
                    binary: record.expected_binaries.first().cloned(),
                    evidence: format!(
                        "px upstream registry (confidence {}, {})",
                        record.identity_confidence, record.security_state
                    ),
                };
                let candidates: Vec<Candidate> = record
                    .install_methods
                    .iter()
                    .enumerate()
                    .filter_map(|(i, m)| {
                        let method = registry_method_to_method(m)?;
                        Some(Candidate {
                            method,
                            confidence: record.identity_confidence,
                            note: match &m.installer_sha256 {
                                Some(_) => format!("registry method {i} (installer hash pinned)"),
                                None => format!("registry method {i}"),
                            },
                            pinned_sha: m.installer_sha256.clone(),
                        })
                    })
                    .collect();
                if !candidates.is_empty() {
                    return Ok(Some((project, candidates)));
                }
            }
            Ok(Some(dead)) => {
                // tombstone: the identity exists but is dead — say so and
                // never let a namesake hijack resolution
                println!(
                    "\n  {} {} is recorded in the px registry as {} — refusing",
                    app.style.warn("⚠"),
                    app.style.bold(&dead.canonical_id),
                    app.style.bold(&dead.security_state)
                );
                println!(
                    "    {} a new project taking over this name cannot hijack resolution",
                    app.style.dim("·")
                );
                return Ok(None);
            }
            Ok(None) => {}
            Err(e) => {
                tracing::debug!("registry lookup failed: {e}");
                // fall through to ad-hoc resolution
            }
        }
    }

    // 2. crates.io: a crate with a repository link establishes identity
    //    AND a high-confidence cargo method in one shot — but ONLY if the
    //    crate actually produces a binary (many crates are libraries; the
    //    is-even case: name exists on crates.io AND npm, the crate is a lib).
    if let Some((repo, mut desc)) = cratesio_identity(&app.client, spec).await {
        let mut candidates = Vec::new();
        if crate_has_binaries(&app.client, &repo).await {
            candidates.push(Candidate {
                method: Method::Cargo {
                    crate_name: spec.to_string(),
                },
                confidence: 90,
                note: "published on crates.io".into(),
                pinned_sha: None,
            });
        }
        // README may document more methods (brew tap, installer…)
        candidates.extend(readme::discover(&app.client, &repo).await);
        candidates.push(Candidate {
            method: Method::Source { repo: repo.clone() },
            confidence: 60,
            note: "build from the repository".into(),
            pinned_sha: None,
        });
        // crates.io identity but no installable binary → npm may be the
        // real distribution channel (name equality is NOT identity, but the
        // npm package's repository link lets us verify it)
        if let Some((npm_repo, npm_desc)) = npm_identity(&app.client, spec).await
            && npm_repo.as_deref() == Some(repo.as_str())
        {
            candidates.push(Candidate {
                method: Method::Npm {
                    package: spec.to_string(),
                },
                confidence: 90,
                note: "npm package linked to the same repository".into(),
                pinned_sha: None,
            });
            desc = desc.or(npm_desc);
        }
        let project = Project {
            canonical: repo,
            description: desc,
            binary: None, // learned after install / from README
            evidence: "crates.io crate with a repository link".into(),
        };
        return Ok(Some((project, candidates)));
    }

    // 3. GitHub search: the repo itself is the identity — but only when
    //    the repo NAME matches the query. A text-search hit on an unrelated
    //    repo ("is-even" → Zyphra/Zonos) is identity 20: shown, never
    //    silently installed.
    let hits = crate::github::search(&app.client, spec, 3)
        .await
        .unwrap_or_default();
    if let Some(mut hit) = hits.first().cloned() {
        let repo_name = hit
            .full_name
            .rsplit('/')
            .next()
            .unwrap_or_default()
            .to_lowercase();
        let spec_norm = crate::registry::normalize_alias(spec);
        let name_match = repo_name == spec_norm
            || repo_name.contains(&spec_norm)
            || spec_norm.contains(&repo_name);
        let identity_confidence = if name_match { 70 } else { 20 };
        let mut candidates = readme::discover(&app.client, &hit.full_name).await;
        // npm cross-verification: an npm package of this name whose
        // repository field points at the repo we found = the same project,
        // verified from two independent sources
        if let Some((npm_repo, npm_desc)) = npm_identity(&app.client, spec).await
            && npm_repo.as_deref() == Some(hit.full_name.as_str())
        {
            candidates.push(Candidate {
                method: Method::Npm {
                    package: spec.to_string(),
                },
                confidence: 90,
                note: "npm package linked to this repository (cross-verified)".into(),
                pinned_sha: None,
            });
            if hit.description.is_none() && npm_desc.is_some() {
                hit.description = npm_desc;
            }
        }
        // documented install methods are authoritative — the generic
        // release/source guesses only appear when nothing else is known
        // (spec: documented > found-on-disk)
        if candidates.is_empty() {
            candidates.push(Candidate {
                method: Method::Release {
                    repo: hit.full_name.clone(),
                },
                confidence: identity_confidence,
                note: "latest GitHub release".into(),
                pinned_sha: None,
            });
            candidates.push(Candidate {
                method: Method::Source {
                    repo: hit.full_name.clone(),
                },
                confidence: identity_confidence,
                note: "build from the repository".into(),
                pinned_sha: None,
            });
        }
        for c in &mut candidates {
            // README-documented methods keep their own confidence when the
            // repo name matched; otherwise they're capped by the weak identity
            if !name_match {
                c.confidence = c.confidence.min(20);
            }
        }
        let project = Project {
            canonical: hit.full_name.clone(),
            description: hit.description.clone(),
            binary: None,
            evidence: if name_match {
                "GitHub repository search (name matches)".into()
            } else {
                "GitHub text search — IDENTITY UNVERIFIED, name does not match".into()
            },
        };
        return Ok(Some((project, candidates)));
    }

    // 4. npm — ONLY with identity evidence. A bare name match on npm is
    //    confidence 20 and requires an explicit "this may be unrelated
    //    software" confirmation. This is the agent-code fix.
    if let Some((npm_repo, desc)) = npm_identity(&app.client, spec).await {
        let canonical = match &npm_repo {
            Some(r) => r.clone(),
            None => format!("npm:{spec}"),
        };
        let (confidence, note) = if npm_repo.is_some() {
            (
                90,
                "npm package with a matching repository link".to_string(),
            )
        } else {
            (
                20,
                "npm package exists but has NO repository link — identity unverified, ".to_string()
                    + "it may be unrelated software squatting the name",
            )
        };
        let project = Project {
            canonical,
            description: desc,
            binary: None,
            evidence: if npm_repo.is_some() {
                "npm registry".into()
            } else {
                "npm registry (unverified identity)".into()
            },
        };
        return Ok(Some((
            project,
            vec![Candidate {
                method: Method::Npm {
                    package: spec.to_string(),
                },
                confidence,
                note,
                pinned_sha: None,
            }],
        )));
    }

    Ok(None)
}

/// Convert a registry install method into a resolver Method.
fn registry_method_to_method(m: &crate::registry::schema::RegistryMethod) -> Option<Method> {
    Some(match m.method.as_str() {
        "script" => Method::Script {
            url: m.url.clone()?,
        },
        "release" => Method::Release {
            repo: m
                .release_url
                .as_deref()?
                .trim_start_matches("https://github.com/")
                .split("/releases")
                .next()?
                .to_string(),
        },
        "cargo" => Method::Cargo {
            crate_name: m.crate_name.clone()?,
        },
        "npm" => Method::Npm {
            package: m.package.clone()?,
        },
        "pipx" => Method::Pipx {
            package: m.package.clone()?,
        },
        "go" => Method::Go {
            module: m.module.clone()?,
        },
        "gem" => Method::Gem {
            gem: m.gem.clone()?,
        },
        "brew" => Method::Brew {
            tap: m.tap.clone()?,
        },
        "source" => Method::Source {
            repo: m
                .url
                .as_deref()?
                .trim_start_matches("https://github.com/")
                .trim_end_matches(".git")
                .to_string(),
        },
        other => {
            tracing::warn!("unknown registry method '{other}'");
            return None;
        }
    })
}

/// Does this crate's repository produce an installable binary?
/// Checks Cargo.toml for [[bin]] or the presence of src/main.rs. Library-
/// only crates (the is-even case) must not be offered via cargo install.
async fn crate_has_binaries(client: &reqwest::Client, repo: &str) -> bool {
    for branch in ["main", "master"] {
        let url = format!("https://raw.githubusercontent.com/{repo}/{branch}/Cargo.toml");
        let Ok(resp) = client.get(&url).send().await else {
            continue;
        };
        if !resp.status().is_success() {
            continue;
        }
        let Ok(toml_text) = resp.text().await else {
            continue;
        };
        if toml_text.contains("[[bin]]") {
            return true;
        }
        if toml_text.contains("[lib]") && !toml_text.contains("path = \"src/main.rs\"") {
            // explicit lib section without a bin — check for main.rs anyway
            let main_url = format!("https://raw.githubusercontent.com/{repo}/{branch}/src/main.rs");
            if let Ok(r) = client.get(main_url).send().await {
                return r.status().is_success();
            }
            return false;
        }
        // no lib section: a default src/main.rs binary is the Cargo default
        let main_url = format!("https://raw.githubusercontent.com/{repo}/{branch}/src/main.rs");
        if let Ok(r) = client.get(main_url).send().await {
            return r.status().is_success();
        }
    }
    // can't tell — don't punish unknown repos; cargo install will error
    // clearly if it's a library
    true
}

#[derive(Deserialize)]
struct CratesCrate {
    #[serde(default)]
    repository: Option<String>,
    #[serde(default)]
    description: Option<String>,
}

#[derive(Deserialize)]
struct CratesResponse {
    #[serde(default, rename = "crate")]
    krate: Option<CratesCrate>,
}

/// crates.io lookup: returns (normalized github repo, description) when a
/// crate exists AND declares a repository.
async fn cratesio_identity(
    client: &reqwest::Client,
    name: &str,
) -> Option<(String, Option<String>)> {
    // disk cache: crates.io rate-limits hard, and identity doesn't change
    let cache_key = format!("cratesio:{name}");
    if let Some(cached) =
        crate::cache::get("identity", &cache_key, std::time::Duration::from_secs(3600))
    {
        if cached == "miss" {
            return None;
        }
        if let Ok(pair) = serde_json::from_str::<(String, Option<String>)>(&cached) {
            return Some(pair);
        }
    }
    let url = format!("https://crates.io/api/v1/crates/{name}");
    let resp = client
        .get(url)
        .header(
            "User-Agent",
            "px package manager (github.com/samuelgirmametaferia/px)",
        )
        .send()
        .await
        .ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let body: CratesResponse = resp.json().await.ok()?;
    let krate = body.krate?;
    let repo = krate.repository?;
    let normalized = normalize_repo(&repo)?;
    let pair = (normalized, krate.description);
    if let Ok(json) = serde_json::to_string(&pair) {
        crate::cache::put("identity", &cache_key, &json);
    }
    Some(pair)
}

#[derive(Deserialize)]
struct NpmRepository {
    #[serde(default)]
    url: Option<String>,
}

#[derive(Deserialize)]
struct NpmDoc {
    #[serde(default)]
    repository: Option<NpmRepository>,
    #[serde(default)]
    description: Option<String>,
}

/// npm lookup: returns (Some(repo) when identity is verifiable via a
/// repository link, None when it is a bare name match).
async fn npm_identity(
    client: &reqwest::Client,
    name: &str,
) -> Option<(Option<String>, Option<String>)> {
    let url = format!("https://registry.npmjs.org/{}", name.replace('/', "%2f"));
    let resp = client.get(url).send().await.ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let doc: NpmDoc = resp.json().await.ok()?;
    let repo = doc
        .repository
        .and_then(|r| r.url)
        .and_then(|u| normalize_repo(&u));
    Some((repo, doc.description))
}

/// Normalize any repository URL form to "owner/repo".
/// Handles https://github.com/o/r, git+ssh://git@github.com:o/r.git, etc.
pub fn normalize_repo(url: &str) -> Option<String> {
    let url = url
        .trim()
        .trim_start_matches("git+")
        .replace("git@github.com:", "https://github.com/");
    let path = url
        .strip_prefix("https://github.com/")
        .or_else(|| url.strip_prefix("http://github.com/"))?;
    let path = path.trim_end_matches(".git");
    let mut parts = path.split('/');
    let owner = parts.next()?.trim();
    let repo = parts.next()?.trim();
    if owner.is_empty() || repo.is_empty() || repo.contains('/') {
        return None;
    }
    Some(format!("{owner}/{repo}"))
}

/// The full universal install flow for one spec. Returns true when handled.
pub async fn try_install(app: &App, spec: &str) -> PxResult<bool> {
    let style = &app.style;
    let Some((project, candidates)) = resolve(app, spec).await? else {
        return Ok(false);
    };
    let candidates = rank_candidates(candidates);
    if candidates.is_empty() {
        return Ok(false);
    }

    println!(
        "\n  {} resolved {} → {}",
        style.ok("✓"),
        style.bold(spec),
        style.bold(&project.canonical)
    );
    if let Some(d) = &project.description {
        println!("    {} {}", style.dim("·"), style.dim(d));
    }
    println!(
        "    {} {}",
        style.dim("identity:"),
        style.dim(&project.evidence)
    );

    // Show every method with its confidence — the user sees the ranking.
    println!("\n    {} installation methods:", style.header("›"));
    for (i, c) in candidates.iter().enumerate() {
        println!(
            "    {} {} {} {}",
            style.dim(&format!("{}.", i + 1)),
            style.bold(&c.method.label()),
            style.dim(&format!("[confidence {}]", c.confidence)),
            style.dim(&c.note)
        );
    }

    if app.cli.dry_run {
        println!(
            "\n    {} would install via {}",
            style.dim("[dry-run]"),
            candidates[0].method.label()
        );
        return Ok(true);
    }

    // Low-confidence candidates are NEVER silent: explicit scary confirm.
    let pick = if candidates.len() == 1 {
        0
    } else if crate::ui::interactive() && !app.cli.yes {
        let items: Vec<String> = candidates
            .iter()
            .map(|c| {
                format!(
                    "{:<40} [{}/{}]",
                    c.method.label(),
                    c.method.rank(),
                    c.confidence
                )
            })
            .collect();
        crate::ui::prompt::select("install using", &items)?
    } else {
        0
    };
    let chosen = candidates[pick].clone();

    if chosen.confidence < 50 {
        println!(
            "\n    {} {}",
            style.warn("⚠"),
            style.warn(&format!(
                "low confidence ({}) — {}",
                chosen.confidence, chosen.note
            ))
        );
        if crate::ui::interactive()
            && !crate::ui::prompt::confirm("install anyway? (identity unverified)", false)?
        {
            println!("    {} skipped", style.dim("ok"));
            return Ok(true);
        }
        if !crate::ui::interactive() {
            // never auto-install an unverified identity
            println!(
                "    {} refusing to install unverified identity non-interactively",
                style.err("✘")
            );
            return Ok(true);
        }
    }

    // Attempt the chosen method; on failure, announce WHY and fall back
    // through the remaining candidates. A security stop (hash mismatch,
    // dangerous installer) never falls through — only ordinary failures do.
    let mut ordered: Vec<&Candidate> = vec![&chosen];
    ordered.extend(
        candidates
            .iter()
            .filter(|c| c.method.label() != chosen.method.label()),
    );

    let mut binary: Option<String> = None;
    let mut verified_path: Option<String> = None;
    let mut used_method: Option<Method> = None;
    let mut failures: Vec<String> = Vec::new();
    let mut library_deadend = false;
    let mut declined = false;

    for (i, candidate) in ordered.iter().enumerate() {
        // after a library dead-end, further cargo/source attempts on the
        // same library will fail identically — skip them
        if library_deadend
            && matches!(
                candidate.method,
                Method::Cargo { .. } | Method::Source { .. }
            )
        {
            continue;
        }
        if i == 0 {
            println!(
                "\n    {} installing via {}",
                style.dim("→"),
                candidate.method.label()
            );
        } else {
            println!(
                "\n    {} falling back to {} {}",
                style.warn("↻"),
                style.bold(&candidate.method.label()),
                style.dim(&format!(
                    "[{}/{}]",
                    candidate.method.rank(),
                    candidate.confidence
                ))
            );
        }
        match attempt_method(app, candidate, &project, spec).await {
            MethodOutcome::Success { binary: b, path } => {
                binary = b;
                verified_path = path;
                used_method = Some(candidate.method.clone());
                break;
            }
            MethodOutcome::Failed { reason, library } => {
                failures.push(format!("{}: {}", candidate.method.label(), reason));
                if library {
                    library_deadend = true;
                    println!(
                        "    {} the crate is a library, not an installable program",
                        style.warn("⚠")
                    );
                } else {
                    println!("    {} {}", style.warn("⚠"), style.dim(&shorten(&reason)));
                }
            }
            MethodOutcome::Fatal(e) => return Err(e),
            MethodOutcome::Declined => {
                println!("    {} skipped", style.dim("ok"));
                declined = true;
                break;
            }
        }
    }

    if used_method.is_none() && !declined {
        // every method under this identity failed as a library → the same
        // name on npm, WITH its own repository link, is a verified identity
        // of a different project (http-server the lib vs the CLI)
        if library_deadend && let Some(bin) = npm_library_fallback(app, spec, &project).await? {
            binary = bin;
            used_method = Some(Method::Npm {
                package: spec.to_string(),
            });
        }
        if used_method.is_none() {
            // announce every failure, not just the first
            println!("\n    {} every install method failed:", style.err("✘"));
            for f in &failures {
                println!("      {} {}", style.err("•"), f);
            }
            return Err(PxError::User(format!(
                "no install method for {spec} succeeded ({} failed)",
                failures.len()
            )));
        }
    }
    if declined {
        return Ok(true);
    }

    let chosen_for_ledger = used_method.unwrap_or_else(|| chosen.method.clone());
    println!(
        "    {} installed via {}",
        style.ok("✔"),
        chosen_for_ledger.label()
    );

    // Record the install with full identity for remove/update.
    if !app.cli.dry_run {
        let mut ledger = crate::ledger::Ledger::load();
        ledger.record_universal(
            spec,
            &project.canonical,
            &chosen_for_ledger,
            binary.as_deref(),
            verified_path.as_deref(),
        );
        ledger.save();
    }
    Ok(true)
}

/// The result of attempting ONE install method.
enum MethodOutcome {
    /// Worked. binary = the name it installed (project's known binary
    /// preferred), path = the verified executable path when verifiable.
    Success {
        binary: Option<String>,
        path: Option<String>,
    },
    /// Failed for an ordinary reason (network, missing asset, empty
    /// package) — the caller may fall back to the next candidate.
    Failed { reason: String, library: bool },
    /// Security stop (hash mismatch, dangerous scan) — never falls through.
    Fatal(PxError),
    /// The user declined a confirmation; stop without installing.
    Declined,
}

/// Shorten an error for the announcement line (first line, capped).
fn shorten(reason: &str) -> String {
    let first = reason.lines().next().unwrap_or(reason);
    if first.len() > 110 {
        format!("{}…", &first[..110])
    } else {
        first.to_string()
    }
}

/// Attempt ONE candidate end to end: pinned-hash check → safety scan →
/// execute → verify the app actually installed. Every failure that isn't a
/// security stop is a `Failed` the caller can fall back from.
async fn attempt_method(
    app: &App,
    candidate: &Candidate,
    project: &Project,
    spec: &str,
) -> MethodOutcome {
    let style = &app.style;

    // scripts: pinned-hash contract, then the static safety scan
    if let Method::Script { url } = &candidate.method {
        // Registry-pinned installers: the content hash is a contract. If
        // the live URL serves different bytes, STOP — never silently
        // execute changed installer content. This is a Fatal, not Failed.
        if let Some(pinned) = &candidate.pinned_sha {
            match app.client.get(url).send().await {
                Ok(resp) if resp.status().is_success() => match resp.bytes().await {
                    Ok(bytes) => {
                        use sha2::{Digest, Sha256};
                        let actual = format!("{:x}", Sha256::digest(&bytes));
                        if actual != *pinned {
                            println!(
                                "\n    {} installer content CHANGED since validation",
                                style.err("✘")
                            );
                            println!("    {} pinned  {pinned}", style.dim("·"));
                            println!("    {} actual  {actual}", style.dim("·"));
                            crate::registry::telemetry::report_failure(
                                &project.canonical,
                                &candidate.method.label(),
                                0,
                                "hash_mismatch",
                                None,
                                url,
                            );
                            return MethodOutcome::Fatal(PxError::User(format!(
                                "installer for {spec} changed since registry validation — refusing"
                            )));
                        }
                        println!(
                            "    {} installer sha256 matches the registry pin",
                            style.ok("✓")
                        );
                    }
                    Err(e) => {
                        return MethodOutcome::Failed {
                            reason: format!("could not read installer: {e}"),
                            library: false,
                        };
                    }
                },
                Ok(resp) => {
                    return MethodOutcome::Failed {
                        reason: format!("installer fetch returned HTTP {}", resp.status()),
                        library: false,
                    };
                }
                Err(e) => {
                    return MethodOutcome::Failed {
                        reason: format!("installer unreachable: {e}"),
                        library: false,
                    };
                }
            }
        }
        // the scan itself: network failure = Failed (fall back); a
        // dangerous verdict = Fatal (never fall through to it)
        match crate::security::scan_remote_script(&app.client, url).await {
            Ok(scan) => match scan.verdict {
                crate::security::Verdict::Dangerous => {
                    println!(
                        "    {} installer failed the safety scan — refusing",
                        style.err("✘")
                    );
                    for f in &scan.flags {
                        println!("      {} {}", style.err("•"), f);
                    }
                    return MethodOutcome::Fatal(PxError::User(format!(
                        "installer for {spec} is unsafe"
                    )));
                }
                crate::security::Verdict::Suspicious => {
                    println!("    {} installer has warning signs:", style.warn("⚠"));
                    for f in &scan.flags {
                        println!("      {} {}", style.warn("•"), f);
                    }
                    if crate::ui::interactive()
                        && !crate::ui::prompt::confirm("run it in the sandbox anyway?", false)
                            .unwrap_or(false)
                    {
                        return MethodOutcome::Declined;
                    }
                }
                crate::security::Verdict::Clean => {
                    println!("    {} safety scan clean", style.ok("✓"));
                }
            },
            Err(e) => {
                return MethodOutcome::Failed {
                    reason: format!("could not fetch installer for scanning: {e}"),
                    library: false,
                };
            }
        }
    }

    // execute
    let method_binary = match exec::execute(app, &candidate.method).await {
        Ok(bin) => bin,
        Err(e) => {
            let msg = format!("{e}");
            let library = matches!(candidate.method, Method::Cargo { .. })
                && msg.contains("nothing to install");
            return MethodOutcome::Failed {
                reason: msg,
                library,
            };
        }
    };

    // the registry knows the real executable name (package ≠ binary); it
    // beats the method's guess (cargo installs `agent`, not `agent-code`)
    let binary = project.binary.clone().or(method_binary);

    // CHECK THE APP ACTUALLY INSTALLED: the binary must exist and run. A
    // method that claims success but leaves no working binary is a Failed —
    // the caller falls back to the next method.
    if let Some(bin) = &binary {
        match exec::verify_binary(bin).await {
            Ok(path) => {
                println!(
                    "    {} binary: {} ({})",
                    style.ok("✔"),
                    style.bold(bin),
                    style.dim(&path)
                );
                return MethodOutcome::Success {
                    binary: Some(bin.clone()),
                    path: Some(path),
                };
            }
            Err(e) => {
                return MethodOutcome::Failed {
                    reason: format!(
                        "method succeeded but the expected binary '{bin}' is not runnable: {e}"
                    ),
                    library: false,
                };
            }
        }
    }
    MethodOutcome::Success { binary, path: None }
}

/// Run an upstream installer script inside the sandbox with a time limit.
/// Never the bare `curl | sh` the docs show — px downloads, scans (caller),
/// then executes with the system read-only and a deadline.
pub async fn exec_script_sandboxed(app: &App, url: &str) -> PxResult<()> {
    let curl = crate::exec::resolve_bin("curl");
    let sh = crate::exec::resolve_bin("sh");
    let script = vec![
        "/bin/sh".to_string(),
        "-c".to_string(),
        format!("{curl} -fsSL {url} | {sh}"),
    ];
    if crate::sandbox::enabled(app) && crate::sandbox::bwrap_available() {
        let out = crate::sandbox::run_sandboxed(app, &script, &[]).await?;
        if out.success() {
            return Ok(());
        }
        // show what failed
        let combined = format!("{}{}", out.stdout, out.stderr);
        if !combined.trim().is_empty() {
            let lines: Vec<&str> = combined.lines().collect();
            let start = lines.len().saturating_sub(15);
            eprintln!("  ── last lines of: {url} ──");
            for line in &lines[start..] {
                eprintln!("  {line}");
            }
        }
        return Err(PxError::User(format!("installer script failed: {url}")));
    }
    // unsandboxed fallback: inherited stdio, kill-on-drop
    crate::ui::prompt::flush();
    let status = tokio::process::Command::new(&script[0])
        .args(&script[1..])
        .kill_on_drop(true)
        .status()
        .await
        .map_err(|e| PxError::User(format!("cannot run installer: {e}")))?;
    if status.success() {
        Ok(())
    } else {
        Err(PxError::User(format!("installer script failed: {url}")))
    }
}

/// After a library dead-end under the crates.io identity: the same name on
/// npm, WITH its own repository link, is a verified identity of a possibly
/// different project (the http-server case). A bare npm name with no repo
/// link stays confidence 20 — never installed silently.
async fn npm_library_fallback(
    app: &App,
    spec: &str,
    original: &Project,
) -> PxResult<Option<Option<String>>> {
    let Some((npm_repo, desc)) = npm_identity(&app.client, spec).await else {
        return Ok(None);
    };
    let Some(repo) = npm_repo else {
        println!(
            "    {} an npm package named '{spec}' exists but has no repository link — identity unverified, not installing",
            app.style.warn("⚠")
        );
        return Ok(None);
    };
    if repo == original.canonical {
        return Ok(None); // same project — already exhausted
    }
    println!(
        "\n    {} {spec} on npm is a different project: {}",
        app.style.ok("✓"),
        app.style.bold(&repo)
    );
    let method = Method::Npm {
        package: spec.to_string(),
    };
    println!(
        "    {} installing via {}",
        app.style.dim("→"),
        method.label()
    );
    let bin = exec::execute(app, &method).await?;
    let binary = bin.clone();
    let project = Project {
        canonical: repo.clone(),
        description: desc,
        binary: binary.clone(),
        evidence: "npm package with a repository link (library namesake fallback)".into(),
    };
    println!(
        "    {} binary: {}",
        app.style.ok("✔"),
        binary.as_deref().unwrap_or("unknown")
    );
    if !app.cli.dry_run {
        let mut ledger = crate::ledger::Ledger::load();
        ledger.record_universal(spec, &project.canonical, &method, binary.as_deref(), None);
        ledger.save();
    }
    Ok(Some(bin))
}
