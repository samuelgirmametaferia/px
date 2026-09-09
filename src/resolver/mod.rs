//! Resolution: one spec → hits from every active source, queried in
//! parallel, merged by priority, with did-you-mean fallback.

pub mod fuzzy;

use crate::backend::{Installer, PackageHit, Provider};
use crate::error::PxError;
use crate::recipe::schema::Recipe;
use std::sync::Arc;

/// What the resolver decided for one spec.
#[derive(Debug)]
pub enum Resolution {
    /// Exact-name hit in the highest-priority source that has it.
    Exact(PackageHit),
    /// No exact hit; ranked candidates from all sources.
    Candidates(Vec<PackageHit>),
    /// Nothing anywhere; close names for did-you-mean.
    NotFound { near_misses: Vec<String> },
}

/// Options for resolution.
#[derive(Debug, Clone, Copy, Default)]
pub struct ResolveOpts {
    /// Distro simulation (--recipe <other distro> + --dry-run): the recipe's
    /// tools aren't installed so searches return nothing; assume the spec
    /// exists in the top source so the plan and its commands can be shown.
    pub assume_exists: bool,
}

/// Query every provider in parallel for one spec.
pub async fn resolve_spec(
    spec: &str,
    providers: &[Arc<dyn Provider>],
    opts: ResolveOpts,
) -> Resolution {
    let mut searches = join_all(providers, spec).await;

    // Exact hit in the highest-priority source wins.
    for (provider, hits) in searches.iter_mut() {
        if let Some(exact) = hits.iter().position(|h| h.name == spec) {
            let mut hit = hits.remove(exact);
            hit.source = provider.source_id().to_string();
            return Resolution::Exact(hit);
        }
    }

    // Otherwise rank every hit by fuzzy score within its source, then
    // interleave by source priority.
    let mut all: Vec<PackageHit> = Vec::new();
    for (provider, hits) in &mut searches {
        for hit in hits.iter_mut() {
            hit.score = fuzzy::score(spec, &hit.name);
            hit.source = provider.source_id().to_string();
        }
        all.append(hits);
    }

    if all.is_empty() {
        // Nothing matched the full spec. Shorter-prefix searches build the
        // did-you-mean pool ("firefxo" → search "firef" → firefox…).
        let mut pool: Vec<PackageHit> = Vec::new();
        for len in [6usize, 5, 4, 3] {
            if spec.len() <= len {
                continue;
            }
            let prefix = &spec[..len];
            for (provider, hits) in join_all(providers, prefix).await {
                for mut hit in hits {
                    hit.source = provider.source_id().to_string();
                    pool.push(hit);
                }
            }
            if !pool.is_empty() {
                break;
            }
        }

        // Distro simulation: assume the spec exists in the top source so the
        // plan and its commands can be inspected on any distro.
        if opts.assume_exists && !providers.is_empty() {
            return Resolution::Exact(PackageHit {
                name: spec.to_string(),
                version: String::new(),
                description: None,
                source: providers[0].source_id().to_string(),
                score: 0,
            });
        }

        let mut names: Vec<String> = pool.iter().map(|h| h.name.clone()).collect();
        names.sort();
        names.dedup();
        let near = fuzzy::near_misses(spec, &names, 5);
        return Resolution::NotFound { near_misses: near };
    }

    all.sort_by_key(|h| std::cmp::Reverse(h.score));
    let best = all[0].score;
    if best < fuzzy::EXACTISH {
        // Nothing even close — treat as not found, keep names for suggestions
        let mut near: Vec<String> = all.iter().map(|h| h.name.clone()).collect();
        near.dedup();
        near.truncate(5);
        return Resolution::NotFound { near_misses: near };
    }

    all.truncate(20);
    Resolution::Candidates(all)
}

/// Tiny local join_all (avoids pulling futures crate just for this).
async fn join_all(
    providers: &[Arc<dyn Provider>],
    spec: &str,
) -> Vec<(Arc<dyn Provider>, Vec<PackageHit>)> {
    let mut handles = Vec::new();
    for p in providers {
        let p = Arc::clone(p);
        let spec = spec.to_string();
        handles.push(tokio::spawn(async move {
            let hits = p.search(&spec).await.unwrap_or_default();
            (p, hits)
        }));
    }
    let mut out = Vec::new();
    for h in handles {
        if let Ok(pair) = h.await {
            out.push(pair);
        }
    }
    out
}

/// Install one resolved hit through its owning provider, if that provider
/// can install. Returns the source id used.
pub async fn install_hit(
    hit: &PackageHit,
    providers: &[Arc<dyn Provider>],
    installers: &[Arc<dyn Installer>],
) -> Result<String, PxError> {
    // Prefer the installer registered for the hit's source; fall back to the
    // first installer (recipes usually define install on the repo source).
    // The provider ordering mirrors the recipe, so this respects priority.
    let idx = providers
        .iter()
        .position(|p| p.source_id() == hit.source)
        .unwrap_or(0);
    if let Some(installer) = installers.get(idx.min(installers.len().saturating_sub(1))) {
        installer
            .install(
                std::slice::from_ref(&hit.name),
                crate::backend::InstallCtx::default(),
            )
            .await?;
        Ok(hit.source.clone())
    } else {
        Err(PxError::User("no installer available".into()))
    }
}

/// Build the display line for a plan item.
pub fn plan_line(style: &crate::ui::style::Style, hit: &PackageHit) -> String {
    let version = if hit.version.is_empty() {
        String::new()
    } else {
        format!(" {}", hit.version)
    };
    format!(
        "{}{}  {}",
        style.by_source(&hit.source, &hit.name),
        style.dim(&version),
        style.dim(&format!("({})", hit.source))
    )
}

/// Check a recipe's github config for whether source builds may run.
pub fn source_enabled(recipe: &Recipe, no_source_flag: bool, cfg: &crate::config::Config) -> bool {
    recipe.github.enabled && cfg.source_enabled && !no_source_flag
}
