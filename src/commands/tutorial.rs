//! `px --tutorial` — a 2-minute interactive walkthrough of everything px
//! can do. Pages of examples with next/prev, no side effects.

use crate::app::App;
use crate::error::PxResult;

struct Page {
    title: &'static str,
    body: Vec<String>,
}

fn pages() -> Vec<Page> {
    vec![
        Page {
            title: "installing packages",
            body: vec![
                "px checks every source your distro has — in parallel —",
                "and picks the best one. Repos beat the AUR, the AUR beats",
                "a source build.",
                "",
                "  px install neovim ripgrep sl",
                "",
                "typo protection built in:",
                "",
                "  px install firefxo    → did you mean: firefox?",
            ]
            .into_iter()
            .map(String::from)
            .collect(),
        },
        Page {
            title: "installing apps that aren't packages",
            body: vec![
                "some apps install through their own channel — npm globals",
                "like claude-code and codex, or curl|sh installers like bun.",
                "px knows them:",
                "",
                "  px install claude-code    → npm install -g @anthropic-ai/claude-code",
                "  px install bun            → curl -fsSL https://bun.sh/install | sh",
                "",
                "if something isn't in your repos, the AUR, or npm, px offers",
                "to find it on github and build it — always asking first.",
            ]
            .into_iter()
            .map(String::from)
            .collect(),
        },
        Page {
            title: "install for <project> — the magic one",
            body: vec![
                "point px at a project or script and it installs the system",
                "packages that project needs:",
                "",
                "  px install for ./my-app",
                "  px -i -> ./my-app           (same thing, arrow syntax)",
                "  px -i                       (interactive)",
                "",
                "python, node, c/c++, shell, go, java and ruby are detected;",
                "imports map to the right package names per distro",
                "(python-numpy on arch, python3-numpy on debian).",
                "",
                "  --local  → project-local env (venv/npm/bundle, no sudo)",
                "  --global → distro packages",
            ]
            .into_iter()
            .map(String::from)
            .collect(),
        },
        Page {
            title: "uninstalling and cleaning up",
            body: vec![
                "  px uninstall neovim         (through your distro's tools)",
                "",
                "px finds things you don't need anymore and offers to free",
                "the space:",
                "",
                "  px suggest",
                "",
                "  ✔ orphans — nothing depends on these, safe to remove",
                "  space hogs — your biggest explicit packages, with sizes",
                "",
                "caches clean themselves on every run.",
            ]
            .into_iter()
            .map(String::from)
            .collect(),
        },
        Page {
            title: "looking around",
            body: vec![
                "  px search ffmpeg        every source at once, ranked",
                "  px info neovim          merged info from every source",
                "  px list                 what px itself installed",
                "  px doctor               what px sees on this machine",
                "  px status               running px instance + resume state",
                "",
                "safety: px never runs as root, elevates only individual",
                "commands through sudo, shows the plan before changing",
                "anything, and --dry-run prints exact commands.",
                "",
                "suspicious packages (especially node ones) get an",
                "investigation offer before install.",
            ]
            .into_iter()
            .map(String::from)
            .collect(),
        },
        Page {
            title: "make it yours",
            body: vec![
                "  --bar rainbow      blocks | shades | rainbow | minimal | sparkles",
                "  -y / --yes         skip confirmations (never source builds)",
                "  --dry-run          print what would run",
                "  --recipe <file>    simulate another distro:",
                "",
                "    px --recipe recipes/debian.toml --dry-run install ffmpeg",
                "    → sudo apt-get install -y ffmpeg",
                "",
                "jokes stream in during long installs. you're welcome.",
                "",
                "that's px. go break something — px will suggest a cleanup.",
            ]
            .into_iter()
            .map(String::from)
            .collect(),
        },
    ]
}

pub fn run(app: App) -> PxResult<()> {
    let style = &app.style;
    println!("{}", style.banner());
    println!();

    if !crate::ui::interactive() {
        // Non-interactive: print the whole thing.
        for (i, page) in pages().iter().enumerate() {
            println!(
                "{}  {}",
                style.header(&format!("{:>2}. {}", i + 1, page.title)),
                style.dim("(cont'd)")
            );
            for line in &page.body {
                println!("  {line}");
            }
            println!();
        }
        return Ok(());
    }

    let pages = pages();
    let mut i = 0;
    loop {
        let page = &pages[i];
        println!(
            "\n{} {}\n",
            style.header(&format!("[{}/{}]", i + 1, pages.len())),
            style.bold(page.title)
        );
        for line in &page.body {
            println!("  {line}");
        }
        println!();

        let mut items = vec!["next".to_string()];
        if i > 0 {
            items.push("previous".to_string());
        }
        if i + 1 < pages.len() {
            items.push("exit".to_string());
        } else {
            items[0] = "finish".to_string();
        }
        match crate::ui::prompt::select("px tutorial", &items)? {
            0 if i + 1 < pages.len() => i += 1,
            0 => break,
            1 if i > 0 && i + 1 < pages.len() => i -= 1,
            _ => break,
        }
    }
    println!(
        "{} you know px now. try: {}",
        style.ok("✔"),
        style.bold("px install for .")
    );
    Ok(())
}
