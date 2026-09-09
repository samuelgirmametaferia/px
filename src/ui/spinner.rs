//! Spinners. Two invariants keep the terminal sane:
//!
//! 1. Every spinner lives in ONE global MultiProgress, so parallel and
//!    sequential spinners stack on separate lines instead of painting over
//!    each other.
//! 2. Anything interactive (a dialoguer prompt, a sudo password) calls
//!    `suspend_all()` first — a prompt hidden behind a redrawing spinner
//!    looks exactly like a hang, because it is one.
//!
//! Elapsed time is always visible so a legitimately long operation never
//! looks frozen.

use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

const TICK_CHARS: &str = "⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏";

fn global_multi() -> &'static MultiProgress {
    static MULTI: OnceLock<MultiProgress> = OnceLock::new();
    MULTI.get_or_init(MultiProgress::new)
}

/// Spinners that are currently live (removed on finish).
fn live() -> &'static Mutex<Vec<ProgressBar>> {
    static LIVE: OnceLock<Mutex<Vec<ProgressBar>>> = OnceLock::new();
    LIVE.get_or_init(|| Mutex::new(Vec::new()))
}

fn register(pb: &ProgressBar) {
    live().lock().unwrap().push(pb.clone());
}

/// Hide every live spinner (a prompt or child is about to own the terminal).
pub fn suspend_all() {
    live().lock().unwrap().retain(|p| !p.is_finished());
    for pb in live().lock().unwrap().iter() {
        pb.set_draw_target(indicatif::ProgressDrawTarget::hidden());
    }
    let _ = global_multi().clear();
}

/// Bring suspended spinners back.
pub fn resume_all() {
    live().lock().unwrap().retain(|p| !p.is_finished());
    for pb in live().lock().unwrap().iter() {
        pb.set_draw_target(indicatif::ProgressDrawTarget::stderr());
        pb.tick();
    }
}

/// Named spinners for parallel work (e.g. searching repo + AUR + GitHub at
/// once). Each spinner collapses to a one-line result via `finish_with`.
pub struct Spinners {
    style_on: bool,
}

impl Default for Spinners {
    fn default() -> Self {
        Self::new(true)
    }
}

impl Spinners {
    pub fn new(style_on: bool) -> Self {
        Spinners { style_on }
    }

    pub fn add(&self, label: &str) -> ProgressBar {
        let pb = global_multi().add(ProgressBar::new_spinner());
        let template = if self.style_on {
            "{spinner:.magenta} {msg} {elapsed}"
        } else {
            "{msg} {elapsed}"
        };
        pb.set_style(
            ProgressStyle::with_template(template)
                .unwrap_or_else(|_| ProgressStyle::default_spinner())
                .tick_chars(TICK_CHARS),
        );
        pb.set_message(format!("{label}…"));
        pb.enable_steady_tick(Duration::from_millis(80));
        register(&pb);
        pb
    }
}

/// A single indeterminate spinner for one long task.
pub fn one(label: &str) -> ProgressBar {
    let pb = global_multi().add(ProgressBar::new_spinner());
    pb.set_style(
        ProgressStyle::with_template("{spinner:.magenta} {msg} {elapsed}")
            .unwrap_or_else(|_| ProgressStyle::default_spinner())
            .tick_chars(TICK_CHARS),
    );
    pb.set_message(label.to_string());
    pb.enable_steady_tick(Duration::from_millis(80));
    register(&pb);
    pb
}

pub fn finish_ok(pb: &ProgressBar, msg: String) {
    live().lock().unwrap().retain(|p| !p.is_finished());
    pb.set_style(
        ProgressStyle::with_template("{prefix:.green} {msg}")
            .unwrap_or_else(|_| ProgressStyle::default_spinner()),
    );
    pb.set_prefix("✔".to_string());
    pb.finish_with_message(msg);
}

pub fn finish_warn(pb: &ProgressBar, msg: String) {
    live().lock().unwrap().retain(|p| !p.is_finished());
    pb.set_style(
        ProgressStyle::with_template("{prefix:.yellow} {msg}")
            .unwrap_or_else(|_| ProgressStyle::default_spinner()),
    );
    pb.set_prefix("⚠".to_string());
    pb.finish_with_message(msg);
}

pub fn finish_err(pb: &ProgressBar, msg: String) {
    live().lock().unwrap().retain(|p| !p.is_finished());
    pb.set_style(
        ProgressStyle::with_template("{prefix:.red} {msg}")
            .unwrap_or_else(|_| ProgressStyle::default_spinner()),
    );
    pb.set_prefix("✘".to_string());
    pb.finish_with_message(msg);
}
