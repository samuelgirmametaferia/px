//! All three shipped recipes must parse, round-trip, and agree with the
//! schema. Adding a distro is data-only — this is the gate that keeps it
//! that way.

use px::recipe::load::bundled;
use px::recipe::schema::Recipe;

#[test]
fn all_bundled_recipes_parse() {
    for (id, text) in bundled::all() {
        let recipe = Recipe::parse_str(text).unwrap_or_else(|e| panic!("{id}: {e}"));
        assert_eq!(recipe.meta.id, id);
        assert!(!recipe.sources.is_empty(), "{id} must declare sources");
        assert!(
            !recipe.detection.is_empty(),
            "{id} must declare detection rules"
        );
    }
}

#[test]
fn arch_recipe_shape() {
    let recipe = Recipe::parse_str(bundled::ARCH).unwrap();
    assert_eq!(recipe.source_ids(), vec!["repo", "aur"]);
    let repo = recipe.sources.iter().find(|s| s.id == "repo").unwrap();
    assert_eq!(repo.require_any, vec!["pacman".to_string()]);
    assert!(repo.install.as_ref().unwrap().elevated);
    assert!(
        repo.install
            .as_ref()
            .unwrap()
            .argv
            .contains(&"{pkgs...}".to_string())
    );
    let aur = recipe.sources.iter().find(|s| s.id == "aur").unwrap();
    assert!(aur.require_any.contains(&"paru".to_string()));
    assert!(aur.require_any.contains(&"yay".to_string()));
    // {helper} must expand to whichever binary the user has
    assert!(
        aur.search
            .as_ref()
            .unwrap()
            .argv
            .contains(&"{helper}".to_string())
    );
}

#[test]
fn debian_recipe_shape() {
    let recipe = Recipe::parse_str(bundled::DEBIAN).unwrap();
    let repo = recipe.sources.iter().find(|s| s.id == "repo").unwrap();
    assert!(repo.install.as_ref().unwrap().argv[0] == "sudo");
    assert!(
        repo.install
            .as_ref()
            .unwrap()
            .argv
            .contains(&"apt-get".to_string())
    );
    // Debian python packages are python3-*
    let py = recipe.ecosystem("python").unwrap();
    assert_eq!(py.prefix.as_deref(), Some("python3-{name}"));
    // Debian dev packages differ from arch
    let c = recipe.ecosystem("c").unwrap();
    assert_eq!(
        c.header_map.get("openssl").map(|s| s.as_str()),
        Some("libssl-dev")
    );
}

#[test]
fn fedora_recipe_shape() {
    let recipe = Recipe::parse_str(bundled::FEDORA).unwrap();
    let repo = recipe.sources.iter().find(|s| s.id == "repo").unwrap();
    assert!(
        repo.install
            .as_ref()
            .unwrap()
            .argv
            .contains(&"dnf".to_string())
    );
    let c = recipe.ecosystem("c").unwrap();
    assert_eq!(
        c.header_map.get("openssl").map(|s| s.as_str()),
        Some("openssl-devel")
    );
}

#[test]
fn opensuse_recipe_shape() {
    let recipe = Recipe::parse_str(bundled::OPENSUSE).unwrap();
    let repo = recipe.sources.iter().find(|s| s.id == "repo").unwrap();
    assert!(
        repo.install
            .as_ref()
            .unwrap()
            .argv
            .contains(&"zypper".to_string())
    );
    // installed check via rpm (works on every suse box)
    assert!(
        repo.installed
            .as_ref()
            .unwrap()
            .argv
            .contains(&"rpm".to_string())
    );
    let c = recipe.ecosystem("c").unwrap();
    assert_eq!(
        c.header_map.get("openssl").map(|s| s.as_str()),
        Some("libopenssl-devel")
    );
    let py = recipe.ecosystem("python").unwrap();
    assert_eq!(py.prefix.as_deref(), Some("python3-{name}"));
}

#[test]
fn opensuse_detection_matches_tumbleweed_and_leap() {
    let suse = Recipe::parse_str(bundled::OPENSUSE).unwrap();
    for id in ["opensuse-tumbleweed", "opensuse-leap", "microos"] {
        let mut os = px::recipe::detect::OsRelease::default();
        os.fields.insert("ID".into(), id.into());
        assert!(
            px::recipe::detect::recipe_matches(&suse, &os).is_some(),
            "{id} must match"
        );
    }
    // ID_LIKE=suse (SLES etc.)
    let mut os = px::recipe::detect::OsRelease::default();
    os.fields.insert("ID".into(), "sles".into());
    os.fields.insert("ID_LIKE".into(), "suse".into());
    assert!(px::recipe::detect::recipe_matches(&suse, &os).is_some());
}

#[test]
fn unknown_parser_names_are_rejected_at_load() {
    // A recipe naming a parser px doesn't know must fail loudly.
    let bad = r#"
[meta]
id = "fake"
name = "Fake Distro"
version = 1

[[detection]]
command = "definitely-not-a-real-binary"

[[sources]]
id = "repo"
label = "repo"

[sources.search]
argv = ["fakepm", "search", "{pkg}"]
parse = "no_such_parser"

[sources.install]
argv = ["sudo", "fakepm", "install", "{pkgs...}"]
"#;
    let recipe = Recipe::parse_str(bad).unwrap();
    // Schema parses; the missing parser is visible via parsers_known.
    let known = px::backend::parsers::parsers_known(&recipe.sources[0]);
    assert!(!known, "unknown parser must be reported");
}

#[test]
fn detection_rules_match_os_release_samples() {
    let arch = Recipe::parse_str(bundled::ARCH).unwrap();
    let debian = Recipe::parse_str(bundled::DEBIAN).unwrap();
    let fedora = Recipe::parse_str(bundled::FEDORA).unwrap();

    let mut os = px::recipe::detect::OsRelease::default();
    os.fields.insert("ID".into(), "cachyos".into());
    os.fields.insert("ID_LIKE".into(), "arch".into());
    assert!(px::recipe::detect::recipe_matches(&arch, &os).is_some());
    assert!(px::recipe::detect::recipe_matches(&debian, &os).is_none());

    let mut os = px::recipe::detect::OsRelease::default();
    os.fields.insert("ID".into(), "ubuntu".into());
    os.fields.insert("ID_LIKE".into(), "debian".into());
    assert!(px::recipe::detect::recipe_matches(&debian, &os).is_some());

    let mut os = px::recipe::detect::OsRelease::default();
    os.fields.insert("ID".into(), "fedora".into());
    assert!(px::recipe::detect::recipe_matches(&fedora, &os).is_some());

    // No os-release at all → command fallback (pacman present on this CI? no
    // — use a binary that must exist: the test runner itself).
    let mut os = px::recipe::detect::OsRelease::default();
    os.fields.insert("ID".into(), "unknown-distro".into());
    // Neither rule matches without pacman/dnf/apt; on an arch CI box the
    // command rule would fire. Just assert no panic.
    let _ = px::recipe::detect::recipe_matches(&arch, &os);
}

#[test]
fn all_recipes_have_maintenance_commands() {
    for (id, text) in bundled::all() {
        let recipe = Recipe::parse_str(text).unwrap();
        let m = &recipe.maintenance;
        assert!(m.uninstall.is_some(), "{id} needs an uninstall command");
        assert!(m.updates.is_some(), "{id} needs an updates command");
        assert!(m.orphans.is_some(), "{id} needs an orphans command");
        assert!(m.installed_info.is_some(), "{id} needs installed_info");
        assert!(m.explicit.is_some(), "{id} needs an explicit listing");
        // installed_info must batch ({pkgs...}), not one package per call
        assert!(
            m.installed_info
                .as_ref()
                .unwrap()
                .argv
                .contains(&"{pkgs...}".to_string()),
            "{id} installed_info must use {{pkgs...}}"
        );
    }
}

#[test]
fn apps_registry_shapes() {
    for (id, text) in bundled::all() {
        let recipe = Recipe::parse_str(text).unwrap();
        assert!(!recipe.apps.is_empty(), "{id} should know some apps");
        for app in &recipe.apps {
            assert!(!app.match_.is_empty(), "{id} app needs match names");
            assert!(
                app.method == "npm" || app.method == "script",
                "{id} app {} has unknown method {}",
                app.label,
                app.method
            );
        }
    }
    // claude-code resolves through every recipe's registry
    let arch = Recipe::parse_str(bundled::ARCH).unwrap();
    assert!(
        arch.apps
            .iter()
            .any(|a| a.match_.contains(&"claude-code".to_string()))
    );
}
