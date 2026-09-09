//! Java detector: JDK + build tools are system deps; Maven/Gradle resolve
//! their own artifacts (stays local, px never touches Maven Central).

use std::path::{Path, PathBuf};

use crate::error::PxResult;
use crate::forfile::detector::{Detected, Detector, Ecosystem, extension, file_name};
use crate::recipe::schema::Recipe;

pub struct JavaDetector;

impl Detector for JavaDetector {
    fn ecosystem(&self) -> Ecosystem {
        Ecosystem::Java
    }

    fn matches(&self, _path: &Path, files: &[PathBuf]) -> bool {
        files.iter().any(|f| {
            extension(f) == "java"
                || matches!(
                    file_name(f).as_str(),
                    "pom.xml" | "build.gradle" | "build.gradle.kts" | "settings.gradle"
                )
        })
    }

    fn analyze(&self, path: &Path, files: &[PathBuf], recipe: &Recipe) -> PxResult<Detected> {
        let eco = recipe.ecosystem("java").cloned().unwrap_or_default();
        let mut evidence: Vec<String> = Vec::new();
        let mut has_maven = false;
        let mut has_gradle = false;

        for file in files {
            match file_name(file).as_str() {
                "pom.xml" => {
                    has_maven = true;
                    evidence.push("pom.xml".into());
                }
                "build.gradle" | "build.gradle.kts" | "settings.gradle" => {
                    has_gradle = true;
                    evidence.push(file_name(file));
                }
                name if extension(file) == "java" && evidence.len() < 8 => {
                    evidence.push(
                        file.strip_prefix(path)
                            .map(|p| p.to_string_lossy().into_owned())
                            .unwrap_or_else(|_| name.to_string()),
                    );
                }
                _ => {}
            }
        }

        let mut detected = Detected {
            tools: eco.tools.clone(),
            evidence,
            ..Default::default()
        };
        if has_maven {
            detected.tools.push("maven".into());
        }
        if has_gradle {
            detected.tools.push("gradle".into());
        }
        Ok(detected)
    }
}
