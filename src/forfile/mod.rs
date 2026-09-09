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
/// Providers come in priority order; a candidate is "valid" when any
/// source can find it. Under `dry_run` (distro simulation) validation and
/// installed-checks are skipped — the point is to see the plan.
pub async fn filter_installed_and_validate(
    analysis: &mut Analysis,
    providers: &[Arc<dyn crate::backend::Provider>],
    dry_run: bool,
) -> PxResult<()> {
    let mut kept: Vec<detector::SystemDep> = Vec::new();
    for mut dep in analysis.system.drain(..) {
        if !dry_run {
            // Validate candidates: keep only ones some source actually has.
            let mut valid: Vec<String> = Vec::new();
            for cand in &dep.candidates {
                for p in providers {
                    if p.info(cand).await.ok().flatten().is_some() {
                        valid.push(cand.clone());
                        break;
                    }
                }
            }
            if valid.is_empty() {
                analysis.unmapped.push(format!(
                    "{} → no package found (tried {})",
                    dep.import,
                    dep.candidates.join(", ")
                ));
                continue;
            }
            dep.candidates = valid;
        }
        // Installed already?
        let installed = if dry_run {
            false
        } else {
            let mut inst = false;
            for p in providers {
                if p.is_installed(&dep.candidates[0]).await.unwrap_or(false) {
                    inst = true;
                    break;
                }
            }
            inst
        };
        if !installed {
            kept.push(dep);
        }
    }
    analysis.system = kept;

    // Tools: drop installed ones.
    let mut tools: Vec<String> = Vec::new();
    for tool in &analysis.tools {
        let installed = if dry_run {
            false
        } else {
            let mut inst = false;
            for p in providers {
                if p.is_installed(tool).await.unwrap_or(false) {
                    inst = true;
                    break;
                }
            }
            inst
        };
        if !installed {
            tools.push(tool.clone());
        }
    }
    analysis.tools = tools;

    Ok(())
}
