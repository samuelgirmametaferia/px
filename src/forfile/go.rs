//! Go detector: go.mod modules are local (the Go toolchain fetches them);
//! system deps are the toolchain + CGO needs.

use std::path::{Path, PathBuf};

use crate::error::PxResult;
use crate::forfile::detector::{Detected, Detector, Ecosystem, LocalDep, extension, file_name};
use crate::recipe::schema::Recipe;

pub struct GoDetector;

impl Detector for GoDetector {
    fn ecosystem(&self) -> Ecosystem {
        Ecosystem::Go
    }

    fn matches(&self, _path: &Path, files: &[PathBuf]) -> bool {
        files
            .iter()
            .any(|f| file_name(f) == "go.mod" || extension(f) == "go")
    }

    fn analyze(&self, path: &Path, files: &[PathBuf], recipe: &Recipe) -> PxResult<Detected> {
        let eco = recipe.ecosystem("go").cloned().unwrap_or_default();
        let mut modules: Vec<String> = Vec::new();
        let mut evidence: Vec<String> = Vec::new();
        let mut uses_cgo = false;

        let import_re = regex::Regex::new(r#"^\s*_?\s*"([\w./-]+)"\s*$"#).unwrap();

        for file in files {
            let name = file_name(file);
            if name == "go.mod" {
                let Ok(text) = std::fs::read_to_string(file) else {
                    continue;
                };
                let mut in_require = false;
                for line in text.lines() {
                    let t = line.trim();
                    if t.starts_with("require (") {
                        in_require = true;
                        continue;
                    }
                    if in_require && t == ")" {
                        in_require = false;
                        continue;
                    }
                    if in_require || t.starts_with("require ") {
                        let module = t
                            .trim_start_matches("require")
                            .split_whitespace()
                            .next()
                            .unwrap_or("");
                        if !module.is_empty()
                            && !module.starts_with("//")
                            && !modules.contains(&module.to_string())
                        {
                            modules.push(module.to_string());
                        }
                    }
                }
                evidence.push(name);
            } else if extension(file) == "go" {
                let Ok(text) = std::fs::read_to_string(file) else {
                    continue;
                };
                if text.contains("import \"C\"") || text.contains("import (\n\t\"C\"") {
                    uses_cgo = true;
                }
                // Standard-ish detection: net and os/user imply CGO defaults.
                if text.contains("\"net\"") || text.contains("\"os/user\"") {
                    uses_cgo = true;
                }
                for line in text.lines() {
                    if let Some(caps) = import_re.captures(line)
                        && let Some(m) = caps.get(1)
                    {
                        let module = m.as_str();
                        if !module.contains('.') && !module.contains('/') {
                            continue; // stdlib
                        }
                        if !modules.contains(&module.to_string()) {
                            modules.push(module.to_string());
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
        if uses_cgo {
            for dep in &eco.runtime_deps {
                if !detected.tools.contains(dep) {
                    detected.tools.push(dep.clone());
                }
            }
        }
        for module in modules {
            detected.local.push(LocalDep {
                name: module,
                ecosystem: Ecosystem::Go,
            });
        }
        Ok(detected)
    }
}
