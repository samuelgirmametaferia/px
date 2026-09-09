//! Registry record schema — mirrors the spec's field list. CBOR-encoded.

use serde::{Deserialize, Serialize};

use super::Keyed;

/// One complete app record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegistryRecord {
    /// "github:avala-ai/agent-code"
    pub canonical_id: String,
    pub aliases: Vec<String>,
    pub repository: String,
    #[serde(default)]
    pub homepage: Option<String>,
    pub description: String,
    /// Executables the install is expected to produce (package ≠ binary).
    pub expected_binaries: Vec<String>,
    pub install_methods: Vec<RegistryMethod>,
    /// 100 curated, 95 official README installer, 90 official docs, 85
    /// canonical-repo installer + strong evidence, 70 probable automated
    /// match. Below 70 px never silently installs. Stars never raise this.
    pub identity_confidence: u32,
    /// "validated" | "unvalidated" | "suspect" | "quarantined" | "dead"
    pub security_state: String,

    // popularity / freshness — tie-breakers ONLY, never identity
    #[serde(default)]
    pub stars: u64,
    #[serde(default)]
    pub forks: u64,
    #[serde(default)]
    pub repo_created_at: Option<String>,
    #[serde(default)]
    pub repo_pushed_at: Option<String>,
    #[serde(default)]
    pub latest_release: Option<String>,
    #[serde(default)]
    pub latest_release_at: Option<String>,
    #[serde(default)]
    pub release_downloads: u64,
    #[serde(default)]
    pub archived: bool,
    #[serde(default)]
    pub license: Option<String>,

    // validation receipt
    #[serde(default)]
    pub last_validated_at: Option<String>,
    #[serde(default)]
    pub validation_result: Option<String>,
    #[serde(default)]
    pub validation_receipt_hash: Option<String>,
}

/// One install method, with pinned content where possible.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegistryMethod {
    /// "script" | "release" | "cargo" | "npm" | "pipx" | "go" | "gem" | "brew" | "source"
    pub method: String,
    /// installer / release URL
    #[serde(default)]
    pub url: Option<String>,
    /// PINNED installer content hash — if the URL serves different bytes,
    /// px STOPS the install. Mutable URLs without a pin score lower.
    #[serde(default)]
    pub installer_sha256: Option<String>,
    #[serde(default)]
    pub installer_commit: Option<String>,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub release_url: Option<String>,
    #[serde(default)]
    pub asset_sha256: Option<String>,
    /// crate/package/tap identifiers for non-URL methods
    #[serde(default)]
    pub crate_name: Option<String>,
    #[serde(default)]
    pub package: Option<String>,
    #[serde(default)]
    pub tap: Option<String>,
    #[serde(default)]
    pub module: Option<String>,
    #[serde(default)]
    pub gem: Option<String>,
}

/// App-shard entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppEntry {
    pub key: [u8; 32],
    pub record: RegistryRecord,
}

impl Keyed for AppEntry {
    fn key(&self) -> &[u8; 32] {
        &self.key
    }
}

/// Dead records keep their identity as a tombstone — an attacker creating
/// a new project with an abandoned name must not hijack resolution.
impl RegistryRecord {
    pub fn is_dead(&self) -> bool {
        self.security_state == "dead"
    }

    /// Ranking per the spec: identity confidence FIRST, security state
    /// SECOND, freshness LAST. Popularity is only ever a tie-breaker.
    pub fn rank_score(&self) -> (u32, u32, u32) {
        let security = match self.security_state.as_str() {
            "validated" => 3,
            "unvalidated" => 2,
            "suspect" | "quarantined" => 1,
            _ => 0, // dead — filtered before ranking
        };
        let freshness = self
            .repo_pushed_at
            .as_deref()
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
            .map(|t| chrono::Utc::now().signed_duration_since(t).num_days() as u32)
            .unwrap_or(u32::MAX);
        (self.identity_confidence, security, u32::MAX - freshness)
    }
}
