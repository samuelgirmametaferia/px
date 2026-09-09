//! ~/.config/px/config.toml — user preferences.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::recipe::load::RecipeRepo;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// owner/repo recipes are fetched from (raw.githubusercontent.com).
    pub recipe_repo: String,
    pub recipe_branch: String,
    /// install-for default: "global" | "local" | "ask"
    pub default_mode: String,
    /// Disable GitHub source-build fallback globally.
    pub source_enabled: bool,
    /// Sandbox mode: "auto" (on when bwrap exists), "on", "off".
    pub sandbox: String,
    /// Upstream fallback registry: a directory or base URL. Empty = off.
    pub upstream_registry: String,
    /// Failure-telemetry endpoint (POST /v1/report). Empty = never send.
    pub telemetry_endpoint: String,
    /// Bearer token for the telemetry endpoint.
    pub telemetry_token: String,
    pub cache_search_secs: u64,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            recipe_repo: "samuelgirmametaferia/px".into(),
            recipe_branch: "main".into(),
            default_mode: "ask".into(),
            source_enabled: true,
            sandbox: "auto".into(),
            upstream_registry:
                "https://github.com/samuelgirmametaferia/px/releases/latest/download".into(),
            telemetry_endpoint: String::new(),
            telemetry_token: String::new(),
            cache_search_secs: 3600,
        }
    }
}

pub fn config_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from(".config"))
        .join("px")
        .join("config.toml")
}

impl Config {
    pub fn load() -> Config {
        match std::fs::read_to_string(config_path()) {
            Ok(text) => toml::from_str(&text).unwrap_or_else(|e| {
                tracing::warn!("config parse error, using defaults: {e}");
                Config::default()
            }),
            Err(_) => Config::default(),
        }
    }

    pub fn recipe_repo(&self) -> RecipeRepo {
        RecipeRepo {
            repo: self.recipe_repo.clone(),
            branch: self.recipe_branch.clone(),
        }
    }
}
