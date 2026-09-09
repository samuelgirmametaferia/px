//! Every external command px runs goes through an Executor.
//!
//! `--dry-run` flips the RealExecutor into print mode: it logs the exact
//! argv it would run and returns success — which makes the entire install
//! path inspectable on any distro, even ones this machine doesn't have.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;

use async_trait::async_trait;
use tokio::process::Command;

use crate::error::{PxError, PxResult};

#[derive(Debug, Clone, Default)]
pub struct RunOpts {
    /// Inherit stdio (child talks to the user directly: sudo prompts,
    /// pacman output) instead of capturing it.
    pub inherit: bool,
    pub cwd: Option<PathBuf>,
    /// Extra env vars (e.g. MAKEFLAGS).
    pub env: HashMap<String, String>,
    /// When true, print the exact argv instead of running it. Used for
    /// mutating commands under --dry-run; searches always run for real.
    pub dry_run: bool,
}

#[derive(Debug, Clone, Default)]
pub struct ExecOutput {
    pub status: i32,
    pub stdout: String,
    pub stderr: String,
}

impl ExecOutput {
    pub fn success(&self) -> bool {
        self.status == 0
    }
}

#[async_trait]
pub trait Executor: Send + Sync {
    async fn run(&self, argv: &[String], opts: RunOpts) -> PxResult<ExecOutput>;
}

/// The real thing. Pass `RunOpts::dry_run` per call: reads (search, info,
/// installed checks) always execute; mutations can be walked in print mode.
pub struct RealExecutor;

/// Resolve a binary to its absolute path — bypasses shell aliases (a fish
/// `sudo` alias must never change what px executes). Falls back to the bare
/// name (PATH lookup) when `which` can't find it, so error messages still
/// make sense.
pub fn resolve_bin(name: &str) -> String {
    match which::which(name) {
        Ok(path) => path.to_string_lossy().into_owned(),
        Err(_) => name.to_string(),
    }
}

/// Expand recipe argv with placeholders. Every expansion becomes its own
/// argv entry — args are passed verbatim to the child, never through a shell.
///
///   {pkg}      — single package name (one arg)
///   {pkgs...}  — N args
///   {helper}   — the matched helper binary (paru/yay)
///   {file}     — single file path
pub fn expand_argv(
    template: &[String],
    pkg: &str,
    pkgs: &[String],
    helper: Option<&str>,
    file: Option<&str>,
) -> Vec<String> {
    let mut out = Vec::with_capacity(template.len() + pkgs.len());
    for tok in template {
        match tok.as_str() {
            "{pkg}" => out.push(pkg.to_string()),
            "{pkgs...}" => out.extend(pkgs.iter().cloned()),
            "{helper}" => out.push(helper.map(|h| h.to_string()).unwrap_or_else(|| tok.clone())),
            "{file}" => out.push(file.map(|f| f.to_string()).unwrap_or_else(|| tok.clone())),
            _ => out.push(tok.clone()),
        }
    }
    out
}

impl RealExecutor {
    pub fn new() -> Self {
        RealExecutor
    }
}

impl Default for RealExecutor {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Executor for RealExecutor {
    async fn run(&self, argv: &[String], opts: RunOpts) -> PxResult<ExecOutput> {
        if argv.is_empty() {
            return Err(PxError::User("empty command".into()));
        }

        // Resolve every token that looks like a bare command name to an
        // absolute path where possible (notably `sudo`), so shell aliases
        // and functions can never change what px runs.
        let mut resolved: Vec<String> = Vec::with_capacity(argv.len());
        for (i, tok) in argv.iter().enumerate() {
            if i == 0 || tok == "sudo" {
                resolved.push(resolve_bin(tok));
            } else {
                resolved.push(tok.clone());
            }
        }

        if opts.dry_run {
            tracing::info!("[dry-run] {}", resolved.join(" "));
            let prefix = if std::io::IsTerminal::is_terminal(&std::io::stdout()) {
                "\x1b[1;33m[dry-run]\x1b[0m"
            } else {
                "[dry-run]"
            };
            println!("{prefix} {}", resolved.join(" "));
            return Ok(ExecOutput {
                status: 0,
                stdout: String::new(),
                stderr: String::new(),
            });
        }

        let mut cmd = Command::new(&resolved[0]);
        cmd.args(&resolved[1..]);
        if let Some(cwd) = &opts.cwd {
            cmd.current_dir(cwd);
        }
        for (k, v) in &opts.env {
            cmd.env(k, v);
        }

        if opts.inherit {
            crate::ui::prompt::flush();
            let status = cmd.status().await.map_err(|e| PxError::Command {
                cmd: resolved.join(" "),
                stderr: e.to_string(),
            })?;
            Ok(ExecOutput {
                status: status.code().unwrap_or(-1),
                stdout: String::new(),
                stderr: String::new(),
            })
        } else {
            let out = cmd.output().await.map_err(|e| PxError::Command {
                cmd: resolved.join(" "),
                stderr: e.to_string(),
            })?;
            Ok(ExecOutput {
                status: out.status.code().unwrap_or(-1),
                stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
                stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
            })
        }
    }
}

/// Records every call and returns canned responses — used by tests to pin
/// exact argv generation for distros this machine doesn't run.
#[derive(Default)]
pub struct MockExecutor {
    pub calls: Mutex<Vec<Vec<String>>>,
    pub responses: Mutex<std::collections::VecDeque<ExecOutput>>,
}

impl MockExecutor {
    pub fn with_responses(responses: Vec<ExecOutput>) -> Self {
        MockExecutor {
            calls: Mutex::new(Vec::new()),
            responses: Mutex::new(responses.into()),
        }
    }
}

#[async_trait]
impl Executor for MockExecutor {
    async fn run(&self, argv: &[String], _opts: RunOpts) -> PxResult<ExecOutput> {
        self.calls.lock().unwrap().push(argv.to_vec());
        Ok(self
            .responses
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_default())
    }
}

/// Convenience for tests / callers: run one command, capture output.
pub async fn run_capture(exec: &dyn Executor, argv: &[String]) -> PxResult<ExecOutput> {
    exec.run(argv, RunOpts::default()).await
}
