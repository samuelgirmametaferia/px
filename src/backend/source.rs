//! The generic, recipe-driven source: this one module is how px supports
//! every distro. It takes a [[sources]] entry and implements search/info/
//! installed/install purely by running the recipe's commands.

use std::sync::Arc;

use async_trait::async_trait;

use crate::backend::{ActiveSource, InstallCtx, Installer, PackageHit, Provider};
use crate::error::{PxError, PxResult};
use crate::exec::{Executor, RunOpts, expand_argv};
use crate::recipe::schema::CommandDef;

pub struct SourceProvider {
    src: ActiveSource,
    exec: Arc<dyn Executor>,
}

impl SourceProvider {
    pub fn new(src: ActiveSource, exec: Arc<dyn Executor>) -> Self {
        SourceProvider { src, exec }
    }

    fn helper(&self) -> Option<&str> {
        if self.src.helper.is_empty() {
            None
        } else {
            Some(&self.src.helper)
        }
    }

    /// Expand + run one recipe command, returning stdout.
    async fn run(&self, cmd: &CommandDef, pkg: &str) -> PxResult<String> {
        let argv = expand_argv(&cmd.argv, pkg, &[pkg.to_string()], self.helper(), None);
        let out = self.exec.run(&argv, RunOpts::default()).await?;
        Ok(out.stdout)
    }

    fn search_def(&self) -> PxResult<&CommandDef> {
        self.src.def.search.as_ref().ok_or_else(|| {
            PxError::Recipe(format!(
                "source '{}' defines no search command",
                self.src.def.id
            ))
        })
    }
}

#[async_trait]
impl Provider for SourceProvider {
    fn source_id(&self) -> &str {
        &self.src.def.id
    }

    fn label(&self) -> &str {
        &self.src.def.label
    }

    async fn search(&self, query: &str) -> PxResult<Vec<PackageHit>> {
        let def = self.search_def()?;
        let stdout = self.run(def, query).await?;
        let parse = def.parse.as_deref().unwrap_or("pacman_search");
        Ok(crate::backend::parsers::parse_output(
            parse,
            &stdout,
            &self.src.def.id,
        ))
    }

    async fn info(&self, exact: &str) -> PxResult<Option<PackageHit>> {
        let Some(def) = &self.src.def.info else {
            return Ok(None);
        };
        // Cross-run disk cache: info checks hit pacman/zypper subprocesses
        // and repeat constantly across runs on the same project.
        let key = format!("info:{}:{}", self.src.def.id, exact);
        if let Some(cached) =
            crate::cache::get("provider", &key, std::time::Duration::from_secs(3600))
        {
            if cached == "miss" {
                return Ok(None);
            }
            if let Ok(hit) = serde_json::from_str::<PackageHit>(&cached) {
                return Ok(Some(hit));
            }
        }
        let stdout = self.run(def, exact).await?;
        let parse = def.parse.as_deref().unwrap_or("pacman_info");
        let hit = crate::backend::parsers::parse_output(parse, &stdout, &self.src.def.id)
            .into_iter()
            .find(|h| h.name == exact);
        let cacheable = match &hit {
            Some(h) => serde_json::to_string(h).unwrap_or_default(),
            None => "miss".to_string(),
        };
        crate::cache::put("provider", &key, &cacheable);
        Ok(hit)
    }

    async fn is_installed(&self, name: &str) -> PxResult<bool> {
        let Some(def) = &self.src.def.installed else {
            return Ok(false);
        };
        // Installed state changes when px itself installs something, so
        // only cache the negative answer briefly; cache "installed" longer.
        let key = format!("installed:{}:{}", self.src.def.id, name);
        if let Some(cached) =
            crate::cache::get("provider", &key, std::time::Duration::from_secs(300))
            && let Ok(v) = cached.parse::<bool>() {
                return Ok(v);
            }
        let argv = expand_argv(&def.argv, name, &[name.to_string()], self.helper(), None);
        let out = self.exec.run(&argv, RunOpts::default()).await?;
        let installed = out.success();
        crate::cache::put("provider", &key, &installed.to_string());
        Ok(installed)
    }
}

#[async_trait]
impl Installer for SourceProvider {
    async fn install(&self, pkgs: &[String], ctx: InstallCtx) -> PxResult<()> {
        let Some(def) = &self.src.def.install else {
            return Err(PxError::Recipe(format!(
                "source '{}' defines no install command",
                self.src.def.id
            )));
        };
        if def.elevated && !ctx.dry_run {
            crate::backend::elevate::preflight().await?;
        }
        let pkg = pkgs.first().cloned().unwrap_or_default();
        let argv = expand_argv(&def.argv, &pkg, pkgs, self.helper(), None);
        let out = self
            .exec
            .run(
                &argv,
                RunOpts {
                    inherit: true,
                    dry_run: ctx.dry_run,
                    ..Default::default()
                },
            )
            .await?;
        if !out.success() {
            return Err(PxError::Command {
                cmd: argv.join(" "),
                stderr: if out.stderr.is_empty() {
                    "(see output above)".into()
                } else {
                    out.stderr
                },
            });
        }
        Ok(())
    }
}
