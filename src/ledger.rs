//! px's own install ledger: ~/.local/state/px/installed.json — powers `px list`.

use std::path::PathBuf;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LedgerEntry {
    pub name: String,
    pub source: String,
    pub installed_at: DateTime<Utc>,
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

    /// Record a successful install (idempotent per name).
    pub fn record(&mut self, name: &str, source: &str) {
        if !self.entries.iter().any(|e| e.name == name) {
            self.entries.push(LedgerEntry {
                name: name.to_string(),
                source: source.to_string(),
                installed_at: Utc::now(),
            });
        }
    }
}
