//! Distro detection: /etc/os-release + command probes → which recipe applies.

use std::collections::BTreeMap;
use std::path::Path;

use crate::recipe::schema::{DetectionRule, Recipe};

#[derive(Debug, Clone, Default)]
pub struct OsRelease {
    pub fields: BTreeMap<String, String>,
}

impl OsRelease {
    pub fn load() -> Self {
        Self::from_file(Path::new("/etc/os-release"))
    }

    pub fn from_file(path: &Path) -> Self {
        let mut fields = BTreeMap::new();
        // Fall back to /usr/lib/os-release like systemd does.
        let content = std::fs::read_to_string(path)
            .or_else(|_| std::fs::read_to_string(Path::new("/usr/lib/os-release")));
        if let Ok(content) = content {
            for line in content.lines() {
                if let Some((k, v)) = line.split_once('=') {
                    let v = v.trim().trim_matches('"').to_string();
                    fields.insert(k.trim().to_string(), v);
                }
            }
        }
        OsRelease { fields }
    }

    pub fn id(&self) -> Option<&str> {
        self.fields.get("ID").map(|s| s.as_str())
    }

    pub fn id_like(&self) -> Vec<String> {
        self.fields
            .get("ID_LIKE")
            .map(|s| s.split_whitespace().map(|w| w.trim().to_string()).collect())
            .unwrap_or_default()
    }

    pub fn pretty(&self) -> String {
        self.fields
            .get("PRETTY_NAME")
            .cloned()
            .unwrap_or_else(|| "unknown".into())
    }
}

fn rule_matches(rule: &DetectionRule, os: &OsRelease) -> bool {
    match rule {
        DetectionRule::OsRelease(m) => {
            if let Some(id) = &m.id
                && os.id() == Some(id.as_str())
            {
                return true;
            }
            if let Some(like) = &m.id_like
                && os.id_like().iter().any(|l| l == like)
            {
                return true;
            }
            false
        }
        DetectionRule::Command(command) => which::which(command).is_ok(),
    }
}

/// Why a recipe matched (for `px doctor` transparency).
#[derive(Debug, Clone, PartialEq)]
pub enum MatchedVia {
    OsReleaseId,
    OsReleaseIdLike,
    Command,
}

#[derive(Debug, Clone)]
pub struct DetectionResult {
    pub recipe_id: String,
    pub via: MatchedVia,
}

/// Evaluate one recipe's rules against this machine.
pub fn recipe_matches(recipe: &Recipe, os: &OsRelease) -> Option<DetectionResult> {
    for rule in &recipe.detection {
        if !rule_matches(rule, os) {
            continue;
        }
        let via = match rule {
            DetectionRule::OsRelease(m) if m.id.is_some() && os.id() == m.id.as_deref() => {
                MatchedVia::OsReleaseId
            }
            DetectionRule::OsRelease(_) => MatchedVia::OsReleaseIdLike,
            DetectionRule::Command(_) => MatchedVia::Command,
        };
        return Some(DetectionResult {
            recipe_id: recipe.meta.id.clone(),
            via,
        });
    }
    None
}

/// Pick the first recipe (in given order) that matches this machine.
pub fn detect<'a>(recipes: &'a [Recipe], os: &OsRelease) -> Option<(&'a Recipe, DetectionResult)> {
    for recipe in recipes {
        if let Some(result) = recipe_matches(recipe, os) {
            return Some((recipe, result));
        }
    }
    None
}
