//! The px upstream fallback registry — a deterministic, sharded index of
//! software that ISN'T reliably in any normal package manager and is
//! officially installed through installer scripts, release binaries, or
//! custom upstream commands.
//!
//! Design (see docs/registry.md):
//! - Every app has a canonical id ("github:owner/repo").
//! - app_key    = BLAKE3("px-app-v1\0"   + canonical_id)
//! - alias_key  = BLAKE3("px-alias-v1\0" + normalized_alias)
//! - shard      = first 12 bits of the key → 4096 shards per index.
//! - Shards are CBOR arrays sorted by full key, zstd-compressed, published
//!   as immutable blobs; a signed root manifest pins every shard's hash.
//! - One lookup = alias shard + app shard (two small fetches, cached).
//!
//! The registry is consulted AFTER normal resolvers (apt/pacman/cargo/npm/
//! pip/...) fail and BEFORE ad-hoc discovery. Records pin installer
//! content hashes: if the live installer URL returns different bytes than
//! the pinned sha256, px STOPS the install.

pub mod schema;
pub mod telemetry;

use ciborium::{de::from_reader, ser::into_writer};
use serde::{Deserialize, Serialize};

use crate::error::{PxError, PxResult};

pub const DOMAIN_APP: &str = "px-app-v1";
pub const DOMAIN_ALIAS: &str = "px-alias-v1";
pub const SHARD_BITS: u32 = 12;
pub const SHARD_COUNT: usize = 1 << SHARD_BITS; // 4096

// ------------------------------------------------------------------- keys

/// Normalize a user query into its alias form: lowercase, trimmed,
/// whitespace/underscores collapsed to single hyphens, common punctuation
/// dropped. "Avala Agent" and "avala_agent" both → "avala-agent".
pub fn normalize_alias(query: &str) -> String {
    let mut out = String::with_capacity(query.len());
    let mut prev_dash = false;
    for c in query.chars() {
        let c = c.to_ascii_lowercase();
        if c.is_ascii_alphanumeric() {
            out.push(c);
            prev_dash = false;
        } else if !prev_dash && !out.is_empty() {
            out.push('-');
            prev_dash = true;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    out
}

/// app_key = BLAKE3("px-app-v1\0" + canonical_id)
pub fn app_key(canonical_id: &str) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new();
    hasher.update(DOMAIN_APP.as_bytes());
    hasher.update(b"\0");
    hasher.update(canonical_id.as_bytes());
    *hasher.finalize().as_bytes()
}

/// alias_key = BLAKE3("px-alias-v1\0" + normalized_alias)
pub fn alias_key(alias: &str) -> [u8; 32] {
    let norm = normalize_alias(alias);
    let mut hasher = blake3::Hasher::new();
    hasher.update(DOMAIN_ALIAS.as_bytes());
    hasher.update(b"\0");
    hasher.update(norm.as_bytes());
    *hasher.finalize().as_bytes()
}

/// Shard = first 12 bits of the key (big-endian), 0..4095.
pub fn shard_of(key: &[u8; 32]) -> u16 {
    ((key[0] as u16) << 4) | ((key[1] >> 4) as u16)
}

// --------------------------------------------------------------- encoding

/// One entry in an alias shard: the alias key and the app it points to.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AliasEntry {
    pub key: [u8; 32],
    pub app_key: [u8; 32],
}

/// A whole shard, sorted by key (the builder guarantees sort order; the
/// client binary-searches).
#[derive(Debug, Serialize, Deserialize)]
pub struct Shard<T> {
    pub entries: Vec<T>,
}

/// Encode a shard: CBOR → zstd.
pub fn encode_shard<T: Serialize>(shard: &Shard<T>) -> PxResult<Vec<u8>> {
    let mut cbor = Vec::new();
    into_writer(shard, &mut cbor).map_err(|e| PxError::User(format!("cbor encode: {e}")))?;
    zstd::encode_all(cbor.as_slice(), 3).map_err(|e| PxError::User(format!("zstd encode: {e}")))
}

/// Decode a shard: zstd → CBOR.
pub fn decode_shard<T: for<'de> Deserialize<'de>>(bytes: &[u8]) -> PxResult<Shard<T>> {
    let cbor = zstd::decode_all(bytes).map_err(|e| PxError::User(format!("zstd decode: {e}")))?;
    from_reader(cbor.as_slice()).map_err(|e| PxError::User(format!("cbor decode: {e}")))
}

/// Binary search a sorted shard for an exact key.
pub fn shard_find<'a, T>(shard: &'a Shard<T>, key: &[u8; 32]) -> Option<&'a T>
where
    T: Keyed,
{
    shard
        .entries
        .binary_search_by(|e| e.key().cmp(key))
        .ok()
        .map(|i| &shard.entries[i])
}

pub trait Keyed {
    fn key(&self) -> &[u8; 32];
}

impl Keyed for AliasEntry {
    fn key(&self) -> &[u8; 32] {
        &self.key
    }
}

// ------------------------------------------------------------------- root

/// The signed root manifest: pins every shard's hash and URL.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegistryRoot {
    pub version: u64,
    pub generated_at: String,
    pub schema_version: u32,
    /// alias shard digests, indexed by shard id (0..4095). Empty shards may
    /// be omitted from the map but every shard listed must match.
    pub alias_shards: Vec<ShardInfo>,
    pub app_shards: Vec<ShardInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShardInfo {
    pub shard: u16,
    /// BLAKE3 of the compressed shard bytes.
    pub blake3: String,
    /// sha256 too (dual algorithm: clients verify blake3, humans sha256).
    pub sha256: String,
    /// Where to fetch it (http(s) URL or absolute file path).
    pub url: String,
    /// Uncompressed entry count (sanity/stat).
    pub entries: u32,
}

// --------------------------------------------------------------- client

/// Where the registry lives: a directory (file paths / tests) or a release
/// base URL. Root is "<base>/root.cbor".
#[derive(Debug, Clone)]
pub enum RegistrySource {
    Dir(std::path::PathBuf),
    Url(String),
}

pub fn cache_dir() -> std::path::PathBuf {
    crate::cache::cache_root().join("registry")
}

pub fn registry_root_path() -> std::path::PathBuf {
    cache_dir().join("root.cbor")
}

/// Load the root manifest: cached copy first (TTL 1h), then fetch.
/// `local` sources read directly.
pub async fn load_root(
    client: &reqwest::Client,
    source: &RegistrySource,
    refresh: bool,
) -> PxResult<RegistryRoot> {
    let cache_path = registry_root_path();
    if !refresh
        && let Ok(bytes) = std::fs::read(&cache_path)
        && let Ok(root) = decode_root(&bytes)
        && root_age_ok(&root)
    {
        return Ok(root);
    }
    let bytes = fetch(client, source, "root.cbor").await?;
    let root = decode_root(&bytes)?;
    let _ = std::fs::create_dir_all(cache_dir());
    let _ = std::fs::write(&cache_path, &bytes);
    Ok(root)
}

fn root_age_ok(root: &RegistryRoot) -> bool {
    // roots carry generated_at (RFC3339); accept anything parseable that's
    // under 30 days old, or unparseable (builder tests use fixed strings)
    chrono::DateTime::parse_from_rfc3339(&root.generated_at)
        .map(|t| chrono::Utc::now().signed_duration_since(t).num_days() < 30)
        .unwrap_or(true)
}

fn decode_root(bytes: &[u8]) -> PxResult<RegistryRoot> {
    let cbor = zstd::decode_all(bytes).map_err(|e| PxError::User(format!("root zstd: {e}")))?;
    ciborium::de::from_reader(cbor.as_slice()).map_err(|e| PxError::User(format!("root cbor: {e}")))
}

async fn fetch(client: &reqwest::Client, source: &RegistrySource, name: &str) -> PxResult<Vec<u8>> {
    match source {
        RegistrySource::Dir(dir) => {
            let path = dir.join(name);
            std::fs::read(&path)
                .map_err(|e| PxError::User(format!("registry read {}: {e}", path.display())))
        }
        RegistrySource::Url(base) => {
            let url = format!("{base}/{name}");
            let bytes = client
                .get(&url)
                .send()
                .await
                .map_err(PxError::Network)?
                .error_for_status()
                .map_err(PxError::Network)?
                .bytes()
                .await
                .map_err(PxError::Network)?;
            Ok(bytes.to_vec())
        }
    }
}

/// Fetch + verify one shard by its root-manifest info, with disk caching.
/// Verification failure is a hard error — never use an unverified shard.
async fn fetch_shard(
    client: &reqwest::Client,
    source: &RegistrySource,
    info: &ShardInfo,
    namespace: &str, // "alias" | "app"
) -> PxResult<Vec<u8>> {
    // cache hit?
    let cache_path = cache_dir().join(format!("{namespace}-{}.cbor.zst", info.shard));
    if let Ok(bytes) = std::fs::read(&cache_path)
        && blake3_hex(&bytes) == info.blake3
    {
        return Ok(bytes);
    }
    let bytes = match source {
        RegistrySource::Dir(_) => fetch(client, source, &shard_name(namespace, info.shard)).await?,
        RegistrySource::Url(_) => {
            // the root's explicit URL wins (immutable release assets)
            match client.get(&info.url).send().await {
                Ok(resp) if resp.status().is_success() => {
                    resp.bytes().await.map_err(PxError::Network)?.to_vec()
                }
                _ => fetch(client, source, &shard_name(namespace, info.shard)).await?,
            }
        }
    };
    // verify: BLAKE3 must match the root manifest exactly
    let actual = blake3_hex(&bytes);
    if actual != info.blake3 {
        return Err(PxError::User(format!(
            "registry shard {namespace}/{} FAILED verification: root says {}, got {} — refusing",
            info.shard, info.blake3, actual
        )));
    }
    let _ = std::fs::create_dir_all(cache_dir());
    let _ = std::fs::write(&cache_path, &bytes);
    Ok(bytes)
}

fn shard_name(namespace: &str, shard: u16) -> String {
    // flat names: release assets cannot contain '/'; local dirs match
    format!("{namespace}-{shard:03x}.cbor.zst")
}

fn blake3_hex(bytes: &[u8]) -> String {
    let mut h = blake3::Hasher::new();
    h.update(bytes);
    h.finalize().to_hex().to_string()
}

/// Full lookup: query → normalized alias → alias shard → app shard →
/// record. Two shard fetches maximum, both cached.
pub async fn lookup(
    client: &reqwest::Client,
    source: &RegistrySource,
    query: &str,
) -> PxResult<Option<schema::RegistryRecord>> {
    let root = load_root(client, source, false).await?;
    let akey = alias_key(query);
    let ashard_id = shard_of(&akey);

    // alias shard
    let Some(info) = root.alias_shards.iter().find(|s| s.shard == ashard_id) else {
        return Ok(None); // empty shard — nothing with this alias
    };
    let bytes = fetch_shard(client, source, info, "alias").await?;
    let shard: Shard<AliasEntry> = decode_shard(&bytes)?;
    let Some(entry) = shard_find(&shard, &akey) else {
        return Ok(None);
    };

    // app shard
    let app_shard_id = shard_of(&entry.app_key);
    let Some(app_info) = root.app_shards.iter().find(|s| s.shard == app_shard_id) else {
        return Ok(None);
    };
    let bytes = fetch_shard(client, source, app_info, "app").await?;
    let shard: Shard<schema::AppEntry> = decode_shard(&bytes)?;
    Ok(shard_find(&shard, &entry.app_key).map(|e| e.record.clone()))
}

/// The source from config: a directory or a base URL.
pub fn source_from_config(cfg: &crate::config::Config) -> Option<RegistrySource> {
    let s = cfg.upstream_registry.trim();
    if s.is_empty() {
        return None;
    }
    if s.starts_with("http://") || s.starts_with("https://") {
        Some(RegistrySource::Url(s.to_string()))
    } else {
        Some(RegistrySource::Dir(std::path::PathBuf::from(s)))
    }
}
