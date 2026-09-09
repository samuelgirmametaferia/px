//! The curated app registry (apps/registry.toml) — the strongest identity
//! signal px has. Entries pin the canonical project, its binary name, and
//! its officially documented install methods.

use serde::Deserialize;

use super::Method;

#[derive(Debug, Clone, Deserialize)]
pub struct AppMethod {
    pub method: String,
    /// TOML key `crate` (keyword in Rust).
    #[serde(default, rename = "crate")]
    pub crate_: Option<String>,
    #[serde(default)]
    pub package: Option<String>,
    #[serde(default)]
    pub tap: Option<String>,
    #[serde(default)]
    pub module: Option<String>,
    #[serde(default)]
    pub gem: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub repo: Option<String>,
    #[serde(default)]
    pub confidence: u32,
    #[serde(default)]
    pub note: String,
}

impl AppMethod {
    pub fn to_method(&self) -> Method {
        match self.method.as_str() {
            "cargo" => Method::Cargo {
                crate_name: self.crate_.clone().unwrap_or_default(),
            },
            "npm" => Method::Npm {
                package: self.package.clone().unwrap_or_default(),
            },
            "brew" => Method::Brew {
                tap: self.tap.clone().unwrap_or_default(),
            },
            "pipx" => Method::Pipx {
                package: self.package.clone().unwrap_or_default(),
            },
            "go" => Method::Go {
                module: self.module.clone().unwrap_or_default(),
            },
            "gem" => Method::Gem {
                gem: self.gem.clone().unwrap_or_default(),
            },
            "script" => Method::Script {
                url: self.url.clone().unwrap_or_default(),
            },
            "release" => Method::Release {
                repo: self.repo.clone().unwrap_or_default(),
            },
            other => {
                tracing::warn!("unknown registry method '{other}'");
                Method::Source {
                    repo: self.repo.clone().unwrap_or_default(),
                }
            }
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct AppEntry {
    /// What users type for this app.
    pub query: Vec<String>,
    /// Canonical "owner/repo" identity.
    pub project: String,
    pub description: String,
    /// The executable it installs (package ≠ binary).
    #[serde(default)]
    pub binary: Option<String>,
    pub methods: Vec<AppMethod>,
}

#[derive(Debug, Deserialize)]
struct Registry {
    #[serde(default, rename = "app")]
    apps: Vec<AppEntry>,
}

pub const REGISTRY: &str = include_str!("../../apps/registry.toml");

fn parse() -> Vec<AppEntry> {
    let reg: Registry = toml::from_str(REGISTRY).unwrap_or_else(|e| {
        tracing::error!("app registry parse error: {e}");
        Registry { apps: Vec::new() }
    });
    reg.apps
}

/// Look up what the user typed against the registry's query aliases.
pub fn lookup(spec: &str) -> Option<AppEntry> {
    let spec_lc = spec.to_lowercase();
    parse()
        .into_iter()
        .find(|a| a.query.iter().any(|q| q.to_lowercase() == spec_lc))
}

/// All entries (tests, doctor).
pub fn entries() -> Vec<AppEntry> {
    parse()
}
