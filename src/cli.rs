use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser, Debug, Clone)]
#[command(
    name = "px",
    version,
    about = "px — the universal package-manager front-end",
    long_about = "px installs anything on any distro by driving the package managers\n\
                  your system already has, defined in per-distro recipes.\n\n\
                  Install packages:      px install neovim ripgrep\n\
                  Analyze a project:     px install for ./my-project\n\
                  Interactive:           px -i  (then type a path)\n\
                  Search everywhere:     px search <term>"
)]
pub struct Cli {
    /// Assume yes for confirmations (never auto-confirms source builds).
    #[arg(short = 'y', long, global = true)]
    pub yes: bool,

    /// Print what would run, change nothing.
    #[arg(long, global = true)]
    pub dry_run: bool,

    /// Disable the GitHub source-build fallback for this run.
    #[arg(long = "no-source", global = true)]
    pub no_source: bool,

    /// Force recipe re-fetch from GitHub.
    #[arg(long, global = true)]
    pub refresh: bool,

    /// Explicit recipe file (also the distro-simulation lever:
    /// `px --recipe recipes/debian.toml --dry-run install ffmpeg`).
    #[arg(long, global = true)]
    pub recipe: Option<PathBuf>,

    /// Disable colors.
    #[arg(long = "no-color", global = true)]
    pub no_color: bool,

    /// Verbosity: -v info, -vv debug.
    #[arg(short = 'v', action = clap::ArgAction::Count, global = true)]
    pub verbose: u8,

    /// Install mode for `install for`: project-local env vs system packages.
    #[arg(long, global = true, conflicts_with = "global")]
    pub local: bool,

    /// Install system packages (default for `install for` when not asking).
    #[arg(long, global = true)]
    pub global: bool,

    /// Progress bar style for installs.
    #[arg(long, global = true, value_parser = bar_style_parser, default_value = "shades")]
    pub bar: String,

    /// Run the interactive tutorial.
    #[arg(long, global = true)]
    pub tutorial: bool,

    #[command(subcommand)]
    pub command: Option<Command>,
}

fn bar_style_parser(s: &str) -> Result<String, String> {
    if crate::ui::progress::BarStyle::parse(s).is_some() {
        Ok(s.to_string())
    } else {
        Err(format!(
            "unknown bar style '{s}' (try: {})",
            crate::ui::progress::BarStyle::names()
        ))
    }
}

#[derive(Subcommand, Debug, Clone)]
pub enum Command {
    /// Install packages, or everything a project needs.
    Install {
        /// Package specs. `for <PATH>` switches to project analysis.
        specs: Vec<String>,
    },

    /// Uninstall packages through your distro's own tools.
    Uninstall { specs: Vec<String> },

    /// Upgrade every package on the system (pacman -Syu / apt upgrade / ...).
    Upgrade,

    /// Generate shell completions (bash, zsh, fish, ...).
    Completions { shell: String },

    /// Find unused/unneeded packages and offer to remove them (free space).
    Suggest,

    /// Running px instance, interrupted installs, cache state.
    Status,

    /// Search every active source in parallel.
    Search { term: Vec<String> },

    /// Show merged info for a package.
    Info { name: String },

    /// List packages px has installed.
    List,

    /// Show recipe status, tools, caches — what px sees on this machine.
    Doctor,

    /// Learn px interactively.
    Tutorial,

    /// Inspect recipes.
    Recipe {
        #[command(subcommand)]
        cmd: RecipeCmd,
    },

    /// Manage px's caches.
    Cache {
        #[command(subcommand)]
        cmd: CacheCmd,
    },
}

#[derive(Subcommand, Debug, Clone)]
pub enum RecipeCmd {
    /// List shipped/cached recipes and their status.
    List,
    /// Show the active recipe and where it came from.
    Show,
}

#[derive(Subcommand, Debug, Clone)]
pub enum CacheCmd {
    /// Clean caches: --search, --recipes, --builds, or everything.
    Clean {
        #[arg(long)]
        search: bool,
        #[arg(long)]
        recipes: bool,
        #[arg(long)]
        builds: bool,
        #[arg(long)]
        all: bool,
    },
}
