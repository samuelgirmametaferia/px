//! px's install database: ~/.local/state/px/installed.json.
//! Records not just WHAT was installed but the full identity — project,
//! method, binary name, binary path — so `px remove`/`px update` can route
//! to the right uninstaller even when package name ≠ executable name
//! (agent-code the package installs `agent` the binary).

use std::path::PathBuf;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::universal::Method;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LedgerEntry {
    pub name: String,
    pub source: String,
    pub installed_at: DateTime<Utc>,
    /// What the user typed.
    #[serde(default)]
    pub query: Option<String>,
    /// Canonical project identity ("avala-ai/agent-code").
    #[serde(default)]
    pub project: Option<String>,
    /// The executable it installed (package ≠ binary!).
    #[serde(default)]
    pub binary: Option<String>,
    /// Where the binary landed, when px placed it itself (release/script).
    #[serde(default)]
    pub binary_path: Option<String>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Ledger {
    pub entries: Vec<LedgerEntry>,
}

pub fn ledger_path() -> PathBuf {
    dirs::state_dir()
        .unwrap_or_else(|| {
            dirs::data_dir()
                .unwrap_or_else(|| PathBuf::from(".local/share"))
                .join("state")
        })
        .join("px")
        .join("installed.json")
}

impl Ledger {
    pub fn load() -> Ledger {
        match std::fs::read_to_string(ledger_path()) {
            Ok(text) => serde_json::from_str(&text).unwrap_or_default(),
            Err(_) => Ledger::default(),
        }
    }

    pub fn save(&self) {
        let path = ledger_path();
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(json) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(path, json);
        }
    }

    /// Record a successful install (idempotent per name, refreshed on
    /// reinstall).
    pub fn record(&mut self, name: &str, source: &str) {
        self.record_full(name, source, None, None, None, None);
    }

    pub fn record_full(
        &mut self,
        name: &str,
        source: &str,
        query: Option<String>,
        project: Option<String>,
        binary: Option<String>,
        binary_path: Option<String>,
    ) {
        self.entries.retain(|e| e.name != name);
        self.entries.push(LedgerEntry {
            name: name.to_string(),
            source: source.to_string(),
            installed_at: Utc::now(),
            query,
            project,
            binary,
            binary_path,
        });
    }

    /// Record a universal (project-resolved) install.
    pub fn record_universal(
        &mut self,
        query: &str,
        project: &str,
        method: &Method,
        binary: Option<&str>,
        binary_path: Option<&str>,
    ) {
        let name = match method {
            Method::Cargo { crate_name } => crate_name.clone(),
            Method::Npm { package } => package.clone(),
            Method::Pipx { package } | Method::Gem { gem: package } => package.clone(),
            Method::Go { module } => module.rsplit('/').next().unwrap_or(module).to_string(),
            Method::Brew { tap } => tap.rsplit('/').next().unwrap_or(tap).to_string(),
            Method::Release { repo } | Method::Source { repo } => {
                repo.rsplit('/').next().unwrap_or(repo).to_string()
            }
            Method::Script { .. } | Method::Native => query.to_string(),
        };
        let source = match method {
            Method::Native => "repo",
            Method::Release { .. } => "release",
            Method::Cargo { .. } => "cargo",
            Method::Npm { .. } => "npm",
            Method::Pipx { .. } => "pipx",
            Method::Go { .. } => "go",
            Method::Gem { .. } => "gem",
            Method::Brew { .. } => "brew",
            Method::Script { .. } => "script",
            Method::Source { .. } => "source",
        }
        .to_string();
        self.record_full(
            &name,
            &source,
            Some(query.to_string()),
            Some(project.to_string()),
            binary.map(|b| b.to_string()),
            binary_path.map(|b| b.to_string()),
        );
    }

    /// Find an entry by the user's query, its package name, its project, or
    /// its installed binary — all four are valid handles for remove/update.
    pub fn find(&self, handle: &str) -> Option<&LedgerEntry> {
        let h = handle.to_lowercase();
        self.entries
            .iter()
            .find(|e| {
                e.name.to_lowercase() == h
                    || e.query.as_deref().is_some_and(|q| q.to_lowercase() == h)
                    || e.project.as_deref().is_some_and(|p| p.to_lowercase() == h)
                    || e.binary.as_deref().is_some_and(|b| b.to_lowercase() == h)
            })
            .or_else(|| {
                // prefix/contains fallback for "agent" → "agent-code"
                self.entries.iter().find(|e| {
                    e.name.to_lowercase().contains(&h) || h.contains(&e.name.to_lowercase())
                })
            })
    }
}
