//! The recipe schema: the entire distro contract as data.
//!
//! A recipe tells px how to drive the package managers that already exist on
//! a machine. px never implements package management itself — every
//! interaction is an argv defined here, run against an installed tool.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

pub const SCHEMA_VERSION: u32 = 1;

// ------------------------------------------------------------------ meta

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Meta {
    pub id: String,
    pub name: String,
    pub version: u32,
}

// ------------------------------------------------------------- detection

/// One detection rule. The first matching rule wins; rules are evaluated in
/// file order. TOML shape:
///   os_release = { id = "arch" }          # /etc/os-release ID
///   os_release = { id_like = "arch" }     # ID_LIKE contains this
///   command = "pacman"                    # binary on PATH
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DetectionRule {
    OsRelease(IdMatch),
    Command(String),
}

/// Matches either `id` (exact) or `id_like` (contains) of /etc/os-release.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IdMatch {
    pub id: Option<String>,
    pub id_like: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Detection {
    #[serde(default, rename = "detection")]
    pub rules: Vec<DetectionRule>,
}

// ---------------------------------------------------------------- sources

/// A source wraps an existing package-management tool. Sources are ordered:
/// earlier = higher priority. A source is only active when one of its
/// `require_any` binaries exists on PATH.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceDef {
    pub id: String,
    /// Binaries that activate this source; first found becomes `{helper}`.
    #[serde(default)]
    pub require_any: Vec<String>,
    /// Human label shown in output ("official repos", "AUR", ...).
    pub label: String,
    pub search: Option<CommandDef>,
    pub info: Option<CommandDef>,
    /// Exit 0 + stdout = installed list; exit 1 = not installed.
    pub installed: Option<CommandDef>,
    pub install: Option<CommandDef>,
    /// Which package provides a file path (header → package mapping).
    pub provides: Option<CommandDef>,
}

/// One command: argv with placeholders + how to parse its output.
///
/// Placeholders (replaced as separate argv entries, never string-spliced):
///   {pkg}      — the single package name
///   {pkgs...}  — expands to N entries
///   {helper}   — the matched binary from `require_any`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandDef {
    pub argv: Vec<String>,
    /// Names a parser registered in `backend::parsers`.
    #[serde(default)]
    pub parse: Option<String>,
    /// Whether this command needs sudo; px preflights elevation once
    /// before starting if any planned command is elevated.
    #[serde(default)]
    pub elevated: bool,
}

// ---------------------------------------------------------------- github

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GithubConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_min_stars")]
    pub min_stars: u64,
    #[serde(default = "default_strategies")]
    pub strategies: Vec<String>,
}

fn default_true() -> bool {
    true
}
fn default_min_stars() -> u64 {
    5
}
fn default_strategies() -> Vec<String> {
    ["cargo", "go", "cmake", "make", "meson"]
        .iter()
        .map(|s| s.to_string())
        .collect()
}

impl Default for GithubConfig {
    fn default() -> Self {
        GithubConfig {
            enabled: true,
            min_stars: default_min_stars(),
            strategies: default_strategies(),
        }
    }
}

// ------------------------------------------------------------- ecosystems

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EcoConfig {
    #[serde(default)]
    pub enabled: bool,
    /// Package-name template: `python-{name}` (arch) / `python3-{name}` (deb).
    #[serde(default)]
    pub prefix: Option<String>,
    /// import/header/command → package, beats the prefix rule.
    #[serde(default)]
    pub overrides: BTreeMap<String, String>,
    /// Ecosystem runtime packages themselves (python, nodejs, gcc, ...).
    #[serde(default)]
    pub tools: Vec<String>,
    /// C/C++: header dir → package, longest-prefix match.
    #[serde(default)]
    pub header_map: BTreeMap<String, String>,
    /// Packages needed to build things in this ecosystem (base-devel, ...).
    #[serde(default)]
    pub runtime_deps: Vec<String>,
}

// ----------------------------------------------------------------- recipe

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Recipe {
    pub meta: Meta,
    #[serde(default)]
    pub detection: Vec<DetectionRule>,
    #[serde(default)]
    pub sources: Vec<SourceDef>,
    #[serde(default)]
    pub github: GithubConfig,
    #[serde(default)]
    pub ecosystems: BTreeMap<String, EcoConfig>,
}

impl Recipe {
    pub fn parse_str(s: &str) -> Result<Self, String> {
        let recipe: Recipe = toml::from_str(s).map_err(|e| format!("recipe parse error: {e}"))?;
        if recipe.meta.version != SCHEMA_VERSION {
            return Err(format!(
                "recipe '{}' uses schema version {} but px supports {}",
                recipe.meta.id, recipe.meta.version, SCHEMA_VERSION
            ));
        }
        if recipe.sources.is_empty() {
            return Err(format!(
                "recipe '{}' declares no [[sources]]",
                recipe.meta.id
            ));
        }
        Ok(recipe)
    }

    pub fn load_file(path: &Path) -> Result<Self, String> {
        let s = std::fs::read_to_string(path)
            .map_err(|e| format!("cannot read recipe {}: {e}", path.display()))?;
        Self::parse_str(&s)
    }

    /// Source ids this recipe declares, in priority order.
    pub fn source_ids(&self) -> Vec<String> {
        self.sources.iter().map(|s| s.id.clone()).collect()
    }

    pub fn ecosystem(&self, id: &str) -> Option<&EcoConfig> {
        self.ecosystems.get(id)
    }
}
