//! The px sandbox: risky install steps (npm packages with install scripts,
//! curl|sh installer scripts, GitHub source builds) run inside a bubblewrap
//! container with the SYSTEM read-only. A malicious install script can
//! touch the project and home (it has to install *somewhere*) but cannot
//! modify /usr, /etc, /boot or persist anything system-wide. System package
//! installs (pacman/apt/dnf/zypper) are NOT sandboxed — mutating the system
//! is their job and they're guarded by sudo instead.
//!
//! Mode is data: config `sandbox = "auto"|"on"|"off"` or `--sandbox` /
//! `--no-sandbox`. auto = on when bwrap exists.

use std::sync::Arc;

use crate::app::App;
use crate::error::PxError;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SandboxMode {
    Auto,
    On,
    Off,
}

impl SandboxMode {
    pub fn parse(s: &str) -> Option<SandboxMode> {
        match s {
            "auto" => Some(SandboxMode::Auto),
            "on" => Some(SandboxMode::On),
            "off" => Some(SandboxMode::Off),
            _ => None,
        }
    }
}

pub fn bwrap_available() -> bool {
    which::which("bwrap").is_ok()
}

/// Effective decision for this run.
pub fn enabled(app: &App) -> bool {
    if app.cli.sandbox {
        return true;
    }
    if app.cli.no_sandbox {
        return false;
    }
    match SandboxMode::parse(&app.config.sandbox) {
        Some(SandboxMode::On) => true,
        Some(SandboxMode::Off) => false,
        _ => bwrap_available(), // auto
    }
}

/// Wrap argv in a bubblewrap invocation that keeps the system read-only:
///   - / mounted read-only (nothing outside home/tmp/build dir is writable)
///   - fresh /proc, /dev, tmpfs /tmp (isolated pid + ipc namespaces)
///   - $HOME writable (installers must install somewhere)
///   - network kept: downloads are the point
pub fn wrap_argv(argv: &[String], extra_writable: &[String]) -> Vec<String> {
    let mut wrapped = vec![crate::exec::resolve_bin("bwrap")];
    wrapped.push("--ro-bind".into());
    wrapped.push("/".into());
    wrapped.push("/".into());
    wrapped.push("--proc".into());
    wrapped.push("/proc".into());
    wrapped.push("--dev".into());
    wrapped.push("/dev".into());
    wrapped.push("--tmpfs".into());
    wrapped.push("/tmp".into());
    wrapped.push("--unshare-ipc".into());
    wrapped.push("--unshare-pid".into());
    if let Some(home) = dirs::home_dir() {
        wrapped.push("--bind".into());
        wrapped.push(home.to_string_lossy().into_owned());
        wrapped.push(home.to_string_lossy().into_owned());
    }
    for dir in extra_writable {
        wrapped.push("--bind".into());
        wrapped.push(dir.clone());
        wrapped.push(dir.clone());
    }
    // env passthrough so npm/cargo/go find their config and caches
    wrapped.push("--clearenv".into());
    for key in [
        "PATH",
        "HOME",
        "USER",
        "SHELL",
        "TERM",
        "TMPDIR",
        "XDG_CONFIG_HOME",
        "XDG_CACHE_HOME",
        "npm_config_prefix",
    ] {
        if let Ok(v) = std::env::var(key) {
            wrapped.push("--setenv".into());
            wrapped.push(key.into());
            wrapped.push(v);
        }
    }
    wrapped.extend(argv.iter().cloned());
    wrapped
}

/// Run a command sandboxed (or plainly, with a warning, when bwrap is
/// missing and mode demands sandboxing). Returns the executor's output.
pub async fn run_sandboxed(
    app: &App,
    argv: &[String],
    extra_writable: &[String],
) -> crate::error::PxResult<crate::exec::ExecOutput> {
    let exec: Arc<dyn crate::exec::Executor> = Arc::clone(&app.exec);
    if !enabled(app) {
        return exec.run(argv, crate::exec::RunOpts::default()).await;
    }
    if !bwrap_available() {
        eprintln!(
            "{}  sandbox requested but bwrap is not installed — running unsandboxed (px install bubblewrap to fix)",
            app.style.warn("⚠")
        );
        return exec.run(argv, crate::exec::RunOpts::default()).await;
    }
    let wrapped = wrap_argv(argv, extra_writable);
    exec.run(&wrapped, crate::exec::RunOpts::default()).await
}

/// True when a sandboxed run should be attempted even though px can't
/// verify the outcome — used by callers to decide on error reporting.
pub fn ensure_available_or_error() -> crate::error::PxResult<()> {
    if bwrap_available() {
        Ok(())
    } else {
        Err(PxError::User(
            "sandboxing requested but bwrap is not installed (px install bubblewrap)".into(),
        ))
    }
}
