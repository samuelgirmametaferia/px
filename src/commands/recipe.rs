//! `px recipe list|show`.

use crate::app::App;
use crate::error::PxResult;

pub fn list(app: App) -> PxResult<()> {
    let style = &app.style;
    println!("{}", style.banner());
    for (id, text) in crate::recipe::load::bundled::all() {
        match crate::recipe::schema::Recipe::parse_str(text) {
            Ok(r) => {
                let sources: Vec<String> = r.sources.iter().map(|s| s.id.clone()).collect();
                println!(
                    "  {} {:<8} {:<24} sources: {}",
                    style.ok("✔"),
                    style.bold(id),
                    r.meta.name,
                    sources.join(", ")
                );
            }
            Err(e) => println!("  {} {id}: {e}", style.err("✘")),
        }
    }
    // Cached but not bundled (custom distros the user fetched).
    if let Ok(entries) = std::fs::read_dir(crate::recipe::load::cache_dir()) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if let Some(id) = name.strip_suffix(".toml") {
                let is_bundled = crate::recipe::load::bundled::all()
                    .iter()
                    .any(|(bid, _)| *bid == id);
                if !is_bundled {
                    println!("  {} {id} (cached)", style.warn("○"));
                }
            }
        }
    }
    Ok(())
}

pub fn show(app: App) -> PxResult<()> {
    let style = &app.style;
    let recipe = app.recipe();
    println!("{}", style.banner());
    println!("active recipe : {} ({})", recipe.meta.name, recipe.meta.id);
    println!("provenance    : {}", app.recipe_source_label());
    println!("schema        : v{}", recipe.meta.version);
    println!("sources       :");
    for s in &recipe.sources {
        println!(
            "  {:<8} requires {:<24} label {}",
            s.id,
            if s.require_any.is_empty() {
                "(nothing)".to_string()
            } else {
                s.require_any.join(" | ")
            },
            s.label
        );
        for (kind, cmd) in [("search", &s.search), ("install", &s.install)] {
            if let Some(c) = cmd {
                println!("    {kind:<8} {}", c.argv.join(" "));
            }
        }
    }
    println!(
        "github        : enabled={} min_stars={}",
        recipe.github.enabled, recipe.github.min_stars
    );
    Ok(())
}
