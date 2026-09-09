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

        for file in files {
            let name = file_name(file);
            if name == "package.json" {
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
        // treat named imports as local deps.
        if deps.is_empty() {
            let import_re =
                regex::Regex::new(r#"(?:require\(|from\s+|import\s+)['"]([^'"]+)['"]"#).unwrap();
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
                for caps in import_re.captures_iter(&text) {
                    if let Some(m) = caps.get(1) {
                        let spec = m.as_str();
                        // Only bare module names (not ./relative, not node: builtins).
                        if !spec.starts_with('.')
                            && !spec.starts_with('/')
                            && !spec.starts_with("node:")
                            && !spec.starts_with('@')
                        {
                            let name = spec.split('/').next().unwrap_or(spec);
                            if !deps.contains(&name.to_string()) {
                                deps.push(name.to_string());
                            }
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
