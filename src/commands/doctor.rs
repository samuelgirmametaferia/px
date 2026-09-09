//! `px doctor` — what px sees: distro, recipe provenance, active and
//! inactive sources, tools, caches.

use crate::app::App;
use crate::error::PxResult;

pub fn run(app: App) -> PxResult<()> {
    let style = &app.style;
    println!("{}", style.banner());
    println!();

    // --- system -----------------------------------------------------------
    let os = crate::recipe::detect::OsRelease::load();
    println!("{}", style.header("system"));
    println!("  os        {}", style.value(&os.pretty()));
    if let Some(id) = os.id() {
        println!("  id        {id}");
    }
    if !os.id_like().is_empty() {
        println!("  id_like   {}", os.id_like().join(" "));
    }
    println!(
        "  sudo      {}",
        if which::which("sudo").is_ok() {
            style.ok("available")
        } else {
            style.warn("missing")
        }
    );
    println!("  git       {}", tool_status("git"));
    // sandbox: what guards npm / install.sh / source-build steps
    if crate::sandbox::bwrap_available() {
        println!(
            "  sandbox   {}",
            style.ok("bwrap — npm, install.sh and source builds run with the system read-only")
        );
    } else {
        println!(
            "  sandbox   {}",
            style.warn("bwrap missing — risky installs run unsandboxed (px install bubblewrap)")
        );
    }
    println!();

    // --- recipe ------------------------------------------------------------
    let recipe = app.recipe();
    println!("{}", style.header("recipe"));
    println!(
        "  active    {} ({})",
        style.bold(&recipe.meta.name),
        recipe.meta.id
    );
    println!("  source    {}", app.recipe_source_label());
    println!("  sources   (in priority order)");
    let (active, inactive) = crate::backend::activate_sources(&recipe.sources);
    for src in &active {
        let parsers_ok = crate::backend::parsers::parsers_known(&src.def);
        let helper = if src.helper.is_empty() {
            String::new()
        } else {
            format!(" via {}", src.helper)
        };
        println!(
            "    {} {}{helper}{}",
            style.ok("✔"),
            style.bold(&src.def.id),
            if parsers_ok {
                style.dim("")
            } else {
                style.warn("  (unknown parser!)")
            }
        );
    }
    for id in &inactive {
        let def = recipe.sources.iter().find(|s| &s.id == id);
        let needs = def.map(|d| d.require_any.join(" or ")).unwrap_or_default();
        println!(
            "    {} {} — inactive (needs {needs})",
            style.warn("○"),
            style.bold(id)
        );
    }
    println!(
        "  github    {}",
        if recipe.github.enabled {
            format!(
                "enabled (source-build fallback, min ★{})",
                recipe.github.min_stars
            )
        } else {
            "disabled".to_string()
        }
    );
    println!();

    // --- ecosystems ---------------------------------------------------------
    println!("{}", style.header("ecosystems (install for)"));
    let mut ecos: Vec<&String> = recipe.ecosystems.keys().collect();
    ecos.sort();
    for eco in ecos {
        let cfg = recipe.ecosystem(eco);
        let state = if cfg.map(|c| c.enabled).unwrap_or(false) {
            style.ok("✔")
        } else {
            style.dim("·")
        };
        let tools = cfg.map(|c| c.tools.join(" ")).unwrap_or_default();
        println!("  {state} {:<8} {}", style.bold(eco), style.dim(&tools));
    }
    println!();

    // --- caches ---------------------------------------------------------------
    println!("{}", style.header("caches"));
    let root = crate::cache::cache_root();
    println!("  root      {}", root.display());
    for ns in ["recipes", "search", "builds"] {
        let dir = root.join(ns);
        let count = std::fs::read_dir(&dir).map(|d| d.count()).unwrap_or(0);
        println!(
            "  {ns:<9} {}",
            if count > 0 {
                style.value(&format!("{count} entries"))
            } else {
                style.dim("empty")
            }
        );
    }
    println!();

    // --- other recipes ---------------------------------------------------------
    println!("{}", style.header("shipped recipes"));
    for (id, _) in crate::recipe::load::bundled::all() {
        let state = if id == recipe.meta.id {
            style.ok("active")
        } else {
            style.dim("available")
        };
        println!("  {:<8} {state}", style.bold(id));
    }

    Ok(())
}

fn tool_status(bin: &str) -> String {
    if which::which(bin).is_ok() {
        crate::ui::style::Style::new(crate::ui::colors_enabled(false)).ok("available")
    } else {
        crate::ui::style::Style::new(crate::ui::colors_enabled(false)).warn("missing")
    }
}
