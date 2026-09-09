//! px runtime state: the instance lock (`px status`) and the install
//! journal that lets an interrupted px resume exactly where it stopped.
//!
//! ~/.local/state/px/
//!   lock        — pid + command of the running px (removed on exit)
//!   journal.json— the pending install plan while one is executing
//!   welcomed    — first-run animation marker (ui::spectrum)

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

pub fn state_dir() -> PathBuf {
    dirs::state_dir()
        .unwrap_or_else(|| {
            dirs::data_dir()
                .unwrap_or_else(|| PathBuf::from(".local/share"))
                .join("state")
        })
        .join("px")
}

fn lock_path() -> PathBuf {
    state_dir().join("lock")
}

fn journal_path() -> PathBuf {
    state_dir().join("journal.json")
}

// ------------------------------------------------------------------- lock

#[derive(Debug, Serialize, Deserialize)]
pub struct LockInfo {
    pub pid: u32,
    pub command: String,
    pub started: String,
}

fn pid_alive(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    #[cfg(target_os = "linux")]
    {
        std::path::Path::new(&format!("/proc/{pid}")).exists()
    }
    #[cfg(not(target_os = "linux"))]
    {
        // kill(pid, 0) semantics without a libc dependency.
        #[cfg(unix)]
        unsafe {
            unsafe extern "C" {
                fn kill(pid: i32, sig: i32) -> i32;
            }
            kill(pid as i32, 0) == 0
        }
        #[cfg(not(unix))]
        {
            true // can't tell; assume alive
        }
    }
}

/// Write the instance lock. A live previous lock is reported but NOT
/// blocked — package managers serialize themselves — px just tells you.
pub fn acquire_lock(command: &str) -> Option<LockInfo> {
    let _ = std::fs::create_dir_all(state_dir());
    let previous = read_lock();
    let lock = LockInfo {
        pid: std::process::id(),
        command: command.to_string(),
        started: chrono::Utc::now().to_rfc3339(),
    };
    let _ = std::fs::write(
        lock_path(),
        serde_json::to_string(&lock).unwrap_or_default(),
    );
    previous
}

pub fn read_lock() -> Option<LockInfo> {
    let text = std::fs::read_to_string(lock_path()).ok()?;
    let lock: LockInfo = serde_json::from_str(&text).ok()?;
    if pid_alive(lock.pid) {
        Some(lock)
    } else {
        let _ = std::fs::remove_file(lock_path()); // stale
        None
    }
}

pub fn release_lock() {
    // Only remove if it's ours.
    if let Some(lock) = read_lock()
        && lock.pid == std::process::id()
    {
        let _ = std::fs::remove_file(lock_path());
    }
}

// ---------------------------------------------------------------- journal

/// An in-flight install plan. Written before anything runs, updated per
/// step, cleared on success — so a killed px can resume the remainder.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Journal {
    pub created: String,
    pub recipe: String,
    pub remaining: Vec<String>,
    pub done: Vec<String>,
}

pub fn read_journal() -> Option<Journal> {
    let text = std::fs::read_to_string(journal_path()).ok()?;
    serde_json::from_str(&text).ok()
}

pub fn write_journal(j: &Journal) {
    let _ = std::fs::create_dir_all(state_dir());
    if let Ok(json) = serde_json::to_string_pretty(j) {
        let _ = std::fs::write(journal_path(), json);
    }
}

/// Mark a package done; persist the remainder.
pub fn journal_step(j: &mut Journal, pkg: &str) {
    j.remaining.retain(|p| p != pkg);
    if !j.done.contains(&pkg.to_string()) {
        j.done.push(pkg.to_string());
    }
    write_journal(j);
}

pub fn clear_journal() {
    let _ = std::fs::remove_file(journal_path());
}
