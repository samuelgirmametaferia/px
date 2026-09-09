//! px-registry — builder and validator for the px upstream fallback
//! registry.
//!
//!   px-registry synth   --count N --out apps.jsonl     generate synthetic apps
//!   px-registry build   --input apps.jsonl --out dir   build sharded registry
//!   px-registry check   --registry dir --query foo     verify a lookup
//!
//! The builder STREAMS: records are read line-by-line, bucketed into 4096
//! per-index temp files, then each shard is sorted, CBOR-encoded and
//! zstd-compressed independently. Memory stays bounded by the largest
//! single shard (~244 records at 1M apps), never the whole registry.

use std::collections::BTreeMap;
use std::io::{BufRead, BufWriter, Write};

use px::registry::schema::{AppEntry, RegistryMethod, RegistryRecord};
use px::registry::{
    Shard, ShardInfo, alias_key, app_key, decode_shard, encode_shard, normalize_alias, shard_of,
};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let cmd = args.get(1).map(|s| s.as_str()).unwrap_or("help");
    let result = match cmd {
        "synth" => cmd_synth(&args[2..]),
        "build" => cmd_build(&args[2..]),
        "check" => cmd_check(&args[2..]),
        _ => {
            eprintln!("usage: px-registry <synth|build|check> [options]");
            eprintln!("  synth --count N --out apps.jsonl");
            eprintln!("  build --input apps.jsonl --out <dir>");
            eprintln!("  check --registry <dir> --query <name>");
            std::process::exit(2);
        }
    };
    if let Err(e) = result {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

fn flag(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1).cloned())
}

// ------------------------------------------------------------------- synth

fn cmd_synth(args: &[String]) -> Result<(), String> {
    let count: usize = flag(args, "--count")
        .and_then(|v| v.parse().ok())
        .ok_or("--count N required")?;
    let out = flag(args, "--out").ok_or("--out FILE required")?;
    let file = std::fs::File::create(&out).map_err(|e| e.to_string())?;
    let mut w = BufWriter::new(file);
    for i in 0..count {
        let rec = synth_record(i);
        serde_json::to_writer(&mut w, &rec).map_err(|e| e.to_string())?;
        writeln!(w).map_err(|e| e.to_string())?;
    }
    w.flush().map_err(|e| e.to_string())?;
    println!("wrote {count} synthetic records to {out}");
    Ok(())
}

/// Deterministic synthetic record: several alias spellings so alias
/// normalization and collisions are exercised at scale.
fn synth_record(i: usize) -> RegistryRecord {
    let slug = format!("synth-app-{i:07}");
    let canonical = format!("github:synth/{slug}");
    let installer = format!("https://raw.githubusercontent.com/synth/{slug}/main/install.sh");
    let method = RegistryMethod {
        method: "script".into(),
        url: Some(installer.clone()),
        // pin the installer to a deterministic hash of the canonical id
        installer_sha256: Some(format!("{:064x}", i as u128)),
        installer_commit: Some(format!("{:040x}", i as u128)),
        version: Some(format!("1.{}.{}", i % 10, i % 100)),
        release_url: Some(format!(
            "https://github.com/synth/{slug}/releases/tag/v1.0.0"
        )),
        asset_sha256: Some(format!("{:064x}", (i * 7) as u128)),
        crate_name: None,
        package: None,
        tap: None,
        module: None,
        gem: None,
    };
    RegistryRecord {
        canonical_id: canonical.clone(),
        aliases: vec![
            slug.clone(),
            slug.replace('-', "_"),
            format!("{} app", slug), // exercises normalization (spaces → -)
        ],
        repository: format!("https://github.com/synth/{slug}"),
        homepage: None,
        description: format!("synthetic test application #{i}"),
        expected_binaries: vec![format!("synthbin{}", i % 97)],
        install_methods: vec![method],
        identity_confidence: 85,
        security_state: "validated".into(),
        stars: (i * 37) as u64 % 50_000,
        forks: (i * 3) as u64 % 900,
        repo_created_at: Some("2024-01-01T00:00:00Z".into()),
        repo_pushed_at: Some("2026-09-01T00:00:00Z".into()),
        latest_release: Some("v1.0.0".into()),
        latest_release_at: Some("2026-08-01T00:00:00Z".into()),
        release_downloads: (i * 11) as u64 % 40_000,
        archived: i.is_multiple_of(997),
        license: Some("MIT".into()),
        last_validated_at: Some("2026-09-01T00:00:00Z".into()),
        validation_result: Some("pass".into()),
        validation_receipt_hash: Some(format!("{:064x}", (i * 13) as u128)),
    }
}

// ------------------------------------------------------------------- build

fn cmd_build(args: &[String]) -> Result<(), String> {
    let input = flag(args, "--input").ok_or("--input FILE required")?;
    let out = flag(args, "--out").ok_or("--out DIR required")?;

    let tmp = std::path::Path::new(&out).join(".tmp");
    let alias_tmp = tmp.join("alias");
    let app_tmp = tmp.join("app");
    for d in [&alias_tmp, &app_tmp] {
        std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
    }

    // Open 4096 bucket files per index. Streaming: one line in, appends to
    // at most one alias bucket (per alias) and one app bucket.
    eprintln!("streaming records into 4096+4096 buckets…");
    let file = std::fs::File::open(&input).map_err(|e| e.to_string())?;
    let reader = std::io::BufReader::new(file);
    let mut total: u64 = 0;

    // lazily-opened bucket writers (open on first write keeps fd use low)
    let mut alias_buckets: BTreeMap<u16, BufWriter<std::fs::File>> = BTreeMap::new();
    let mut app_buckets: BTreeMap<u16, BufWriter<std::fs::File>> = BTreeMap::new();

    for line in reader.lines() {
        let line = line.map_err(|e| e.to_string())?;
        if line.trim().is_empty() {
            continue;
        }
        let rec: RegistryRecord =
            serde_json::from_str(&line).map_err(|e| format!("bad record: {e}"))?;
        total += 1;

        // app entry
        let akey = app_key(&rec.canonical_id);
        let entry = AppEntry {
            key: akey,
            record: rec.clone(),
        };
        let bucket = app_buckets
            .entry(shard_of(&akey))
            .or_insert_with_key(|shard| bucket_writer(&app_tmp, *shard));
        serde_json::to_writer(&mut *bucket, &entry).map_err(|e| e.to_string())?;
        writeln!(*bucket).map_err(|e| e.to_string())?;

        // alias entries (normalized, deduped per record)
        let mut seen = std::collections::BTreeSet::new();
        for alias in rec
            .aliases
            .iter()
            .map(|a| normalize_alias(a))
            .chain(std::iter::once(normalize_alias(
                rec.canonical_id.rsplit('/').next().unwrap_or(""),
            )))
        {
            if alias.is_empty() || !seen.insert(alias.clone()) {
                continue;
            }
            let k = alias_key(&alias);
            let bucket = alias_buckets
                .entry(shard_of(&k))
                .or_insert_with_key(|shard| bucket_writer(&alias_tmp, *shard));
            let ae = px::registry::AliasEntry {
                key: k,
                app_key: akey,
            };
            serde_json::to_writer(&mut *bucket, &ae).map_err(|e| e.to_string())?;
            writeln!(*bucket).map_err(|e| e.to_string())?;
        }
    }
    for (_, mut w) in alias_buckets.into_iter() {
        w.flush().map_err(|e| e.to_string())?;
    }
    for (_, mut w) in app_buckets.into_iter() {
        w.flush().map_err(|e| e.to_string())?;
    }
    eprintln!("{total} records bucketed");

    // Build shards: each bucket independently sorted + encoded + verified.
    let base = std::path::Path::new(&out);
    std::fs::create_dir_all(base).map_err(|e| e.to_string())?;

    let mut alias_infos = Vec::new();
    let mut app_infos = Vec::new();

    for shard in 0..px::registry::SHARD_COUNT as u16 {
        alias_infos.extend(build_one_shard::<px::registry::AliasEntry>(
            &alias_tmp, base, "alias", shard,
        )?);
        app_infos.extend(build_one_shard::<AppEntry>(&app_tmp, base, "app", shard)?);
    }

    // Root manifest
    let root = px::registry::RegistryRoot {
        version: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0),
        generated_at: chrono::Utc::now().to_rfc3339(),
        schema_version: 1,
        alias_shards: alias_infos,
        app_shards: app_infos,
    };
    let mut cbor = Vec::new();
    ciborium::ser::into_writer(&root, &mut cbor).map_err(|e| e.to_string())?;
    let compressed = zstd::encode_all(cbor.as_slice(), 3).map_err(|e| e.to_string())?;
    std::fs::write(base.join("root.cbor"), &compressed).map_err(|e| e.to_string())?;

    let _ = std::fs::remove_dir_all(&tmp);
    println!(
        "registry built: {total} apps, root.cbor + {} alias shards + {} app shards",
        root.alias_shards.len(),
        root.app_shards.len()
    );
    Ok(())
}

fn bucket_writer(dir: &std::path::Path, shard: u16) -> BufWriter<std::fs::File> {
    let path = dir.join(format!("{shard:03x}.jsonl"));
    let file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .expect("bucket file");
    BufWriter::new(file)
}

/// Sort one bucket by key, encode as a shard, write it, return its info.
fn build_one_shard<T>(
    tmp_dir: &std::path::Path,
    out_dir: &std::path::Path,
    namespace: &str,
    shard: u16,
) -> Result<Option<ShardInfo>, String>
where
    T: serde::de::DeserializeOwned + serde::Serialize + px::registry::Keyed,
{
    let bucket = tmp_dir.join(format!("{shard:03x}.jsonl"));
    if !bucket.exists() {
        return Ok(None); // empty shard — omitted from the root entirely
    }
    let file = std::fs::File::open(&bucket).map_err(|e| e.to_string())?;
    let mut entries: Vec<T> = Vec::new();
    for line in std::io::BufReader::new(file).lines() {
        let line = line.map_err(|e| e.to_string())?;
        if line.trim().is_empty() {
            continue;
        }
        entries.push(serde_json::from_str(&line).map_err(|e| e.to_string())?);
    }
    entries.sort_by(|a, b| a.key().cmp(b.key()));
    let count = entries.len() as u32;
    let shard_data = Shard { entries };
    let bytes = encode_shard(&shard_data).map_err(|e| e.to_string())?;

    // verify the round-trip before publishing (corrupt shards must never
    // enter the registry)
    let decoded: Shard<T> = decode_shard(&bytes).map_err(|e| e.to_string())?;
    if decoded.entries.len() != count as usize {
        return Err(format!("{namespace}/{shard}: round-trip size mismatch"));
    }

    let name = format!("{namespace}-{shard:03x}.cbor.zst");
    std::fs::write(out_dir.join(&name), &bytes).map_err(|e| e.to_string())?;

    // blake3 + sha256 digests
    let mut h = blake3::Hasher::new();
    h.update(&bytes);
    let blake = h.finalize().to_hex().to_string();
    use sha2::{Digest, Sha256};
    let sha = Sha256::digest(&bytes);

    Ok(Some(ShardInfo {
        shard,
        blake3: blake,
        sha256: format!("{sha:x}"),
        url: name,
        entries: count,
    }))
}

// ------------------------------------------------------------------- check

fn cmd_check(args: &[String]) -> Result<(), String> {
    let reg = flag(args, "--registry").ok_or("--registry DIR required")?;
    let query = flag(args, "--query").ok_or("--query NAME required")?;

    let rt = tokio::runtime::Runtime::new().unwrap();
    let client = reqwest::Client::new();
    let source = px::registry::RegistrySource::Dir(reg.into());
    let record = rt
        .block_on(px::registry::lookup(&client, &source, &query))
        .map_err(|e| e.to_string())?;
    match record {
        Some(r) => {
            println!("✓ {} → {}", query, r.canonical_id);
            println!("  description: {}", r.description);
            println!("  binaries:    {:?}", r.expected_binaries);
            println!(
                "  confidence:  {} ({})",
                r.identity_confidence, r.security_state
            );
            println!("  methods:     {}", r.install_methods.len());
            Ok(())
        }
        None => {
            println!("✗ '{query}' not in the registry");
            std::process::exit(1);
        }
    }
}
