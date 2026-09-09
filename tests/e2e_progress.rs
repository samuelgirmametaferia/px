//! End-to-end progress verification: a fake recipe whose install command
//! performs a REAL multi-megabyte download. The full pipeline runs —
//! resolve → plan → bar → network monitor → summary — with zero mocking.
//! This is the test the google-chrome incident demanded: if the monitor or
//! bar is wired wrong, `downloaded` in the summary comes back empty/zero.

use std::process::Command;

const RECIPE: &str = r#"
[meta]
id = "fake"
name = "Fake Distro (progress e2e)"
version = 1

[[detection]]
command = "definitely-not-a-real-binary"

[[sources]]
id = "repo"
label = "fake repo"

[sources.search]
argv = ["echo", "bigpkg"]
parse = "pacman_search"

[sources.info]
argv = ["printf", "Name : bigpkg\nVersion : 1.0\nDownload Size : 10.00 MiB\n"]
parse = "pacman_info"

[sources.installed]
argv = ["false"]

[sources.install]
# a real 10 MiB download — the network-flow monitor must credit it
argv = ["curl", "-sSL", "https://registry.npmjs.org/typescript/-/typescript-5.6.3.tgz", "-o", "/dev/null"]
"#;

#[test]
fn install_pipeline_credits_real_download_bytes() {
    let recipe_path = std::env::temp_dir().join("px-e2e-recipe.toml");
    std::fs::write(&recipe_path, RECIPE).unwrap();

    let bin = env!("CARGO_BIN_EXE_px");
    // isolated caches so the test is hermetic (a stale provider cache once
    // made this test lie — px cached a "miss" from before a parser fix)
    let cache_dir = std::env::temp_dir().join(format!("px-e2e-cache-{}", std::process::id()));
    let state_dir = std::env::temp_dir().join(format!("px-e2e-state-{}", std::process::id()));
    let output = Command::new(bin)
        .arg("--recipe")
        .arg(&recipe_path)
        .arg("-y")
        .arg("install")
        .arg("bigpkg")
        .env("NO_COLOR", "1")
        .env("XDG_CACHE_HOME", &cache_dir)
        .env("XDG_STATE_HOME", &state_dir)
        .output()
        .expect("px must run");
    let _ = std::fs::remove_dir_all(&cache_dir);
    let _ = std::fs::remove_dir_all(&state_dir);

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "install must succeed\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );

    // the resolution must have found the package
    assert!(stdout.contains("bigpkg"), "resolved package: {stdout}");

    // THE assertion: the monitor credited real bytes from the wire
    assert!(
        stdout.contains("downloaded "),
        "summary must report downloaded bytes: {stdout}"
    );

    // extract the number and require a meaningful chunk of the 10 MiB
    let kb: u64 = stdout
        .lines()
        .find_map(|l| {
            let pos = l.find("downloaded ")?;
            let rest = &l[pos + "downloaded ".len()..];
            let num: String = rest
                .chars()
                .take_while(|c| c.is_ascii_digit() || *c == '.')
                .collect();
            let unit = rest[num.len()..].split_whitespace().next().unwrap_or("");
            let n: f64 = num.parse().ok()?;
            match unit {
                "MiB" => Some((n * 1024.0) as u64),
                "KiB" => Some(n as u64),
                _ => None,
            }
        })
        .unwrap_or(0);
    assert!(
        kb >= 512,
        "monitor must credit a real chunk of the 10 MiB download (got {kb} KiB)\n{stdout}"
    );
    assert!(
        stdout.contains("1 installed"),
        "summary stats present: {stdout}"
    );
}
