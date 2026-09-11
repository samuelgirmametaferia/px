//! The detector contract: one ecosystem, one implementation, zero distro
//! knowledge. Mapping import → package name is the recipe's job.

use std::path::Path;

use crate::error::PxResult;
use crate::recipe::schema::Recipe;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Ecosystem {
    Python,
    Node,
    CCpp,
    Shell,
    Go,
    Java,
    Ruby,
}

impl Ecosystem {
    pub fn id(&self) -> &'static str {
        match self {
            Ecosystem::Python => "python",
            Ecosystem::Node => "node",
            Ecosystem::CCpp => "c",
            Ecosystem::Shell => "shell",
            Ecosystem::Go => "go",
            Ecosystem::Java => "java",
            Ecosystem::Ruby => "ruby",
        }
    }
}

/// A system-level dependency a detector wants installed.
#[derive(Debug, Clone)]
pub struct SystemDep {
    /// What we saw (import name, header, command) — provenance for display.
    pub import: String,
    /// Distro package candidates, ordered best-first. Validated against
    /// real sources before anything is installed.
    pub candidates: Vec<String>,
    pub ecosystem: Ecosystem,
}

/// A language-local dependency (pip/npm/gem module — handled by the
/// language's own tooling under --local).
#[derive(Debug, Clone)]
pub struct LocalDep {
    pub name: String,
    pub ecosystem: Ecosystem,
}

/// Everything one detector found in a project.
#[derive(Debug, Default)]
pub struct Detected {
    pub system: Vec<SystemDep>,
    pub local: Vec<LocalDep>,
    /// Tools (interpreters, compilers) this ecosystem needs, from the recipe.
    pub tools: Vec<String>,
    /// Imports we couldn't map — shown as a footnote, never guessed.
    pub unmapped: Vec<String>,
    /// Files that contributed, for provenance display.
    pub evidence: Vec<String>,
}

/// Cheap file-classification helpers shared by detectors.
pub fn extension(path: &Path) -> String {
    path.extension()
        .map(|e| e.to_string_lossy().into_owned())
        .unwrap_or_default()
}

pub fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|e| e.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// First line of a file, if readable (shebang detection).
pub fn first_line(path: &Path) -> Option<String> {
    std::fs::read_to_string(path)
        .ok()?
        .lines()
        .next()
        .map(|s| s.to_string())
}

/// Does this path look like a project file px can analyze? Used by `px -i`
/// to decide whether typed input is a package name or a project path.
pub fn looks_like_project_file(path: &Path) -> bool {
    let ext = extension(path);
    matches!(
        ext.as_str(),
        "py" | "js"
            | "mjs"
            | "cjs"
            | "jsx"
            | "tsx"
            | "zsh"
            | "ts"
            | "c"
            | "h"
            | "cpp"
            | "hpp"
            | "cc"
            | "go"
            | "java"
            | "rb"
            | "sh"
            | "bash"
    ) || matches!(
        file_name(path).as_str(),
        "package.json"
            | "go.mod"
            | "Gemfile"
            | "pom.xml"
            | "build.gradle"
            | "pyproject.toml"
            | "requirements.txt"
            | "CMakeLists.txt"
            | "Makefile"
            | "meson.build"
    )
}

pub trait Detector: Send + Sync {
    fn ecosystem(&self) -> Ecosystem;
    /// Does this path contain a project of my ecosystem?
    fn matches(&self, path: &Path, files: &[std::path::PathBuf]) -> bool;
    /// Extract dependencies. Pure filesystem analysis — no network, no sudo.
    fn analyze(
        &self,
        path: &Path,
        files: &[std::path::PathBuf],
        recipe: &Recipe,
    ) -> PxResult<Detected>;
}

/// Bound a directory scan: text source files only, max `max` entries,
/// skipping the usual noise directories.
pub fn scan_source_files(root: &Path, max: usize) -> Vec<std::path::PathBuf> {
    use walkdir::WalkDir;
    const SKIP: &[&str] = &[
        "node_modules",
        ".git",
        ".venv",
        "venv",
        "__pycache__",
        "target",
        "build",
        "dist",
        ".cache",
        "vendor",
        ".idea",
        ".vscode",
        "coverage",
    ];
    if max == 0 {
        return Vec::new();
    }
    let mut out = Vec::new();
    for entry in WalkDir::new(root)
        .into_iter()
        .filter_entry(|e| {
            let name = e.file_name().to_string_lossy();
            e.depth() == 0 || !SKIP.contains(&name.as_ref())
        })
        .filter_map(|e| e.ok())
    {
        if !entry.file_type().is_file() {
            continue;
        }
        // Only plausible text files, and skip huge ones.
        if let Ok(meta) = entry.metadata()
            && meta.len() > 1_000_000
        {
            continue;
        }
        out.push(entry.into_path());
        if out.len() >= max {
            break;
        }
    }
    out
}
