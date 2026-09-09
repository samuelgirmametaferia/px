//! Suspicious-package investigation. Before installing anything px deems
//! unusual — especially node packages — it asks "want me to investigate?"
//! and then produces a report from public metadata. No network judgement
//! calls, just facts the user should see before piping anything into sh.

use strsim::jaro_winkler;

use crate::app::App;
use crate::apps::NpmMeta;
use crate::error::PxResult;

/// Packages whose *names* are so commonly typosquatted that a near-match is
/// worth flagging (typosquatting is the classic npm supply-chain attack).
const POPULAR_NPM: &[&str] = &[
    "express",
    "react",
    "request",
    "axios",
    "chalk",
    "lodash",
    "commander",
    "debug",
    "moment",
    "typescript",
    "eslint",
    "jest",
    "webpack",
    "vue",
    "next",
    "babel",
    "cross-env",
    "dotenv",
    "uuid",
    "glob",
    "minimist",
    "fs-extra",
    "yargs",
    "ms",
    "colors",
    "underscore",
    "bluebird",
    "socket.io",
    "mongoose",
    "node-fetch",
];

/// A reason a package looks unusual. Plain facts, shown to the user.
#[derive(Debug)]
pub struct Suspicion {
    pub reason: String,
}

/// Heuristic pass over npm metadata. Empty = looks ordinary.
pub fn analyze_npm(meta: &NpmMeta) -> Vec<Suspicion> {
    let mut flags = Vec::new();

    // Install scripts: preinstall/install/postinstall run arbitrary code.
    let risky: Vec<&str> = meta
        .scripts
        .iter()
        .filter(|k| {
            matches!(
                k.as_str(),
                "preinstall" | "install" | "postinstall" | "prepublish" | "prepare"
            )
        })
        .map(|k| k.as_str())
        .collect();
    if !risky.is_empty() {
        flags.push(Suspicion {
            reason: format!("runs code at install time via {}", risky.join(", ")),
        });
    }

    // Typosquat: similar to a very popular package but not it.
    let base = meta.name.rsplit('/').next().unwrap_or(&meta.name);
    if !POPULAR_NPM.contains(&base)
        && let Some(popular) = POPULAR_NPM
            .iter()
            .map(|p| (jaro_winkler(base, p), p))
            .filter(|(sim, _)| *sim >= 0.88)
            .max_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal))
            .map(|(_, p)| p)
    {
        flags.push(Suspicion {
            reason: format!("name is suspiciously close to the popular package '{popular}'"),
        });
    }

    // One maintainer is fine for a hobby lib, worth seeing in a report.
    if meta.maintainers.len() == 1 {
        flags.push(Suspicion {
            reason: format!("single maintainer ({})", meta.maintainers[0].name),
        });
    }

    // Very recently published and tiny.
    if meta.versions == 1 {
        flags.push(Suspicion {
            reason: "only ever published once (v0.0.1-style)".into(),
        });
    }
    if let Some(size) = meta.unpacked_size
        && size < 4 * 1024
    {
        flags.push(Suspicion {
            reason: "suspiciously small package (< 4 KiB)".into(),
        });
    }

    flags
}

/// Full report for the "investigate" flow: everything the user should see.
pub async fn investigate(app: &App, package: &str) -> PxResult<String> {
    let meta = crate::apps::npm_meta(&app.client, package).await?;
    let mut report = String::new();

    report.push_str(&format!("  name:          {}\n", meta.name));
    if let Some(d) = &meta.description {
        report.push_str(&format!("  description:   {}\n", d));
    }
    report.push_str(&format!("  versions:      {}\n", meta.versions));
    if let Some(size) = meta.unpacked_size {
        report.push_str(&format!(
            "  unpacked:      {}\n",
            crate::maintenance::human_size(size)
        ));
    }
    report.push_str(&format!(
        "  maintainers:   {}\n",
        meta.maintainers
            .iter()
            .map(|m| m.name.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    ));
    if let Some(created) = &meta.created {
        report.push_str(&format!("  first publish: {created}\n"));
    }
    if let Some(modified) = &meta.modified {
        report.push_str(&format!("  last publish:  {modified}\n"));
    }
    if !meta.scripts.is_empty() {
        report.push_str(&format!("  scripts:       {}\n", meta.scripts.join(", ")));
    }

    let flags = analyze_npm(&meta);
    if !flags.is_empty() {
        report.push_str("\n  flags:\n");
        for f in &flags {
            report.push_str(&format!("    • {}\n", f.reason));
        }
    } else {
        report.push_str("\n  nothing unusual found in the metadata\n");
    }
    Ok(report)
}
