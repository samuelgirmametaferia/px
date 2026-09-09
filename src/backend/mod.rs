//! Provider/Installer traits and the generic recipe-driven source.

pub mod elevate;
pub mod parsers;
pub mod source;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::error::PxResult;
use crate::recipe::schema::SourceDef;

/// A search hit from any source.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageHit {
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub description: Option<String>,
    /// Recipe source id ("repo", "aur", "copr", "ppa", "github").
    pub source: String,
    /// Fuzzy score, populated by the resolver (higher = better).
    #[serde(default)]
    pub score: i64,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct InstallCtx {
    pub assume_yes: bool,
    pub dry_run: bool,
}

/// Search side of a source.
#[async_trait]
pub trait Provider: Send + Sync {
    fn source_id(&self) -> &str;
    fn label(&self) -> &str;
    async fn search(&self, query: &str) -> PxResult<Vec<PackageHit>>;
    async fn info(&self, exact: &str) -> PxResult<Option<PackageHit>>;
    /// Whether the exact package is already installed on this machine.
    async fn is_installed(&self, name: &str) -> PxResult<bool>;
}

/// Install side of a source.
#[async_trait]
pub trait Installer: Send + Sync {
    async fn install(&self, pkgs: &[String], ctx: InstallCtx) -> PxResult<()>;
}

/// An active source: a recipe [[sources]] entry whose `require_any` matched
/// a binary on this machine (helper = which binary).
#[derive(Debug, Clone)]
pub struct ActiveSource {
    pub def: SourceDef,
    /// The matched binary name ("pacman", "paru", "yay", ...).
    pub helper: String,
}

/// Which sources are usable on this machine, in recipe priority order.
/// Sources whose tools are missing are returned as names so px can say
/// "AUR is available if you install paru or yay" instead of failing silently.
///
/// `force` (set when the user passed --recipe explicitly) activates every
/// source regardless of installed tools — that's the distro-simulation
/// lever: `px --recipe recipes/debian.toml --dry-run install ffmpeg` works
/// on an Arch machine. The helper falls back to the first `require_any`
/// entry so `{helper}` still expands.
pub fn activate_sources(defs: &[SourceDef]) -> (Vec<ActiveSource>, Vec<String>) {
    activate_sources_opt(defs, false)
}

pub fn activate_sources_opt(defs: &[SourceDef], force: bool) -> (Vec<ActiveSource>, Vec<String>) {
    let mut active = Vec::new();
    let mut inactive = Vec::new();
    for def in defs {
        if def.require_any.is_empty() {
            active.push(ActiveSource {
                def: def.clone(),
                helper: String::new(),
            });
            continue;
        }
        match def.require_any.iter().find(|b| which::which(b).is_ok()) {
            Some(helper) => active.push(ActiveSource {
                def: def.clone(),
                helper: helper.clone(),
            }),
            None if force => active.push(ActiveSource {
                def: def.clone(),
                helper: def.require_any[0].clone(),
            }),
            None => inactive.push(def.id.clone()),
        }
    }
    (active, inactive)
}
