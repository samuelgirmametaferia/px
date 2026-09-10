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

    /// Expand + run one recipe command, returning stdout. Queries get a hard
    /// timeout — a wedged package manager must never hang px.
    async fn run(&self, cmd: &CommandDef, pkg: &str) -> PxResult<String> {
        let argv = expand_argv(&cmd.argv, pkg, &[pkg.to_string()], self.helper(), None);
        let out = self
            .exec
            .run(
                &argv,
                RunOpts {
                    timeout: Some(std::time::Duration::from_secs(20)),
                    ..Default::default()
                },
            )
            .await?;
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
            && let Ok(v) = cached.parse::<bool>()
        {
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
        // sudo credentials are cached by the preflight, so the install
        // command itself shouldn't need the terminal — EXCEPT AUR-style
        // helpers (yay/paru): they invoke sudo again mid-build, and
        // without a tty that sudo dies ("timed out reading password").
        // NOT "has a helper" — every source's require_any match becomes
        // one (pacman too, which made ALL installs noisy). The argv shape
        // is the truth: a helper that IS argv[0] owns its own stdio; a
        // package manager behind a sudo prefix stays captured and quiet.
        let is_helper_source = def
            .argv
            .first()
            .map(|first| Some(first.as_str()) == self.helper())
            .unwrap_or(false);
        if def.elevated && !ctx.dry_run {
            crate::backend::elevate::preflight().await?;
        }
        let pkg = pkgs.first().cloned().unwrap_or_default();
        let argv = expand_argv(&def.argv, &pkg, pkgs, self.helper(), None);
        // px owns the display: package-manager output is captured (quiet),
        // except with -v — or for helpers, which own their own progress.
        let out = self
            .exec
            .run(
                &argv,
                RunOpts {
                    inherit: ctx.verbose || is_helper_source,
                    dry_run: ctx.dry_run,
                    ..Default::default()
                },
            )
            .await?;
        if !out.success() {
            // Failure: surface what the package manager actually said —
            // the last lines carry the real error.
            print_output_tail(&argv.join(" "), &out);
            return Err(PxError::Command {
                cmd: argv.join(" "),
                stderr: "(see output above)".into(),
            });
        }
        Ok(())
    }
}

/// Show the tail of a failed command's output (stdout+stderr interleaved,
/// last 25 lines) — enough to see the real error without the flood.
/// Recipe commands can drift (package-manager flag changes) — failures
/// always hint at pulling fresh recipes from the web.
fn print_output_tail(cmd: &str, out: &crate::exec::ExecOutput) {
    let combined = format!("{}{}", out.stdout, out.stderr);
    if combined.trim().is_empty() {
        return;
    }
    let lines: Vec<&str> = combined.lines().collect();
    let start = lines.len().saturating_sub(25);
    eprintln!("  ── last lines of: {cmd} ──");
    for line in &lines[start..] {
        eprintln!("  {line}");
    }
    eprintln!(
        "  ── if the command looks wrong for your package manager, `px update` pulls fresh recipes ──"
    );
}
