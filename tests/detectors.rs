//! Detector tests against the committed fixtures. These run offline — no
//! pacman, no network — because detectors are pure filesystem analysis.

use px::recipe::schema::Recipe;

fn recipe() -> Recipe {
    Recipe::parse_str(px::recipe::load::bundled::ARCH).expect("bundled arch recipe parses")
}

fn analyze(eco_id: &str, path: &str) -> px::forfile::detector::Detected {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(path);
    let recipe = recipe();
    let det = px::forfile::all_detectors()
        .into_iter()
        .find(|d| d.ecosystem().id() == eco_id)
        .unwrap_or_else(|| panic!("no detector for {eco_id}"));
    let files = px::forfile::detector::scan_source_files(&root, 400);
    assert!(!files.is_empty(), "no files scanned in {path}");
    det.analyze(&root, &files, &recipe).expect("analysis")
}

#[test]
fn python_fixture() {
    let d = analyze("python", "fixtures/python");
    let imports: Vec<&str> = d.system.iter().map(|s| s.import.as_str()).collect();
    assert!(imports.contains(&"numpy"), "imports: {imports:?}");
    assert!(imports.contains(&"cv2"), "imports: {imports:?}");
    // cv2 maps via override
    let cv2 = d.system.iter().find(|s| s.import == "cv2").unwrap();
    assert_eq!(cv2.candidates[0], "python-opencv");
    // numpy candidates include both prefixed and bare names
    let numpy = d.system.iter().find(|s| s.import == "numpy").unwrap();
    assert_eq!(numpy.candidates[0], "python-numpy");
    assert!(numpy.candidates.contains(&"numpy".to_string()));
    // stdlib filtered out
    assert!(!imports.contains(&"os"));
    assert!(!imports.contains(&"json"));
    // local deps present
    assert!(d.local.iter().any(|l| l.name == "flask"));
}

#[test]
fn c_fixture_finds_headers() {
    let d = analyze("c", "fixtures/c");
    let imports: Vec<&str> = d.system.iter().map(|s| s.import.as_str()).collect();
    assert!(imports.contains(&"openssl.h"), "system deps: {imports:?}");
    assert!(imports.contains(&"curl.h"), "system deps: {imports:?}");
    assert!(imports.contains(&"zlib.h"), "system deps: {imports:?}");
    assert!(imports.contains(&"sqlite3.h"), "system deps: {imports:?}");
    // stdio/stdlib never proposed
    assert!(!imports.contains(&"stdio.h"));
    // mapped to the right packages
    let ssl = d.system.iter().find(|s| s.import == "openssl.h").unwrap();
    assert_eq!(ssl.candidates[0], "openssl");
    let sqlite = d.system.iter().find(|s| s.import == "sqlite3.h").unwrap();
    assert_eq!(sqlite.candidates[0], "sqlite");
    // curl maps to curl, NOT libcurlpp (no reverse prefix matching)
    let curl = d.system.iter().find(|s| s.import == "curl.h").unwrap();
    assert_eq!(curl.candidates[0], "curl");
}

#[test]
fn shell_fixture_finds_commands() {
    let d = analyze("shell", "fixtures/shell");
    let imports: Vec<&str> = d.system.iter().map(|s| s.import.as_str()).collect();
    // deploy.sh uses ffmpeg, jq, inotifywait (override), curl (denylisted)
    assert!(
        imports.iter().any(|i| i.contains("ffmpeg")),
        "system deps: {imports:?}"
    );
    assert!(
        imports.iter().any(|i| i.contains("jq")),
        "system deps: {imports:?}"
    );
    assert!(
        imports.iter().any(|i| i.contains("inotifywait")),
        "system deps: {imports:?}"
    );
}

/// Regression for px 0.1.0, which proposed Windows_NT, CDPATH, fetched,
/// check_download, ... as packages from a real-world launcher script.
#[test]
fn shell_fixture_produces_no_garbage() {
    let d = analyze("shell", "fixtures/shell");
    let proposed: Vec<String> = d
        .system
        .iter()
        .map(|s| s.import.trim_start_matches("command: ").to_string())
        .collect();
    for garbage in [
        "Windows_NT",
        "CDPATH",
        "CYGWIN",
        "MINGW",
        "MSYS",
        "Linux",
        "Darwin",
        "cached",
        "fetched",
        "refusing",
        "sandboxes",
        "asset",
        "sidecar_ok",
        "skips",
        "fall",
        "home_bin",
        "exe",
        "amd64",
        "aarch64",
        "expected",
        "check_download",
        "fetch_url",
        "probe_ok",
        "impeccable",
        "machine",
        "probe",
        "none",
        "yes",
    ] {
        assert!(
            !proposed.iter().any(|p| p == garbage),
            "{garbage} must never be proposed (got {proposed:?})"
        );
    }
    // and nothing that ISN'T a real signal leaks: only the deploy.sh
    // commands (ffmpeg, jq, inotifywait) plus shebang/override entries
    for p in &proposed {
        assert!(
            p == "ffmpeg" || p == "jq" || p == "inotifywait" || p == "bash",
            "unexpected proposal: {p}"
        );
    }
}

#[test]
fn ruby_fixture_finds_gems() {
    let d = analyze("ruby", "fixtures/ruby");
    assert!(
        d.local.iter().any(|l| l.name == "rails"),
        "local: {:?}",
        d.local
    );
    assert!(d.local.iter().any(|l| l.name == "puma"));
}

#[test]
fn node_fixture_keeps_deps_local() {
    let d = analyze("node", "fixtures/node");
    assert!(d.local.iter().any(|l| l.name == "express"));
    assert!(d.local.iter().any(|l| l.name == "lodash"));
    // npm deps never become system packages
    assert!(d.system.is_empty(), "system: {:?}", d.system);
}

#[test]
fn go_fixture_finds_modules() {
    let d = analyze("go", "fixtures/go");
    assert!(
        d.local.iter().any(|l| l.name == "github.com/spf13/cobra"),
        "local: {:?}",
        d.local
    );
}

#[test]
fn java_fixture_finds_build_tool() {
    let d = analyze("java", "fixtures/java");
    assert!(d.tools.iter().any(|t| t == "maven"));
    assert!(d.tools.iter().any(|t| t == "jdk-openjdk"));
}

#[test]
fn node_imports_preserve_package_identity() {
    let d = analyze("node", "fixtures/regressions/node");
    let mut names: Vec<_> = d.local.iter().map(|dep| dep.name.as_str()).collect();
    names.sort();
    assert_eq!(names, ["@scope/package", "chalk", "http-server", "lodash"]);
}

#[test]
fn empty_node_manifest_is_authoritative() {
    assert!(
        analyze("node", "fixtures/regressions/node-empty")
            .local
            .is_empty()
    );
}

#[test]
fn typing_extensions_is_a_third_party_dependency() {
    let d = analyze("python", "fixtures/regressions/python");
    assert!(d.system.iter().any(|dep| dep.import == "typing_extensions"));
    assert!(!d.system.iter().any(|dep| dep.import == "typing"));
}

#[test]
fn shell_handles_env_split_string_indentation_and_functions() {
    use px::forfile::detector::Detector;
    let root =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/regressions/shell");
    let files = px::forfile::detector::scan_source_files(&root, 100);
    assert!(px::forfile::shell::ShellDetector.matches(&root, &files));
    let d = analyze("shell", "fixtures/regressions/shell");
    let names: Vec<_> = d.system.iter().map(|dep| dep.import.as_str()).collect();
    for command in ["command: jq", "command: ffmpeg", "command: inotifywait"] {
        assert!(names.contains(&command), "{names:?}");
    }
    assert!(!names.contains(&"command: localhelper"));
}

#[test]
fn project_detection_accepts_supported_script_extensions() {
    for file in ["app.mjs", "app.cjs", "app.jsx", "app.tsx", "app.zsh"] {
        assert!(px::forfile::detector::looks_like_project_file(
            std::path::Path::new(file)
        ));
    }
    assert!(
        px::forfile::detector::scan_source_files(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")),
            0
        )
        .is_empty()
    );
}
