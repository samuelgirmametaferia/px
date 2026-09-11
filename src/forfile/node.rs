//! Node detector: npm deps are LOCAL dependencies (installed by npm itself);
//! system deps only for the toolchain and native builds.

use std::path::{Path, PathBuf};

use crate::error::PxResult;
use crate::forfile::detector::{Detected, Detector, Ecosystem, LocalDep, extension, file_name};
use crate::recipe::schema::Recipe;

pub struct NodeDetector;

impl Detector for NodeDetector {
    fn ecosystem(&self) -> Ecosystem {
        Ecosystem::Node
    }

    fn matches(&self, _path: &Path, files: &[PathBuf]) -> bool {
        files.iter().any(|f| {
            file_name(f) == "package.json"
                || matches!(
                    extension(f).as_str(),
                    "js" | "ts" | "mjs" | "cjs" | "jsx" | "tsx"
                )
        })
    }

    fn analyze(&self, path: &Path, files: &[PathBuf], recipe: &Recipe) -> PxResult<Detected> {
        let eco = recipe.ecosystem("node").cloned().unwrap_or_default();
        let mut deps: Vec<String> = Vec::new();
        let mut evidence: Vec<String> = Vec::new();
        let mut native_build = false;
        let mut has_manifest = false;

        for file in files {
            let name = file_name(file);
            if name == "package.json" {
                has_manifest = true;
                let Ok(text) = std::fs::read_to_string(file) else {
                    continue;
                };
                if let Ok(doc) = serde_json::from_str::<serde_json::Value>(&text) {
                    for key in ["dependencies", "devDependencies", "optionalDependencies"] {
                        if let Some(map) = doc.get(key).and_then(|v| v.as_object()) {
                            for dep in map.keys() {
                                if !deps.contains(dep) {
                                    deps.push(dep.clone());
                                }
                            }
                        }
                    }
                }
                evidence.push(name);
            } else if name == "binding.gyp" {
                // Native module → needs a toolchain to build.
                native_build = true;
            }
        }

        // No package.json? Scan imports for evidence of usage but keep it
        // light — without a manifest we can't know versions, so we only
        // treat named imports as local deps. Minified/bundled files are
        // skipped: their regex-matched "imports" are code fragments, not
        // dependencies (the scores_mfe.js garbage: "),L,," and friends).
        if !has_manifest {
            let import_re = regex::Regex::new(
                r#"(?:require\s*\(|from\s+|import\s*\(|import\s+)\s*['"]([^'"]+)['"]"#,
            )
            .unwrap();
            for file in files {
                if !matches!(
                    extension(file).as_str(),
                    "js" | "ts" | "mjs" | "cjs" | "jsx" | "tsx"
                ) {
                    continue;
                }
                let Ok(text) = std::fs::read_to_string(file) else {
                    continue;
                };
                if looks_minified(&text) {
                    tracing::debug!("skipping minified/bundled file: {}", file_name(file));
                    continue;
                }
                for caps in import_re.captures_iter(&text) {
                    if let Some(m) = caps.get(1) {
                        let spec = m.as_str();
                        // only bare module names: no relative, absolute,
                        // builtin, or code-fragment garbage
                        if !is_bare_module_name(spec) {
                            continue;
                        }
                        let name = if spec.starts_with('@') {
                            spec.split('/').take(2).collect::<Vec<_>>().join("/")
                        } else {
                            spec.split('/').next().unwrap_or(spec).to_string()
                        };
                        if !deps.contains(&name) {
                            deps.push(name);
                        }
                    }
                }
                if evidence.len() < 8 {
                    evidence.push(
                        file.strip_prefix(path)
                            .map(|p| p.to_string_lossy().into_owned())
                            .unwrap_or(file_name(file)),
                    );
                }
            }
        }

        let mut detected = Detected {
            tools: eco.tools.clone(),
            evidence,
            ..Default::default()
        };

        // npm packages stay local — mapping them to distro packages is noise.
        for dep in deps {
            detected.local.push(LocalDep {
                name: dep,
                ecosystem: Ecosystem::Node,
            });
        }

        if native_build {
            for dep in &eco.runtime_deps {
                if !detected.tools.contains(dep) {
                    detected.tools.push(dep.clone());
                }
            }
        }

        Ok(detected)
    }
}

/// Minified/bundled JS: long lines, few newlines, huge files. Import
/// scanning them yields code fragments, not dependencies.
fn looks_minified(text: &str) -> bool {
    // > 100KB with an average line > 500 chars is unambiguously built output
    let lines = text.lines().count().max(1);
    let avg_line = text.len() / lines;
    (text.len() > 100_000 && avg_line > 500)
        // or the first line alone is enormous (single-line bundles)
        || text.lines().next().is_some_and(|l| l.len() > 5_000)
}

/// A bare npm module name: optional @scope, then name — letters, digits,
/// `.`, `_`, `-`, `/` only. Rejects relative paths, node: builtins, and
/// minified code fragments (parens, commas, $, createElement, ...).
fn is_bare_module_name(spec: &str) -> bool {
    if spec.is_empty()
        || spec.starts_with('.')
        || spec.starts_with('/')
        || spec.starts_with("node:")
        || spec.starts_with("http:")
        || spec.starts_with("https:")
    {
        return false;
    }
    // @scope/name or name — nothing else
    let pattern = if spec.starts_with('@') {
        r"^@[a-zA-Z0-9][a-zA-Z0-9._-]*/[a-zA-Z0-9][a-zA-Z0-9._-]*(?:/[a-zA-Z0-9._-]+)*$"
    } else {
        r"^[a-zA-Z0-9][a-zA-Z0-9._-]*(?:/[a-zA-Z0-9._-]+)*$"
    };
    regex::Regex::new(pattern)
        .map(|re| re.is_match(spec))
        .unwrap_or(false)
}
