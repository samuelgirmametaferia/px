//! Output parsers, one per `parse = "..."` name a recipe can declare.
//!
//! These are deliberately tolerant: real-world package-manager output has
//! locale quirks and version drift, so each parser skips what it doesn't
//! understand instead of failing. All of them are tested against committed
//! fixture files, not live output.

use crate::backend::PackageHit;

/// Parse a `key: value` paragraph block (pacman -Si, apt-cache show,
/// dnf info, dpkg -s) into a flat map.
fn parse_paragraph(text: &str) -> Vec<Vec<(String, String)>> {
    let mut blocks: Vec<Vec<(String, String)>> = Vec::new();
    let mut current: Vec<(String, String)> = Vec::new();
    for line in text.lines() {
        if line.trim().is_empty() {
            if !current.is_empty() {
                blocks.push(std::mem::take(&mut current));
            }
            continue;
        }
        if let Some((k, v)) = line.split_once(':') {
            let k = k.trim();
            // Heuristic: a key is short and has no spaces; otherwise this is
            // a description line containing a colon.
            if !k.is_empty() && !k.contains(' ') && k.len() < 40 {
                current.push((k.to_string(), v.trim().to_string()));
            }
        }
    }
    if !current.is_empty() {
        blocks.push(current);
    }
    blocks
}

fn field<'a>(block: &'a [(String, String)], key: &str) -> Option<&'a str> {
    block
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(key))
        .map(|(_, v)| v.as_str())
}

fn hit_from_block(block: &[(String, String)], source: &str) -> Option<PackageHit> {
    let name = field(block, "Name")
        .or_else(|| field(block, "Package"))?
        .to_string();
    let version = field(block, "Version").unwrap_or("").to_string();
    // zypper puts the real text on indented lines after "Description :"
    // (empty value) and the one-liner on "Summary :" — prefer whichever is
    // actually non-empty.
    let description = ["Description", "Summary"]
        .iter()
        .filter_map(|key| field(block, key))
        .map(|s| s.trim())
        .find(|s| !s.is_empty())
        .map(|s| s.to_string());
    Some(PackageHit {
        name,
        version,
        description,
        source: source.to_string(),
        score: 0,
    })
}

// ---------------------------------------------------------------- pacman

/// `pacman -Ssq <term>` / `paru -Ssqa <term>` / `yay -Ssqa <term>`:
/// package names, one per line.
pub fn pacman_search(text: &str, source: &str) -> Vec<PackageHit> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(|name| PackageHit {
            name: name.to_string(),
            version: String::new(),
            description: None,
            source: source.to_string(),
            score: 0,
        })
        .collect()
}

/// `pacman -Si <pkg>` / `paru -Sia <pkg>`: key-value block.
pub fn pacman_info(text: &str, source: &str) -> Vec<PackageHit> {
    parse_paragraph(text)
        .iter()
        .filter_map(|b| hit_from_block(b, source))
        .collect()
}

/// `pacman -Q <pkg>`: "name version" on stdout, exit 0 when installed.
pub fn pacman_installed(text: &str) -> Vec<(String, String)> {
    text.lines()
        .filter_map(|l| {
            let mut it = l.split_whitespace();
            match (it.next(), it.next()) {
                (Some(n), Some(v)) => Some((n.to_string(), v.to_string())),
                (Some(n), None) => Some((n.to_string(), String::new())),
                _ => None,
            }
        })
        .collect()
}

/// `pacman -F <path>`: "repo/name version  /path" lines.
pub fn pacman_files(text: &str, source: &str) -> Vec<PackageHit> {
    text.lines()
        .filter_map(|l| {
            let first = l.split_whitespace().next()?;
            let (repo, name) = first.split_once('/')?;
            let _ = repo;
            Some(PackageHit {
                name: name.to_string(),
                version: l.split_whitespace().nth(1).unwrap_or("").to_string(),
                description: None,
                source: source.to_string(),
                score: 0,
            })
        })
        .collect()
}

// ------------------------------------------------------------------- apt

/// `apt-cache search <term>`: "package - description" per line.
pub fn apt_search(text: &str, source: &str) -> Vec<PackageHit> {
    text.lines()
        .filter_map(|l| {
            let (name, desc) = l.split_once(" - ")?;
            Some(PackageHit {
                name: name.trim().to_string(),
                version: String::new(),
                description: Some(desc.trim().to_string()),
                source: source.to_string(),
                score: 0,
            })
        })
        .collect()
}

/// `apt-cache show <pkg>`: key-value block ("Package:").
pub fn apt_info(text: &str, source: &str) -> Vec<PackageHit> {
    parse_paragraph(text)
        .iter()
        .filter_map(|b| hit_from_block(b, source))
        .collect()
}

/// `apt-file search <path>`: "package: path" per line.
pub fn apt_file_search(text: &str, source: &str) -> Vec<PackageHit> {
    text.lines()
        .filter_map(|l| {
            let (pkg, _) = l.split_once(':')?;
            Some(PackageHit {
                name: pkg.trim().to_string(),
                version: String::new(),
                description: None,
                source: source.to_string(),
                score: 0,
            })
        })
        .collect()
}

// ------------------------------------------------------------------- dnf

/// `dnf -q search <term>`: header noise, then "name.arch : summary" lines.
pub fn dnf_search(text: &str, source: &str) -> Vec<PackageHit> {
    text.lines()
        .map(str::trim)
        .filter(|l| {
            !l.is_empty()
                && !l.starts_with('=')
                && !l.starts_with("Last metadata")
                && !l.starts_with("Nome")
                && !l.starts_with("Name and Summary")
                && !l.starts_with("N/S")
                && !l.starts_with("Name exactly")
        })
        .filter_map(|l| {
            let (name_field, desc) = match l.split_once(':') {
                Some((n, d)) => (n, Some(d.trim().to_string())),
                None => (l, None),
            };
            // Strip ".arch" suffix: "vim-enhanced.x86_64" → "vim-enhanced"
            let name = name_field
                .trim()
                .split('.')
                .next()
                .unwrap_or(name_field.trim())
                .to_string();
            if name.is_empty() || name.contains(' ') {
                return None;
            }
            Some(PackageHit {
                name,
                version: String::new(),
                description: desc,
                source: source.to_string(),
                score: 0,
            })
        })
        .collect()
}

/// `dnf -q info <pkg>`: key-value block.
pub fn dnf_info(text: &str, source: &str) -> Vec<PackageHit> {
    parse_paragraph(text)
        .iter()
        .filter_map(|b| hit_from_block(b, source))
        .collect()
}

/// `dnf -q provides <path>`: "name-version.arch : path" or
/// "name-version.arch repo" lines. Keep just the name.
pub fn dnf_provides(text: &str, source: &str) -> Vec<PackageHit> {
    text.lines()
        .filter_map(|l| {
            let first = l.split(':').next()?.trim();
            if first.is_empty() {
                return None;
            }
            Some(PackageHit {
                name: first.to_string(),
                version: String::new(),
                description: None,
                source: source.to_string(),
                score: 0,
            })
        })
        .collect()
}

// ------------------------------------------------------------------ copr

/// `dnf copr search <term>`: "owner/project" lines plus noise.
pub fn copr_search(text: &str, source: &str) -> Vec<PackageHit> {
    text.lines()
        .map(str::trim)
        .filter(|l| l.contains('/') && !l.contains(' '))
        .map(|l| PackageHit {
            name: l.to_string(),
            version: String::new(),
            description: None,
            source: source.to_string(),
            score: 0,
        })
        .collect()
}

// ---------------------------------------------------------------- zypper

/// `zypper se -t package <term>`: pipe-delimited table.
///   S  | Name        | Type    | Version   | Arch   | Repository
///   ---+-------------+---------+-----------+--------+-----------
///   i+ | ffmpeg      | package | 6.1.1-4.1 | x86_64 | Main (OSS)
pub fn zypper_search(text: &str, source: &str) -> Vec<PackageHit> {
    text.lines()
        .filter_map(|l| {
            if !l.contains('|') {
                return None; // headers, "Loading repository data", blank lines
            }
            let cols: Vec<&str> = l.split('|').map(|c| c.trim()).collect();
            if cols.len() < 3 || cols[1].is_empty() || cols[1] == "Name" {
                return None;
            }
            if cols[2] != "package" {
                return None; // srcpackage, pattern, application...
            }
            let name = cols[1].to_string();
            if name
                .chars()
                .next()
                .is_some_and(|c| !c.is_ascii_alphanumeric() && c != '_')
            {
                return None; // separator rows / malformed
            }
            Some(PackageHit {
                version: cols.get(3).unwrap_or(&"").to_string(),
                description: None,
                name,
                source: source.to_string(),
                score: 0,
            })
        })
        .collect()
}

/// `zypper info <pkg>`: key-value block ("Name :", "Version :", "Summary :").
pub fn zypper_info(text: &str, source: &str) -> Vec<PackageHit> {
    parse_paragraph(text)
        .iter()
        .filter_map(|b| hit_from_block(b, source))
        .collect()
}

/// `zypper se -f <path>`: file-provider search, same table shape.
pub fn zypper_provides(text: &str, source: &str) -> Vec<PackageHit> {
    zypper_search(text, source)
}

// ------------------------------------------------------------- dispatcher

/// Run the parser named by a recipe's `parse = "..."`. Unknown names are an
/// error at load time; here they produce an empty result with a warning.
pub fn parse_output(name: &str, text: &str, source: &str) -> Vec<PackageHit> {
    match name {
        "pacman_search" | "helper_search" => pacman_search(text, source),
        "pacman_info" | "helper_info" => pacman_info(text, source),
        "pacman_files" => pacman_files(text, source),
        "apt_search" => apt_search(text, source),
        "apt_info" => apt_info(text, source),
        "apt_file_search" => apt_file_search(text, source),
        "dnf_search" => dnf_search(text, source),
        "dnf_info" => dnf_info(text, source),
        "dnf_provides" => dnf_provides(text, source),
        "copr_search" => copr_search(text, source),
        "zypper_search" => zypper_search(text, source),
        "zypper_info" => zypper_info(text, source),
        "zypper_provides" => zypper_provides(text, source),
        other => {
            tracing::warn!("unknown parser '{other}' (source {source})");
            Vec::new()
        }
    }
}

/// All parser names a recipe may reference.
pub const KNOWN_PARSERS: &[&str] = &[
    "pacman_search",
    "pacman_info",
    "pacman_files",
    "helper_search",
    "helper_info",
    "apt_search",
    "apt_info",
    "apt_file_search",
    "dnf_search",
    "dnf_info",
    "dnf_provides",
    "copr_search",
    "zypper_search",
    "zypper_info",
    "zypper_provides",
];

/// True when a source's declared parsers all exist (drives `px doctor`).
pub fn parsers_known(def: &crate::recipe::schema::SourceDef) -> bool {
    [&def.search, &def.info, &def.provides]
        .into_iter()
        .flatten()
        .filter_map(|c| c.parse.as_deref())
        .all(|p| KNOWN_PARSERS.contains(&p))
}

// ------------------------------------------------------------ maintenance
// Parsers for the recipe's [maintenance.*] commands — uninstall support,
// update notices, orphan/unused detection. These return plain data rather
// than PackageHits.

/// One name per line ("pacman -Qtdq", "apt-mark showmanual", ...).
pub fn names_lines(text: &str) -> Vec<String> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|l| l.split_whitespace().next().unwrap_or(l).to_string())
        .collect()
}

/// Update listings across package managers:
///   pacman -Qu:   "firefox 130.0-1 -> 131.0-1"
///   apt list:     "ffmpeg/jammy-updates 7:7.0.2-3 upgradable [...]"
///   dnf check:    "firefox.x86_64  131.0-1.fc40  updates"
pub fn updates_names(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(|l| {
            let t = l.trim();
            if t.is_empty()
                || t.starts_with("Listing")
                || t.starts_with("Last metadata")
                || t.starts_with('=')
            {
                return None;
            }
            let first = t.split_whitespace().next()?;
            let name = first
                .split('/')
                .next()
                .unwrap_or(first)
                .split('.')
                .next()
                .unwrap_or(first);
            if name.is_empty() || name.contains(':') && !name.contains('/') {
                return None;
            }
            Some(name.to_string())
        })
        .collect()
}

/// "SIZE<TAB>NAME" lines (dpkg-query -W -f, rpm --queryformat).
pub fn size_lines(text: &str) -> Vec<(String, u64)> {
    text.lines()
        .filter_map(|l| {
            let mut parts = l.split('\t');
            let size: u64 = parts.next()?.trim().parse().ok()?;
            let name = parts.next()?.trim().to_string();
            if name.is_empty() {
                None
            } else {
                Some((name, size))
            }
        })
        .collect()
}

/// Human size → bytes ("2.40 MiB", "1.5 GiB", "740 KiB", "1024 B").
pub fn human_size_bytes(s: &str) -> Option<u64> {
    let s = s.trim();
    let (num, unit) = s.split_once(' ')?;
    let num: f64 = num.parse().ok()?;
    let mult = match unit.trim() {
        "B" => 1.0,
        "KiB" | "kB" | "KB" => 1024.0,
        "MiB" | "MB" => 1024.0 * 1024.0,
        "GiB" | "GB" => 1024.0 * 1024.0 * 1024.0,
        _ => return None,
    };
    Some((num * mult) as u64)
}

/// pacman -Qi block → (installed size, install date) for suggest.
pub fn pacman_qi(text: &str) -> Option<(u64, Option<String>)> {
    let mut size = None;
    let mut date = None;
    for line in text.lines() {
        if let Some((k, v)) = line.split_once(':') {
            let k = k.trim();
            let v = v.trim();
            if k == "Installed Size" {
                size = human_size_bytes(v);
            } else if k == "Install Date" {
                date = Some(v.to_string());
            }
        }
    }
    size.map(|s| (s, date))
}

/// Dispatch a maintenance parse by recipe parser name → package names.
pub fn parse_maintenance_names(parse: &str, text: &str) -> Vec<String> {
    match parse {
        "names_lines" | "apt_autoremove" | "dnf_leaves" => names_lines(text),
        "name_version_lines" | "apt_upgradable" | "dnf_check_update" => updates_names(text),
        "zypper_search" => zypper_search(text, "repo")
            .into_iter()
            .map(|h| h.name)
            .collect(),
        "dpkg_size" => size_lines(text).into_iter().map(|(n, _)| n).collect(),
        other => {
            tracing::warn!("unknown maintenance parser '{other}'");
            Vec::new()
        }
    }
}
