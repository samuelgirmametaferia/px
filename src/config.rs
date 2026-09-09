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
    pub cache_search_secs: u64,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            recipe_repo: "arrow/px".into(),
            recipe_branch: "main".into(),
            default_mode: "ask".into(),
            source_enabled: true,
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
