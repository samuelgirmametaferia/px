//! Broken-URL / failure telemetry: privacy-scrubbed, batched, async.
//! The normal install path NEVER touches the network for telemetry; only
//! failures enqueue a report, and reports are only sent when an endpoint
//! is configured. Payloads contain NO usernames, paths, commands, or IPs —
//! record id, method id, registry version, error class, HTTP status, a
//! HASH of the URL, and a coarse time bucket.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FailureReport {
    pub record_id: String, // canonical id from the registry record
    pub method_id: String, // e.g. "script:0"
    pub registry_version: u64,
    pub error_class: String, // "http_404" | "hash_mismatch" | "tls" | ...
    pub http_status: Option<u16>,
    pub url_hash: String,         // BLAKE3 of the URL — never the URL itself
    pub timestamp_bucket: String, // hour-precision, e.g. "2026-09-09T14"
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnresolvedReport {
    /// BLAKE3 of the normalized query — the raw query is NEVER stored or
    /// sent. Demand-driven discovery counts hashes; nothing identifies
    /// the user.
    pub query_hash: String,
    pub timestamp_bucket: String,
}

pub fn report_path() -> PathBuf {
    crate::registry::cache_dir().join("reports.jsonl")
}

pub fn unresolved_path() -> PathBuf {
    crate::registry::cache_dir().join("unresolved.jsonl")
}

/// Enqueue a failure report locally. Never blocks, never fails the install.
pub fn enqueue(report: FailureReport) {
    let path = report_path();
    let _ = std::fs::create_dir_all(path.parent().unwrap_or(&PathBuf::from(".")));
    // cap the queue at 500 entries so it can't grow unbounded offline
    if let Ok(existing) = std::fs::read_to_string(&path) {
        let lines = existing.lines().count();
        if lines > 500 {
            let _ = std::fs::remove_file(&path);
        }
    }
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    {
        use std::io::Write;
        if let Ok(json) = serde_json::to_string(&report) {
            let _ = writeln!(f, "{json}");
        }
    }
}

/// Report an unresolved query (px couldn't find the thing anywhere).
/// Hashed locally; only the hash ever leaves the machine, and only when an
/// endpoint is configured.
pub fn report_unresolved(query: &str) {
    let norm = crate::registry::normalize_alias(query);
    let mut h = blake3::Hasher::new();
    h.update(norm.as_bytes());
    let report = UnresolvedReport {
        query_hash: h.finalize().to_hex().to_string(),
        timestamp_bucket: chrono::Utc::now().format("%Y-%m-%dT%H").to_string(),
    };
    let path = unresolved_path();
    let _ = std::fs::create_dir_all(path.parent().unwrap_or(&PathBuf::from(".")));
    if let Ok(existing) = std::fs::read_to_string(&path)
        && existing.lines().count() > 500
    {
        let _ = std::fs::remove_file(&path);
    }
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    {
        use std::io::Write;
        if let Ok(json) = serde_json::to_string(&report) {
            let _ = writeln!(f, "{json}");
        }
    }
}

/// Build a report for an installer failure.
pub fn report_failure(
    record_id: &str,
    method_id: &str,
    registry_version: u64,
    error_class: &str,
    http_status: Option<u16>,
    url: &str,
) {
    let mut h = blake3::Hasher::new();
    h.update(url.as_bytes());
    enqueue(FailureReport {
        record_id: record_id.to_string(),
        method_id: method_id.to_string(),
        registry_version,
        error_class: error_class.to_string(),
        http_status,
        url_hash: h.finalize().to_hex().to_string(),
        timestamp_bucket: chrono::Utc::now().format("%Y-%m-%dT%H").to_string(),
    });
}

/// Flush queued reports to the configured endpoint (batched POST).
/// No endpoint configured → nothing is sent, ever.
pub async fn flush(client: &reqwest::Client, endpoint: &str) -> PxResultFlush {
    if endpoint.trim().is_empty() {
        return PxResultFlush::NoEndpoint;
    }
    let path = report_path();
    let Ok(existing) = std::fs::read_to_string(&path) else {
        return PxResultFlush::Nothing;
    };
    let reports: Vec<FailureReport> = existing
        .lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect();
    if reports.is_empty() {
        return PxResultFlush::Nothing;
    }
    #[derive(Serialize)]
    struct Batch {
        reports: Vec<FailureReport>,
    }
    let n = reports.len();
    let resp = client
        .post(endpoint)
        .json(&Batch { reports })
        .timeout(std::time::Duration::from_secs(5))
        .send()
        .await;
    // unresolved queries ride along to the sibling endpoint (best effort)
    let unresolved_file = unresolved_path();
    if let Ok(existing) = std::fs::read_to_string(&unresolved_file) {
        let queries: Vec<UnresolvedReport> = existing
            .lines()
            .filter_map(|l| serde_json::from_str(l).ok())
            .collect();
        if !queries.is_empty() {
            #[derive(Serialize)]
            struct UnresolvedBatch {
                queries: Vec<UnresolvedReport>,
            }
            let base = endpoint.trim_end_matches("/v1/report");
            let _ = client
                .post(format!("{base}/v1/unresolved"))
                .json(&UnresolvedBatch { queries })
                .timeout(std::time::Duration::from_secs(5))
                .send()
                .await
                .map(|r| r.status().is_success())
                .map(|ok| {
                    if ok {
                        let _ = std::fs::remove_file(&unresolved_file);
                    }
                });
        }
    }
    match resp {
        Ok(r) if r.status().is_success() => {
            // sent reports are consumed; failures stay queued for retry
            let _ = std::fs::remove_file(&path);
            PxResultFlush::Sent(n)
        }
        _ => PxResultFlush::Failed, // keep the queue, retry next time
    }
}

#[derive(Debug, PartialEq)]
pub enum PxResultFlush {
    NoEndpoint,
    Nothing,
    Sent(usize),
    Failed,
}
