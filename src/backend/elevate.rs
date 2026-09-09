//! sudo preflight: validate/cached credentials ONCE, before any long-running
//! work, so the user is never prompted mid-build. Only Linux/sudo recipes
//! ever call this (recipes with no `elevated` commands skip it entirely).

use crate::error::{PxError, PxResult};

/// Check that elevation is available without blocking later.
/// - interactive tty: `sudo -v` (prompts once, caches credentials)
/// - non-tty: `sudo -n true` (must be passwordless or already cached)
///
/// ALL live drawing is suspended first and the user is told a password may
/// be asked: a sudo prompt drawn behind a spinner is invisible, and an
/// invisible prompt is indistinguishable from a hang.
pub async fn preflight() -> PxResult<()> {
    if !nix_like() {
        return Ok(());
    }

    let sudo = crate::exec::resolve_bin("sudo");
    if which::which("sudo").is_err() {
        return Err(PxError::User(
            "this command needs sudo, but sudo is not installed".into(),
        ));
    }

    // Fast path: credentials already cached / passwordless — no prompt.
    let cached = tokio::process::Command::new(&sudo)
        .args(["-n", "true"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .await;
    if cached.map(|s| s.success()).unwrap_or(false) {
        return Ok(());
    }

    let interactive = std::io::IsTerminal::is_terminal(&std::io::stdin());
    let mut argv: Vec<String> = vec![sudo];
    if interactive {
        argv.push("-v".into());
    } else {
        argv.push("-n".into());
        argv.push("true".into());
    }

    crate::ui::prompt::flush();
    crate::ui::spinner::suspend_all();
    if interactive {
        eprintln!("px needs sudo — you may be asked for your password");
    }
    let out = tokio::process::Command::new(&argv[0])
        .args(&argv[1..])
        .kill_on_drop(true)
        .status()
        .await
        .map_err(|e| PxError::User(format!("cannot run sudo: {e}")));
    crate::ui::spinner::resume_all();
    let out = out?;

    if out.success() {
        Ok(())
    } else if interactive {
        Err(PxError::User("sudo authentication failed".into()))
    } else {
        Err(PxError::User(
            "px needs sudo in this session but cannot prompt — run px in a terminal, \
             or configure passwordless sudo for the install command"
                .into(),
        ))
    }
}

fn nix_like() -> bool {
    cfg!(target_os = "linux") || cfg!(target_os = "macos")
}

/// px must never run as root itself (children like makepkg refuse root, and
/// building as root is dangerous).
pub fn refuse_root() -> PxResult<()> {
    if cfg!(unix) {
        let uid = unsafe { libc_uid() };
        if uid == 0 {
            return Err(PxError::User(
                "px refuses to run as root — run it as your normal user; \
                 it elevates individual commands via sudo when needed"
                    .into(),
            ));
        }
    }
    Ok(())
}

#[cfg(unix)]
unsafe fn libc_uid() -> u32 {
    // Avoid a libc dependency just for getuid: read it from /proc on Linux,
    // fall back to the `id -u` child process elsewhere.
    #[cfg(target_os = "linux")]
    {
        if let Ok(status) = std::fs::read_to_string("/proc/self/status") {
            for line in status.lines() {
                if let Some(rest) = line.strip_prefix("Uid:")
                    && let Some(uid) = rest.split_whitespace().next()
                    && let Ok(n) = uid.parse::<u32>()
                {
                    return n;
                }
            }
        }
        1000 // could not determine; assume non-root
    }
    #[cfg(not(target_os = "linux"))]
    {
        1000
    }
}
