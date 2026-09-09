//! Registry system tests: keys, normalization, sharding, verification,
//! telemetry scrubbing, ranking. These build small registries with the
//! actual builder binary and look them up through the actual client.

use px::registry::{self, schema::RegistryRecord};

fn record(repo: &str, aliases: &[&str], conf: u32, state: &str) -> RegistryRecord {
    px::registry::schema::RegistryRecord {
        canonical_id: format!("github:{repo}"),
        aliases: aliases.iter().map(|s| s.to_string()).collect(),
        repository: format!("https://github.com/{repo}"),
        homepage: None,
        description: "test".into(),
        expected_binaries: vec!["bin".into()],
        install_methods: vec![px::registry::schema::RegistryMethod {
            method: "script".into(),
            url: Some("https://example.com/install.sh".into()),
            installer_sha256: Some("a".repeat(64)),
            installer_commit: None,
            version: None,
            release_url: None,
            asset_sha256: None,
            crate_name: None,
            package: None,
            tap: None,
            module: None,
            gem: None,
        }],
        identity_confidence: conf,
        security_state: state.into(),
        stars: 0,
        forks: 0,
        repo_created_at: None,
        repo_pushed_at: None,
        latest_release: None,
        latest_release_at: None,
        release_downloads: 0,
        archived: false,
        license: None,
        last_validated_at: None,
        validation_result: None,
        validation_receipt_hash: None,
    }
}

fn write_jsonl(path: &std::path::Path, records: &[RegistryRecord]) {
    let mut s = String::new();
    for r in records {
        s.push_str(&serde_json::to_string(r).unwrap());
        s.push('\n');
    }
    std::fs::write(path, s).unwrap();
}

fn build_registry(records: &[RegistryRecord]) -> std::path::PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    let n = N.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("px-reg-test-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let input = dir.join("apps.jsonl");
    write_jsonl(&input, records);
    let out = std::env::temp_dir().join(format!("px-reg-test-out-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&out);
    let status = std::process::Command::new(env!("CARGO_BIN_EXE_px-registry"))
        .args([
            "build",
            "--input",
            input.to_str().unwrap(),
            "--out",
            out.to_str().unwrap(),
        ])
        .output()
        .expect("builder runs");
    assert!(
        status.status.success(),
        "builder: {}",
        String::from_utf8_lossy(&status.stderr)
    );
    out
}

fn lookup(dir: &std::path::Path, query: &str) -> Option<RegistryRecord> {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let client = reqwest::Client::new();
    let source = registry::RegistrySource::Dir(dir.to_path_buf());
    // fresh cache per lookup so shard verification always runs
    let _ = std::fs::remove_dir_all(registry::cache_dir());
    rt.block_on(registry::lookup(&client, &source, query))
        .expect("lookup must not error")
}

// ------------------------------------------------------------------- keys

#[test]
fn alias_normalization_collapses_forms() {
    assert_eq!(registry::normalize_alias("Avala Agent"), "avala-agent");
    assert_eq!(registry::normalize_alias("avala_agent"), "avala-agent");
    assert_eq!(
        registry::normalize_alias("  Avala   Agent  "),
        "avala-agent"
    );
    assert_eq!(registry::normalize_alias("agent.code!"), "agent-code");
}

#[test]
fn keys_are_domain_separated() {
    let a = registry::app_key("github:o/r");
    let b = registry::alias_key("github:o/r");
    assert_ne!(a, b, "app and alias keys must never collide");
    // deterministic
    assert_eq!(a, registry::app_key("github:o/r"));
}

#[test]
fn shards_span_the_full_range() {
    // shard = first 12 bits: 0 and 4095 must be reachable
    let mut zero = false;
    let mut max = false;
    for i in 0..10_000 {
        let k = registry::app_key(&format!("x{i}"));
        let s = registry::shard_of(&k);
        zero |= s == 0;
        max |= s == registry::SHARD_COUNT as u16 - 1;
    }
    assert!(zero && max, "12-bit shard space must be covered");
}

// --------------------------------------------------------------- lookups

#[test]
fn build_and_lookup_roundtrip() {
    let reg = build_registry(&[
        record(
            "avala-ai/agent-code",
            &["agent-code", "agent"],
            100,
            "validated",
        ),
        record("oven-sh/bun", &["bun", "bunjs"], 95, "validated"),
    ]);
    let hit = lookup(&reg, "agent-code").expect("must resolve");
    assert_eq!(hit.canonical_id, "github:avala-ai/agent-code");
    // alias form (spaces) normalizes to the same record
    let hit2 = lookup(&reg, "BunJS").expect("normalized alias must resolve");
    assert_eq!(hit2.canonical_id, "github:oven-sh/bun");
    // clean miss
    assert!(lookup(&reg, "not-a-real-thing").is_none());
}

#[test]
fn corrupted_shard_is_rejected_not_returned() {
    let reg = build_registry(&[record("a/b", &["ab"], 100, "validated")]);
    // corrupt the EXACT shard that serves the "ab" alias
    let shard = registry::shard_of(&registry::alias_key("ab"));
    let shard_file = reg.join(format!("alias-{shard:03x}.cbor.zst"));
    assert!(shard_file.exists(), "shard {shard} must exist");
    let data = std::fs::read(&shard_file).unwrap();
    let mut corrupted = data.clone();
    let mid = corrupted.len() / 2;
    corrupted[mid] ^= 0xFF;
    std::fs::write(&shard_file, corrupted).unwrap();

    let rt = tokio::runtime::Runtime::new().unwrap();
    let client = reqwest::Client::new();
    let _ = std::fs::remove_dir_all(registry::cache_dir());
    let result = rt.block_on(registry::lookup(
        &client,
        &registry::RegistrySource::Dir(reg.clone()),
        "ab",
    ));
    assert!(
        result.is_err(),
        "a corrupted shard must fail verification, never resolve"
    );
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("FAILED verification")
    );
}

// ----------------------------------------------------------------- ranking

#[test]
fn identity_dominates_popularity_in_ranking() {
    // a high-confidence fresh record beats a 100k-star stale one
    let mut strong = record("good/app", &[], 95, "validated");
    strong.repo_pushed_at = Some("2026-09-01T00:00:00Z".into());
    strong.stars = 5;
    let mut popular = record("viral/app", &[], 70, "validated");
    popular.repo_pushed_at = Some("2020-01-01T00:00:00Z".into());
    popular.stars = 100_000;
    assert!(strong.rank_score() > popular.rank_score());
    // dead records never rank
    let dead = record("dead/app", &[], 100, "dead");
    assert!(dead.is_dead());
}

#[test]
fn dead_records_are_tombstones() {
    let reg = build_registry(&[record("dead/app", &["deadapp"], 100, "dead")]);
    // the record resolves (identity preserved) and reports dead
    let hit = lookup(&reg, "deadapp").expect("identity must survive");
    assert!(hit.is_dead());
}

// --------------------------------------------------------------- telemetry

#[test]
fn telemetry_reports_are_scrubbed() {
    // the queue path is isolated per test via the registry cache dir
    registry::telemetry::report_failure(
        "github:o/r",
        "script:0",
        7,
        "hash_mismatch",
        Some(200),
        "https://secret.example.com/install.sh?token=hunter2",
    );
    let path = registry::telemetry::report_path();
    let content = std::fs::read_to_string(&path).unwrap();
    // the URL itself must NEVER appear — only its hash
    assert!(!content.contains("secret.example"), "URL leaked: {content}");
    assert!(!content.contains("hunter2"), "token leaked: {content}");
    let report: registry::telemetry::FailureReport =
        serde_json::from_str(content.lines().next().unwrap()).unwrap();
    assert_eq!(report.error_class, "hash_mismatch");
    assert_eq!(report.http_status, Some(200));
    assert_eq!(report.url_hash.len(), 64, "blake3 hex");
    assert!(report.timestamp_bucket.len() == 13); // YYYY-MM-DDTHH
    let _ = std::fs::remove_file(&path);
}

// ------------------------------------------------------------- shard codec

#[test]
fn shard_codec_roundtrip_and_binary_search() {
    let mut entries: Vec<registry::AliasEntry> = (0..500)
        .map(|i| {
            let key = registry::alias_key(&format!("app-{i}"));
            registry::AliasEntry {
                key,
                app_key: registry::app_key(&format!("github:o/{i}")),
            }
        })
        .collect();
    entries.sort_by_key(|a| a.key);
    let shard = registry::Shard { entries };
    let bytes = registry::encode_shard(&shard).unwrap();
    // compression should be meaningful
    assert!(bytes.len() < 500 * 96, "shard too large: {}", bytes.len());
    let decoded: registry::Shard<registry::AliasEntry> = registry::decode_shard(&bytes).unwrap();
    assert_eq!(decoded.entries.len(), 500);
    // binary search finds every key
    for e in &decoded.entries {
        let found = registry::shard_find(&decoded, &e.key).unwrap();
        assert_eq!(found.app_key, e.app_key);
    }
    // and no other key
    let absent = registry::alias_key("not-there");
    assert!(registry::shard_find(&decoded, &absent).is_none());
}

#[test]
fn unresolved_reports_never_contain_the_raw_query() {
    let path = px::registry::telemetry::unresolved_path();
    let _ = std::fs::remove_file(&path);
    px::registry::telemetry::report_unresolved("My Super Secret Project Name");
    let content = std::fs::read_to_string(&path).unwrap();
    assert!(
        !content.contains("secret") && !content.contains("Super"),
        "raw query must never be stored: {content}"
    );
    let report: px::registry::telemetry::UnresolvedReport =
        serde_json::from_str(content.lines().next().unwrap()).unwrap();
    assert_eq!(
        report.query_hash.len(),
        64,
        "BLAKE3 hex of the normalized query"
    );
    let _ = std::fs::remove_file(&path);
}

/// A stale cached registry root (registry republished with new shard
/// hashes under the same URL) must not poison lookups: shard verification
/// failure triggers a root refresh and retry instead of erroring or
/// returning nothing. Runs hermetically in a subprocess with an isolated
/// XDG cache — the shared cache is owned by the parallel tests.
#[test]
fn stale_root_triggers_refresh_not_failure() {
    let tmp = std::env::temp_dir().join(format!("px-stale-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    let reg_dir = tmp.join("registry");
    let cache = tmp.join("cache");
    std::fs::create_dir_all(&cache).unwrap();

    let build_in_dir = |description: &str| {
        let mut r = record("stale/app", &["staleapp"], 100, "validated");
        r.description = description.to_string();
        let input = tmp.join("apps.jsonl");
        std::fs::write(&input, serde_json::to_string(&r).unwrap()).unwrap();
        let _ = std::fs::remove_dir_all(&reg_dir);
        let status = std::process::Command::new(env!("CARGO_BIN_EXE_px-registry"))
            .args([
                "build",
                "--input",
                input.to_str().unwrap(),
                "--out",
                reg_dir.to_str().unwrap(),
            ])
            .output()
            .unwrap();
        assert!(status.status.success());
    };

    let check = || {
        std::process::Command::new(env!("CARGO_BIN_EXE_px-registry"))
            .args([
                "check",
                "--registry",
                reg_dir.to_str().unwrap(),
                "--query",
                "staleapp",
            ])
            .env("XDG_CACHE_HOME", &cache)
            .output()
            .unwrap()
    };

    // v1 of the registry; the check caches its root + shards
    build_in_dir("first version");
    let out = check();
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("stale/app"),
        "v1 must resolve"
    );

    // republish with different content (different shard hashes) in place;
    // the cached root from v1 now pins stale hashes
    build_in_dir("second version, republished");
    let out = check();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("stale/app"),
        "stale root must refresh and resolve, got: {stdout}"
    );

    let _ = std::fs::remove_dir_all(&tmp);
}

/// The registry URL is a DEDICATED tag (releases/download/registry-latest),
/// NOT releases/latest — a product release (like px v3) must never steal
/// the registry pointer. This pins the default config against regressions.
#[test]
fn registry_default_url_uses_dedicated_tag() {
    let cfg = px::config::Config::default();
    assert!(
        cfg.upstream_registry.contains("/download/registry-latest"),
        "registry URL must be the dedicated tag, got: {}",
        cfg.upstream_registry
    );
    assert!(
        !cfg.upstream_registry.contains("/latest/download"),
        "releases/latest/download is stolen by product releases — must not be used"
    );
}
