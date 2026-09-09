//! Command generation: the distro contract, pinned with a MockExecutor.
//! These tests prove px drives apt/dnf correctly WITHOUT a Debian or Fedora
//! machine — the entire point of the recipe design.

use px::backend::source::SourceProvider;
use px::backend::{InstallCtx, Installer, activate_sources_opt};
use px::exec::{ExecOutput, MockExecutor, expand_argv};
use px::recipe::schema::Recipe;
use std::sync::Arc;

fn run_install(recipe_text: &str, pkgs: &[&str]) -> Vec<Vec<String>> {
    let recipe = Recipe::parse_str(recipe_text).unwrap();
    let (active, _) = activate_sources_opt(&recipe.sources, true);
    let exec = Arc::new(MockExecutor::with_responses(vec![ExecOutput {
        status: 0,
        ..Default::default()
    }]));
    let provider = SourceProvider::new(active[0].clone(), exec.clone());
    let rt = tokio::runtime::Runtime::new().unwrap();
    // dry_run skips the sudo preflight (no real sudo in tests); the
    // MockExecutor records the argv either way.
    rt.block_on(provider.install(
        &pkgs.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
        InstallCtx {
            dry_run: true,
            ..Default::default()
        },
    ))
    .unwrap();
    exec.calls.lock().unwrap().clone()
}

#[test]
fn arch_install_generates_pacman_argv() {
    let calls = run_install(px::recipe::load::bundled::ARCH, &["ripgrep", "sl"]);
    assert_eq!(calls.len(), 1);
    // --noconfirm: px already confirmed the plan, pacman stays quiet
    assert_eq!(
        calls[0],
        vec![
            "sudo".to_string(),
            "pacman".to_string(),
            "-S".to_string(),
            "--needed".to_string(),
            "--noconfirm".to_string(),
            "ripgrep".to_string(),
            "sl".to_string(),
        ]
    );
}

#[test]
fn debian_install_generates_apt_argv() {
    let calls = run_install(px::recipe::load::bundled::DEBIAN, &["ffmpeg"]);
    assert_eq!(calls.len(), 1);
    assert_eq!(
        calls[0],
        vec![
            "sudo".to_string(),
            "apt-get".to_string(),
            "install".to_string(),
            "-y".to_string(),
            "ffmpeg".to_string(),
        ]
    );
}

#[test]
fn fedora_install_generates_dnf_argv() {
    let calls = run_install(px::recipe::load::bundled::FEDORA, &["ffmpeg", "jq"]);
    assert_eq!(calls.len(), 1);
    assert_eq!(
        calls[0],
        vec![
            "sudo".to_string(),
            "dnf".to_string(),
            "install".to_string(),
            "-y".to_string(),
            "ffmpeg".to_string(),
            "jq".to_string(),
        ]
    );
}

#[test]
fn opensuse_install_generates_zypper_argv() {
    let calls = run_install(px::recipe::load::bundled::OPENSUSE, &["ffmpeg"]);
    assert_eq!(calls.len(), 1);
    assert_eq!(
        calls[0],
        vec![
            "sudo".to_string(),
            "zypper".to_string(),
            "--non-interactive".to_string(),
            "install".to_string(),
            "ffmpeg".to_string(),
        ]
    );
}

#[test]
fn helper_placeholder_expands() {
    // {helper} must become whichever AUR helper binary matched.
    let argv = expand_argv(
        &["{helper}", "-S", "--needed", "--noconfirm", "{pkgs...}"]
            .iter()
            .map(|s| s.to_string())
            .collect::<Vec<String>>(),
        "google-chrome",
        &["google-chrome".to_string()],
        Some("yay"),
        None,
    );
    assert_eq!(argv[0], "yay");
    assert_eq!(argv.last().unwrap(), "google-chrome");
}

#[test]
fn aur_search_uses_helper() {
    let recipe = Recipe::parse_str(px::recipe::load::bundled::ARCH).unwrap();
    let (active, inactive) = px::backend::activate_sources(&recipe.sources);
    // On a machine with paru/yay, aur is active; without, it's reported.
    if active.iter().any(|s| s.def.id == "aur") {
        let aur = active.iter().find(|s| s.def.id == "aur").unwrap();
        assert!(!aur.helper.is_empty());
    }
    // repo is always active where pacman exists.
    assert!(active.iter().any(|s| s.def.id == "repo"));
    let _ = inactive;
}

#[test]
fn multi_pkg_placeholder_expands_every_arg() {
    let argv = expand_argv(
        &["sudo", "pacman", "-S", "--needed", "{pkgs...}"]
            .iter()
            .map(|s| s.to_string())
            .collect::<Vec<String>>(),
        "a",
        &["a".to_string(), "b".to_string(), "c".to_string()],
        None,
        None,
    );
    assert_eq!(argv.len(), 7); // 4 fixed + 3 expanded pkgs
    assert_eq!(argv[4], "a");
    assert_eq!(argv[5], "b");
    assert_eq!(argv[6], "c");
}

#[test]
fn upgrade_generates_per_distro_argv() {
    fn expect(text: &str) -> Vec<String> {
        let recipe = Recipe::parse_str(text).unwrap();
        let def = recipe.maintenance.upgrade.expect("upgrade command");
        expand_argv(&def.argv, "", &[], None, None)
    }
    assert_eq!(
        expect(px::recipe::load::bundled::ARCH),
        vec![
            "sudo".to_string(),
            "pacman".to_string(),
            "-Syu".to_string(),
            "--noconfirm".to_string(),
        ]
    );
    assert_eq!(
        expect(px::recipe::load::bundled::DEBIAN),
        vec![
            "sudo".to_string(),
            "apt-get".to_string(),
            "upgrade".to_string(),
            "-y".to_string(),
        ]
    );
    assert_eq!(
        expect(px::recipe::load::bundled::FEDORA),
        vec![
            "sudo".to_string(),
            "dnf".to_string(),
            "upgrade".to_string(),
            "-y".to_string(),
        ]
    );
    assert_eq!(
        expect(px::recipe::load::bundled::OPENSUSE),
        vec![
            "sudo".to_string(),
            "zypper".to_string(),
            "--non-interactive".to_string(),
            "update".to_string(),
        ]
    );
}

#[test]
fn all_recipes_declare_upgrade() {
    for (id, text) in px::recipe::load::bundled::all() {
        let recipe = Recipe::parse_str(text).unwrap();
        assert!(
            recipe.maintenance.upgrade.is_some(),
            "{id} needs an upgrade command"
        );
    }
}
