//! Parser tests against recorded fixture output — including the apt/dnf
//! fixtures, so non-Arch distros are covered without their machines.

use px::backend::parsers as p;

fn fixture(path: &str) -> String {
    let full = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures")
        .join(path);
    std::fs::read_to_string(&full).unwrap_or_else(|e| panic!("{}: {e}", full.display()))
}

// ---------------------------------------------------------------- pacman

#[test]
fn pacman_search_names_only() {
    let hits = p::pacman_search(&fixture("pacman/ssq-neovim.txt"), "repo");
    assert!(!hits.is_empty());
    assert!(hits.iter().all(|h| h.source == "repo"));
    assert!(hits.iter().any(|h| h.name == "neovim"));
}

#[test]
fn pacman_info_block() {
    let hits = p::pacman_info(&fixture("pacman/si-python-flask.txt"), "repo");
    assert!(!hits.is_empty(), "pacman -Si output must parse");
    let flask = hits.iter().find(|h| h.name == "python-flask").unwrap();
    assert!(!flask.version.is_empty());
    assert!(flask.description.is_some());
}

#[test]
fn pacman_installed() {
    let pairs = p::pacman_installed(&fixture("pacman/q-installed.txt"));
    assert!(!pairs.is_empty());
    assert_eq!(pairs[0].0, "ripgrep");
}

#[test]
fn pacman_files() {
    let hits = p::pacman_files(&fixture("pacman/f-openssl.txt"), "repo");
    assert!(!hits.is_empty());
    assert_eq!(hits[0].name, "openssl");
}

// -------------------------------------------------------------------- apt

#[test]
fn apt_search_format() {
    let hits = p::apt_search(&fixture("apt/search-ffmpeg.txt"), "repo");
    assert_eq!(hits.len(), 3);
    assert_eq!(hits[0].name, "ffmpeg");
    assert_eq!(
        hits[0].description.as_deref(),
        Some("Complete solution to record, convert and stream audio and video")
    );
}

#[test]
fn apt_show_paragraphs() {
    let hits = p::apt_info(&fixture("apt/show-ffmpeg.txt"), "repo");
    assert_eq!(hits.len(), 2);
    assert_eq!(hits[0].name, "ffmpeg");
    assert!(hits[0].version.starts_with("7:"));
    assert_eq!(hits[1].name, "libffmpeg-dev");
}

// -------------------------------------------------------------------- dnf

#[test]
fn dnf_search_skips_headers_strips_arch() {
    let hits = p::dnf_search(&fixture("dnf/search-vim.txt"), "repo");
    let names: Vec<&str> = hits.iter().map(|h| h.name.as_str()).collect();
    assert_eq!(names, vec!["vim-enhanced", "vim-common"]);
    assert_eq!(
        hits[0].description.as_deref(),
        Some("The latest version of the VIM editor")
    );
}

#[test]
fn dnf_info_block() {
    let hits = p::dnf_info(&fixture("dnf/info-vim.txt"), "repo");
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].name, "vim-enhanced");
    assert_eq!(hits[0].version, "9.1.600");
}

#[test]
fn dnf_provides() {
    let hits = p::dnf_provides(&fixture("dnf/provides-openssl.txt"), "repo");
    assert!(!hits.is_empty());
    assert_eq!(hits[0].name, "openssl-devel-3.2.2-1.fc40.x86_64");
}

// ----------------------------------------------------------------- zypper

#[test]
fn zypper_search_pipe_table() {
    let hits = p::zypper_search(&fixture("zypper/search-ffmpeg.txt"), "repo");
    let names: Vec<&str> = hits.iter().map(|h| h.name.as_str()).collect();
    // only package rows, no srcpackage, no header/separator noise
    assert_eq!(names, vec!["ffmpeg", "ffmpeg-6", "ffmpeg-7"]);
    assert_eq!(hits[0].version, "6.1.1-4.1");
}

#[test]
fn zypper_info_block() {
    let hits = p::zypper_info(&fixture("zypper/info-ffmpeg.txt"), "repo");
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].name, "ffmpeg");
    assert_eq!(hits[0].version, "6.1.1-4.1");
    assert_eq!(
        hits[0].description.as_deref(),
        Some("Library and tools for audio and video conversion")
    );
}

// ------------------------------------------------------- dispatcher + helpers

#[test]
fn dispatcher_maps_recipe_names() {
    assert!(!p::parse_output("apt_search", "ffmpeg - x", "repo").is_empty());
    assert!(!p::parse_output("dnf_search", "vim.x86_64 : editor", "repo").is_empty());
    assert!(p::parse_output("nope", "anything", "repo").is_empty());
}
