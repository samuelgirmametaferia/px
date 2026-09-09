//! `install for` orchestration: run every matching detector, merge, filter,
//! validate candidates against real sources, present one plan.

pub mod ccpp;
pub mod detector;
pub mod go;
pub mod java;
pub mod localenv;
pub mod node;
pub mod python;
pub mod ruby;
pub mod shell;

use detector::{Detected, Detector, Ecosystem};

pub use detector::looks_like_project_file;

use crate::error::{PxError, PxResult};
use crate::recipe::schema::Recipe;
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub fn all_detectors() -> Vec<Box<dyn Detector>> {
    vec![
        Box::new(python::PythonDetector),
        Box::new(shell::ShellDetector),
        Box::new(ccpp::CCppDetector),
        Box::new(node::NodeDetector),
        Box::new(go::GoDetector),
        Box::new(java::JavaDetector),
        Box::new(ruby::RubyDetector),
    ]
}

/// Everything found in one project, merged across ecosystems.
#[derive(Debug, Default)]
pub struct Analysis {
    pub root: PathBuf,
    pub system: Vec<detector::SystemDep>,
    pub local: Vec<detector::LocalDep>,
    pub tools: Vec<String>,
    pub unmapped: Vec<String>,
    pub ecosystems: Vec<Ecosystem>,
    pub evidence: Vec<String>,
}

/// Analyze a path (file or directory). Pure filesystem work + recipe lookups;
/// validating candidates against sources happens later (validate()).
pub fn analyze(path: &Path, recipe: &Recipe) -> PxResult<Analysis> {
    let root = if path.is_dir() {
        path.to_path_buf()
    } else {
        path.parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."))
    };

    let files: Vec<PathBuf> = if path.is_dir() {
        detector::scan_source_files(path, 400)
    } else {
        vec![path.to_path_buf()]
    };

    if files.is_empty() {
        return Err(PxError::User(format!(
            "no files to analyze under {}",
            path.display()
        )));
    }

    let mut analysis = Analysis {
        root,
        ..Default::default()
    };

    for det in all_detectors() {
        let eco_cfg = recipe.ecosystem(det.ecosystem().id());
        if eco_cfg.map(|c| !c.enabled).unwrap_or(true) {
            continue; // ecosystem disabled in this recipe
        }
        if !det.matches(path, &files) {
            continue;
        }
        let found: Detected = det.analyze(path, &files, recipe)?;
        analysis.system.extend(found.system);
        analysis.local.extend(found.local);
        for t in found.tools {
            if !analysis.tools.contains(&t) {
                analysis.tools.push(t);
            }
        }
        for u in found.unmapped {
            if !analysis.unmapped.contains(&u) {
                analysis.unmapped.push(u);
            }
        }
        analysis.ecosystems.push(det.ecosystem());
        for e in found.evidence {
            if !analysis.evidence.contains(&e) {
                analysis.evidence.push(e);
            }
        }
    }

    if analysis.ecosystems.is_empty() {
        return Err(PxError::User(format!(
            "couldn't identify any project type in {} (supported: python, node, c/c++, shell, go, java, ruby)",
            path.display()
        )));
    }
    Ok(analysis)
}

/// Post-analysis filtering: drop system deps + tools already installed.
///
/// Every candidate check (info + installed) is a subprocess — running them
/// one-by-one is what made early versions crawl on big projects. All checks
/// now run concurrently on the multi-threaded runtime, and results are
/// memoized in a disk cache so repeated runs on the same project are near
/// instant. Under `dry_run` (distro simulation) validation and
/// installed-checks are skipped — the point is to see the plan.
pub async fn filter_installed_and_validate(
    analysis: &mut Analysis,
    providers: &[Arc<dyn crate::backend::Provider>],
    dry_run: bool,
) -> PxResult<()> {
    use std::sync::{Arc, Mutex};

    type CheckCache = Arc<Mutex<std::collections::HashMap<(String, String), bool>>>;

    // Memoize per (provider, name) across deps — several deps can share
    // candidates, and tools overlap with deps constantly.
    let info_cache: CheckCache = Arc::new(Mutex::new(std::collections::HashMap::new()));
    let installed_cache: CheckCache = Arc::new(Mutex::new(std::collections::HashMap::new()));

    async fn info_exists(
        p: &Arc<dyn crate::backend::Provider>,
        name: &str,
        cache: &CheckCache,
    ) -> bool {
        let key = (p.source_id().to_string(), name.to_string());
        if let Some(v) = cache.lock().unwrap().get(&key) {
            return *v;
        }
        let v = p.info(name).await.ok().flatten().is_some();
        cache.lock().unwrap().insert(key, v);
        v
    }

    async fn is_installed(
        p: &Arc<dyn crate::backend::Provider>,
        name: &str,
        cache: &CheckCache,
    ) -> bool {
        let key = (p.source_id().to_string(), name.to_string());
        if let Some(v) = cache.lock().unwrap().get(&key) {
            return *v;
        }
        let v = p.is_installed(name).await.unwrap_or(false);
        cache.lock().unwrap().insert(key, v);
        v
    }

    // Phase 1: resolve all deps concurrently (deps don't depend on each other).
    let mut tasks = Vec::new();
    for dep in analysis.system.drain(..) {
        let providers: Vec<Arc<dyn crate::backend::Provider>> = providers.to_vec();
        let info_cache = Arc::clone(&info_cache);
        let installed_cache = Arc::clone(&installed_cache);
        tasks.push(tokio::spawn(async move {
            let mut dep = dep;
            if !dry_run {
                let mut valid: Vec<String> = Vec::new();
                for cand in &dep.candidates {
                    for p in &providers {
                        if info_exists(p, cand, &info_cache).await {
                            valid.push(cand.clone());
                            break;
                        }
                    }
                }
                if valid.is_empty() {
                    return (
                        None,
                        Some(format!(
                            "{} → no package found (tried {})",
                            dep.import,
                            dep.candidates.join(", ")
                        )),
                    );
                }
                dep.candidates = valid;
            }
            if !dry_run {
                let mut inst = false;
                for p in &providers {
                    if is_installed(p, &dep.candidates[0], &installed_cache).await {
                        inst = true;
                        break;
                    }
                }
                if inst {
                    return (None, None);
                }
            }
            (Some(dep), None)
        }));
    }

    let mut kept: Vec<detector::SystemDep> = Vec::new();
    for t in tasks {
        let (dep, unmapped) = t.await.expect("filter task panicked");
        if let Some(dep) = dep {
            kept.push(dep);
        } else if let Some(u) = unmapped {
            analysis.unmapped.push(u);
        }
    }
    analysis.system = kept;

    // Phase 2: tools, same concurrency.
    let mut tool_tasks = Vec::new();
    for tool in analysis.tools.drain(..) {
        let providers: Vec<Arc<dyn crate::backend::Provider>> = providers.to_vec();
        let installed_cache = Arc::clone(&installed_cache);
        tool_tasks.push(tokio::spawn(async move {
            if dry_run {
                return tool;
            }
            for p in &providers {
                if is_installed(p, &tool, &installed_cache).await {
                    return String::new();
                }
            }
            tool
        }));
    }
    for t in tool_tasks {
        let tool = t.await.expect("tool task panicked");
        if !tool.is_empty() {
            analysis.tools.push(tool);
        }
    }

    // Heuristic same-name shell commands that didn't validate are script
    // words, not packages — drop them silently instead of noising up the
    // "couldn't map" list. (Override-backed proposals still report.)
    analysis
        .unmapped
        .retain(|u| !u.starts_with("command: ") || !u.contains("→ no package found"));

    Ok(())
}
