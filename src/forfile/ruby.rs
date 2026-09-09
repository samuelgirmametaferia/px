//! Ruby detector: gems in a Gemfile are local (bundler's job); system deps
//! are ruby + native-extension build tooling.

use std::path::{Path, PathBuf};

use crate::error::PxResult;
use crate::forfile::detector::{Detected, Detector, Ecosystem, LocalDep, extension, file_name};
use crate::recipe::schema::Recipe;

pub struct RubyDetector;

impl Detector for RubyDetector {
    fn ecosystem(&self) -> Ecosystem {
        Ecosystem::Ruby
    }

    fn matches(&self, _path: &Path, files: &[PathBuf]) -> bool {
        files.iter().any(|f| {
            extension(f) == "rb"
                || matches!(
                    file_name(f).as_str(),
                    "Gemfile" | "Gemfile.lock" | "Rakefile"
                )
                || extension(f) == "gemspec"
        })
    }

    fn analyze(&self, path: &Path, files: &[PathBuf], recipe: &Recipe) -> PxResult<Detected> {
        let eco = recipe.ecosystem("ruby").cloned().unwrap_or_default();
        let mut gems: Vec<String> = Vec::new();
        let mut evidence: Vec<String> = Vec::new();

        let gem_re = regex::Regex::new(r#"(?m)^\s*gem\s+['"]([\w-]+)['"]"#).unwrap();
        let require_re =
            regex::Regex::new(r#"(?m)^\s*require(?:_relative)?\s+['"]([\w/-]+)['"]"#).unwrap();

        for file in files {
            let name = file_name(file);
            let Ok(text) = std::fs::read_to_string(file) else {
                continue;
            };
            if name == "Gemfile" {
                for caps in gem_re.captures_iter(&text) {
                    if let Some(m) = caps.get(1)
                        && !gems.contains(&m.as_str().to_string())
                    {
                        gems.push(m.as_str().to_string());
                    }
                }
                evidence.push("Gemfile".into());
            } else if extension(file) == "rb" {
                for caps in require_re.captures_iter(&text) {
                    if let Some(m) = caps.get(1) {
                        let first = m.as_str().split('/').next().unwrap_or("");
                        if !first.is_empty() && !gems.contains(&first.to_string()) {
                            gems.push(first.to_string());
                        }
                    }
                }
                if evidence.len() < 8 {
                    evidence.push(
                        file.strip_prefix(path)
                            .map(|p| p.to_string_lossy().into_owned())
                            .unwrap_or(name),
                    );
                }
            }
        }

        let mut detected = Detected {
            tools: eco.tools.clone(),
            evidence,
            ..Default::default()
        };
        // Gems resolve via bundler locally. Only recipe overrides (known
        // distro packages) become system deps — and they're validated
        // against real sources before install anyway.
        for gem in gems {
            if let Some(pkg) = eco.overrides.get(&gem) {
                detected.system.push(crate::forfile::detector::SystemDep {
                    import: gem.clone(),
                    candidates: vec![pkg.clone()],
                    ecosystem: Ecosystem::Ruby,
                });
            } else {
                detected.local.push(LocalDep {
                    name: gem,
                    ecosystem: Ecosystem::Ruby,
                });
            }
        }
        if !detected.local.is_empty() {
            for dep in &eco.runtime_deps {
                if !detected.tools.contains(dep) {
                    detected.tools.push(dep.clone());
                }
            }
        }
        Ok(detected)
    }
}
