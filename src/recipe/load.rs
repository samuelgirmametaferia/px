//! Recipe loading: explicit path → local cache (TTL) → fetch from the px
//! GitHub repo → bundled (embedded) fallback. `--refresh` forces a re-fetch.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use crate::error::{PxError, PxResult};
use crate::recipe::schema::Recipe;

/// Recipes shipped in the px repo, embedded at compile time so px works
/// before its first network call to itself.
pub mod bundled {
    pub const ARCH: &str = include_str!("../../recipes/arch.toml");
    pub const DEBIAN: &str = include_str!("../../recipes/debian.toml");
    pub const FEDORA: &str = include_str!("../../recipes/fedora.toml");
    pub const OPENSUSE: &str = include_str!("../../recipes/opensuse.toml");

    pub fn all() -> Vec<(&'static str, &'static str)> {
        vec![
            ("arch", ARCH),
            ("debian", DEBIAN),
            ("fedora", FEDORA),
            ("opensuse", OPENSUSE),
        ]
    }
}

pub const RECIPE_TTL: Duration = Duration::from_secs(24 * 60 * 60);

/// Where a loaded recipe came from (drives `px recipe show` / `doctor`).
#[derive(Debug, Clone, PartialEq)]
pub enum RecipeSource {
    Explicit(PathBuf),
    Cached { age: Duration },
    Fetched,
    Bundled,
}

pub struct LoadedRecipe {
    pub recipe: Arc<Recipe>,
    pub source: RecipeSource,
}

#[derive(Debug, Clone)]
pub struct RecipeRepo {
    /// owner/repo on GitHub that recipes are fetched from.
    pub repo: String,
    /// Branch/ref to fetch from.
    pub branch: String,
}

impl Default for RecipeRepo {
    fn default() -> Self {
        RecipeRepo {
            repo: "arrow/px".into(),
            branch: "main".into(),
        }
    }
}

pub fn cache_dir() -> PathBuf {
    dirs::cache_dir()
        .unwrap_or_else(|| PathBuf::from(".cache"))
        .join("px")
        .join("recipes")
}

fn cache_path(id: &str) -> PathBuf {
    cache_dir().join(format!("{id}.toml"))
}

fn cached_age(id: &str) -> Option<Duration> {
    let path = cache_path(id);
    let meta = std::fs::metadata(&path).ok()?;
    let modified = meta.modified().ok()?;
    SystemTime::now().duration_since(modified).ok()
}

pub fn read_cached(id: &str) -> Option<(Recipe, Duration)> {
    let age = cached_age(id)?;
    let recipe = Recipe::load_file(&cache_path(id)).ok()?;
    Some((recipe, age))
}

pub fn write_cache(id: &str, toml_text: &str) -> PxResult<()> {
    let dir = cache_dir();
    std::fs::create_dir_all(&dir)
        .map_err(|e| PxError::Recipe(format!("cannot create cache dir: {e}")))?;
    std::fs::write(cache_path(id), toml_text)
        .map_err(|e| PxError::Recipe(format!("cannot write cache: {e}")))?;
    Ok(())
}

pub fn fetch_url(repo: &RecipeRepo, id: &str) -> String {
    format!(
        "https://raw.githubusercontent.com/{}/{}/recipes/{id}.toml",
        repo.repo, repo.branch
    )
}

pub async fn fetch_remote(
    client: &reqwest::Client,
    repo: &RecipeRepo,
    id: &str,
) -> PxResult<String> {
    let url = fetch_url(repo, id);
    let resp = client.get(&url).send().await.map_err(PxError::Network)?;
    if !resp.status().is_success() {
        return Err(PxError::Recipe(format!(
            "fetch {url} returned {}",
            resp.status()
        )));
    }
    Ok(resp.text().await?)
}

/// Full load chain. `explicit` wins; then cache if fresh; then network
/// (unless `offline`); then the embedded copy. Never fails while the
/// embedded copy parses.
pub async fn load(
    explicit: Option<&Path>,
    id: &str,
    repo: &RecipeRepo,
    client: &reqwest::Client,
    refresh: bool,
    offline: bool,
) -> PxResult<LoadedRecipe> {
    // 1. explicit --recipe path
    if let Some(path) = explicit {
        let recipe = Recipe::load_file(path).map_err(PxError::Recipe)?;
        return Ok(LoadedRecipe {
            recipe: Arc::new(recipe),
            source: RecipeSource::Explicit(path.to_path_buf()),
        });
    }

    // 2. fresh cache
    if !refresh
        && let Some((recipe, age)) = read_cached(id)
        && age < RECIPE_TTL
    {
        return Ok(LoadedRecipe {
            recipe: Arc::new(recipe),
            source: RecipeSource::Cached { age },
        });
    }

    // 3. network
    if !offline {
        match fetch_remote(client, repo, id).await {
            Ok(text) => {
                let recipe = Recipe::parse_str(&text).map_err(PxError::Recipe)?;
                let _ = write_cache(id, &text);
                return Ok(LoadedRecipe {
                    recipe: Arc::new(recipe),
                    source: RecipeSource::Fetched,
                });
            }
            Err(e) => {
                tracing::debug!("recipe fetch failed, falling back: {e}");
            }
        }
    }

    // 4. stale cache is better than nothing
    if let Some((recipe, age)) = read_cached(id) {
        return Ok(LoadedRecipe {
            recipe: Arc::new(recipe),
            source: RecipeSource::Cached { age },
        });
    }

    // 5. embedded
    if let Some((_, text)) = bundled::all().into_iter().find(|(bid, _)| *bid == id) {
        let recipe = Recipe::parse_str(text).map_err(PxError::Recipe)?;
        return Ok(LoadedRecipe {
            recipe: Arc::new(recipe),
            source: RecipeSource::Bundled,
        });
    }

    Err(PxError::Recipe(format!(
        "no recipe '{id}' found (tried cache, {}/{}, bundled)",
        repo.repo, repo.branch
    )))
}

/// All bundled recipes parsed, for detection and `px recipe list`.
pub fn bundled_recipes() -> Vec<Recipe> {
    bundled::all()
        .into_iter()
        .filter_map(|(_, text)| Recipe::parse_str(text).ok())
        .collect()
}
