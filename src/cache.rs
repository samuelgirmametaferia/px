//! Disk cache: recipes (handled in recipe::load) + search results.
//! Plain files with mtime-based TTL — no cleverness.

use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use sha2::{Digest, Sha256};

pub fn cache_root() -> PathBuf {
    dirs::cache_dir()
        .unwrap_or_else(|| PathBuf::from(".cache"))
        .join("px")
}

pub fn search_dir() -> PathBuf {
    cache_root().join("search")
}

fn key_path(namespace: &str, key: &str) -> PathBuf {
    let mut hasher = Sha256::new();
    hasher.update(key.as_bytes());
    let digest = hex(&hasher.finalize());
    cache_root().join(namespace).join(format!("{digest}.txt"))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn get(namespace: &str, key: &str, ttl: Duration) -> Option<String> {
    let path = key_path(namespace, key);
    let meta = std::fs::metadata(&path).ok()?;
    let age = SystemTime::now()
        .duration_since(meta.modified().ok()?)
        .ok()?;
    if age > ttl {
        return None;
    }
    std::fs::read_to_string(&path).ok()
}

pub fn put(namespace: &str, key: &str, value: &str) {
    let path = key_path(namespace, key);
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(&path, value);
}

/// Invalidate one cached entry — used after installs/uninstalls so stale
/// `installed: false` answers don't survive the operation that changed them.
pub fn delete(namespace: &str, key: &str) {
    let _ = std::fs::remove_file(key_path(namespace, key));
}

/// Housekeeping: drop entries older than their TTL would allow (called on
/// startup so caches can never grow unbounded).
pub fn purge_expired(max_age: Duration) {
    for ns in ["provider", "search"] {
        let dir = cache_root().join(ns);
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let Ok(meta) = entry.metadata() else {
                continue;
            };
            if let Ok(modified) = meta.modified()
                && SystemTime::now()
                    .duration_since(modified)
                    .map(|age| age > max_age)
                    .unwrap_or(false)
            {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }
}

/// Drop a cache namespace ("search", "recipes", "builds") or everything.
pub fn clean(namespace: Option<&str>) -> std::io::Result<()> {
    match namespace {
        Some(ns) => {
            let dir = cache_root().join(ns);
            if dir.exists() {
                std::fs::remove_dir_all(dir)?;
            }
        }
        None => {
            let root = cache_root();
            if root.exists() {
                std::fs::remove_dir_all(root)?;
            }
        }
    }
    Ok(())
}
