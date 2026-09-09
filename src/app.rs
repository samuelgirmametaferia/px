//! Command orchestration — the glue between the CLI and the modules.

use crate::commands;

use std::sync::Arc;

use crate::backend::source::SourceProvider;
use crate::backend::{Installer, Provider};
use crate::cli::{CacheCmd, Cli, Command, RecipeCmd};
use crate::config::Config;
use crate::error::{PxError, PxResult};
use crate::exec::{Executor, RealExecutor};
use crate::recipe::load::{self, LoadedRecipe, RecipeSource};
use crate::recipe::schema::Recipe;
use crate::ui::style::Style;

pub struct App {
    pub cli: Cli,
    pub config: Config,
    pub style: Style,
    pub recipe: LoadedRecipe,
    pub client: reqwest::Client,
    pub exec: Arc<dyn Executor>,
}

impl App {
    pub async fn init(cli: Cli) -> PxResult<App> {
        let config = Config::load();
        let style = Style::new(crate::ui::colors_enabled(cli.no_color));

        let client = reqwest::Client::builder()
            .user_agent(concat!("px/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(PxError::Network)?;

        // Detection: bundled recipes → pick the one matching this machine.
        let os = crate::recipe::detect::OsRelease::load();
        let bundled = load::bundled_recipes();
        let detected = crate::recipe::detect::detect(&bundled, &os);

        // An explicit --recipe overrides detection; otherwise use detected id
        // (falling back to "arch" so px still boots on unknown distros).
        let recipe_id = if cli.recipe.is_some() {
            String::new() // loaded from path; id comes from the file
        } else {
            detected
                .as_ref()
                .map(|(r, _)| r.meta.id.clone())
                .unwrap_or_else(|| "arch".into())
        };

        let recipe = load::load(
            cli.recipe.as_deref(),
            &recipe_id,
            &config.recipe_repo(),
            &client,
            cli.refresh,
            false,
        )
        .await?;

        let exec: Arc<dyn Executor> = Arc::new(RealExecutor::new());

        Ok(App {
            cli,
            config,
            style,
            recipe,
            client,
            exec,
        })
    }

    pub fn recipe(&self) -> &Recipe {
        &self.recipe.recipe
    }

    /// Active providers + installers, in recipe priority order. An explicit
    /// --recipe forces all sources active (distro simulation).
    pub fn providers(&self) -> Vec<Arc<dyn Provider>> {
        let force = self.cli.recipe.is_some();
        let (active, _) =
            crate::backend::activate_sources_opt(self.recipe().sources.as_slice(), force);
        active
            .into_iter()
            .map(|src| {
                Arc::new(SourceProvider::new(src, Arc::clone(&self.exec))) as Arc<dyn Provider>
            })
            .collect()
    }

    pub fn installers(&self) -> Vec<Arc<dyn Installer>> {
        let force = self.cli.recipe.is_some();
        let (active, _) =
            crate::backend::activate_sources_opt(self.recipe().sources.as_slice(), force);
        active
            .into_iter()
            .map(|src| {
                Arc::new(SourceProvider::new(src, Arc::clone(&self.exec))) as Arc<dyn Installer>
            })
            .collect()
    }

    pub fn recipe_source_label(&self) -> String {
        match &self.recipe.source {
            RecipeSource::Explicit(p) => format!("explicit ({})", p.display()),
            RecipeSource::Cached { age } => {
                format!("cached ({}h old)", age.as_secs() / 3600)
            }
            RecipeSource::Fetched => "fetched from GitHub".into(),
            RecipeSource::Bundled => "bundled with px binary".into(),
        }
    }

    pub async fn run(self) -> PxResult<()> {
        let Some(command) = self.cli.command.clone() else {
            // Bare `px` or `px -i` with no subcommand → interactive install.
            return commands::install::interactive(self).await;
        };
        match command {
            Command::Install { specs } => {
                if specs.len() >= 2 && specs[0] == "for" {
                    let path = specs[1..].join(" ");
                    commands::install_for::run(self, &path).await
                } else if specs.is_empty() {
                    commands::install::interactive(self).await
                } else {
                    commands::install::run(self, &specs).await
                }
            }
            Command::Search { term } => commands::search::run(self, &term.join(" ")).await,
            Command::Info { name } => commands::info::run(self, &name).await,
            Command::List => commands::list::run(self),
            Command::Doctor => commands::doctor::run(self),
            Command::Recipe { cmd } => match cmd {
                RecipeCmd::List => commands::recipe::list(self),
                RecipeCmd::Show => commands::recipe::show(self),
            },
            Command::Cache { cmd } => match cmd {
                CacheCmd::Clean {
                    search,
                    recipes,
                    builds,
                    all,
                } => commands::cache::clean(self, search, recipes, builds, all),
            },
        }
    }
}
