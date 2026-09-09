//! Selectable progress bar styles for installs (`--bar <style>`).
//! All styles are indicatif templates behind one enum so new ones are a
//! line each.

use indicatif::{ProgressBar, ProgressStyle};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BarStyle {
    /// Classic ASCII blocks
    Blocks,
    /// Fine-grained shades (default)
    Shades,
    /// Rainbow — because px
    Rainbow,
    /// Minimal single-char fill
    Minimal,
    /// Emoji vibes
    Sparkles,
}

impl BarStyle {
    pub fn parse(s: &str) -> Option<BarStyle> {
        match s {
            "blocks" => Some(BarStyle::Blocks),
            "shades" => Some(BarStyle::Shades),
            "rainbow" => Some(BarStyle::Rainbow),
            "minimal" => Some(BarStyle::Minimal),
            "sparkles" => Some(BarStyle::Sparkles),
            _ => None,
        }
    }

    pub fn names() -> &'static str {
        "blocks, shades, rainbow, minimal, sparkles"
    }
}

/// A determinate bar over `total` steps (one step per package).
pub fn install_bar(style: BarStyle, total: usize, label: &str) -> ProgressBar {
    let pb = ProgressBar::new(total as u64);
    pb.set_style(style.template());
    pb.set_message(label.to_string());
    // Redraw on a timer, not just on inc(): a single-package install can
    // legitimately run for minutes without a step, and a bar frozen at
    // "0% [00:00:00]" looks exactly like a hang. The steady tick keeps the
    // elapsed clock live so the user always knows it's working.
    pb.enable_steady_tick(std::time::Duration::from_millis(500));
    set_active(&pb);
    pb
}

/// A byte-based bar driven by the network-flow monitor: position is
/// received-bytes, length is the known download total, so percent is real.
pub fn bytes_bar(style: BarStyle, total_bytes: u64, label: &str) -> ProgressBar {
    let pb = ProgressBar::new(total_bytes.max(1));
    let template = match style {
        BarStyle::Blocks => {
            "{spinner:.green} {msg}\n{wide_bar:.cyan/blue} {bytes}/{total_bytes} ({percent}%) ↓{bytes_per_sec} eta {eta}"
        }
        BarStyle::Shades => {
            "{spinner:.green} {msg}\n{wide_bar:.magenta/white} {bytes}/{total_bytes} {percent}% ↓{bytes_per_sec} eta {eta} [{elapsed_precise}]"
        }
        BarStyle::Rainbow => {
            "{spinner:.green} {msg}\n{wide_bar:.yellow/green} {bytes}/{total_bytes} {percent}% ↓{bytes_per_sec} eta {eta} ✨"
        }
        BarStyle::Minimal => {
            "{msg} [{wide_bar:.white/dim}] {bytes}/{total_bytes} ↓{bytes_per_sec} eta {eta}"
        }
        BarStyle::Sparkles => {
            "{spinner:.magenta} {msg}\n{wide_bar:.yellow/cyan} {bytes}/{total_bytes} {percent}% ↓{bytes_per_sec} eta {eta} 🚀"
        }
    };
    pb.set_style(
        ProgressStyle::with_template(template)
            .unwrap_or_else(|_| ProgressStyle::default_bar())
            .progress_chars(match style {
                BarStyle::Blocks => "█▉▊▋▌▍▎▏░",
                BarStyle::Shades => "█▓▒░·",
                BarStyle::Rainbow => "█▓▒░ ",
                BarStyle::Minimal => "=-",
                BarStyle::Sparkles => "━─",
            }),
    );
    pb.set_message(label.to_string());
    pb.enable_steady_tick(std::time::Duration::from_millis(500));
    set_active(&pb);
    pb
}

/// An open-ended flow bar for installs whose download size is unknowable
/// (AUR source builds): no percentage, but the live byte counter climbs as
/// the package manager downloads — the bar never looks dead.
pub fn flow_bar(style: BarStyle, label: &str) -> ProgressBar {
    let pb = ProgressBar::new_spinner();
    let template = match style {
        BarStyle::Blocks | BarStyle::Shades => {
            "{spinner:.green} {msg} ↓ {bytes} @ {bytes_per_sec} {elapsed}"
        }
        BarStyle::Rainbow => "{spinner:.green} {msg} ↓ {bytes} @ {bytes_per_sec} {elapsed} ✨",
        BarStyle::Minimal => "{msg} ↓ {bytes} @ {bytes_per_sec} {elapsed}",
        BarStyle::Sparkles => "{spinner:.magenta} {msg} ↓ {bytes} @ {bytes_per_sec} {elapsed} 🚀",
    };
    pb.set_style(
        ProgressStyle::with_template(template)
            .unwrap_or_else(|_| ProgressStyle::default_spinner())
            .tick_chars(match style {
                BarStyle::Blocks => "█▉▊▋▌▍▎▏░",
                BarStyle::Shades => "█▓▒░·",
                BarStyle::Rainbow => "█▓▒░ ",
                BarStyle::Minimal => "=-",
                BarStyle::Sparkles => "━─",
            }),
    );
    pb.set_message(label.to_string());
    pb.enable_steady_tick(std::time::Duration::from_millis(500));
    set_active(&pb);
    pb
}

// ------------------------------------------------------- live-bar hiding
//
// Children with inherited stdio (sudo prompts, pacman/npm output) must not
// interleave with a live bar — that's how terminals got cooked. The
// executor hides the active bar around every inherited child and restores
// it afterwards.

static ACTIVE: std::sync::OnceLock<std::sync::Mutex<Option<ProgressBar>>> =
    std::sync::OnceLock::new();

fn active_slot() -> &'static std::sync::Mutex<Option<ProgressBar>> {
    ACTIVE.get_or_init(|| std::sync::Mutex::new(None))
}

pub fn set_active(bar: &ProgressBar) {
    *active_slot().lock().unwrap() = Some(bar.clone());
}

/// Hide the active bar (child about to take over the terminal).
pub fn hide_active() {
    if let Some(bar) = active_slot().lock().unwrap().as_ref() {
        bar.set_draw_target(indicatif::ProgressDrawTarget::hidden());
    }
}

/// Restore the active bar after a child finished.
pub fn show_active() {
    if let Some(bar) = active_slot().lock().unwrap().as_ref() {
        bar.set_draw_target(indicatif::ProgressDrawTarget::stderr());
        bar.tick();
    }
}

/// Clear everything (interrupt / shutdown) so the terminal is left clean.
pub fn clear_all() {
    if let Some(bar) = active_slot().lock().unwrap().take() {
        bar.finish_and_clear();
    }
}

impl BarStyle {
    fn template(&self) -> ProgressStyle {
        let template = match self {
            BarStyle::Blocks => "{spinner:.green} {msg}\n{wide_bar:.cyan/blue} {pos}/{len}",
            BarStyle::Shades => {
                "{spinner:.green} {msg}\n{wide_bar:.magenta/white} {percent}% [{elapsed_precise}]"
            }
            BarStyle::Rainbow => "{spinner:.green} {msg}\n{wide_bar:.yellow/green} {pos}/{len} ✨",
            BarStyle::Minimal => "{msg} [{wide_bar:.white/dim}] {pos}/{len}",
            BarStyle::Sparkles => {
                "{spinner:.magenta} {msg}\n{wide_bar:.yellow/cyan} {pos}/{len} 🚀"
            }
        };
        // progress_chars: first char = full, last = empty, middle = states.
        let chars = match self {
            BarStyle::Blocks => "█▉▊▋▌▍▎▏░",
            BarStyle::Shades => "█▓▒░·",
            BarStyle::Rainbow => "█▓▒░ ",
            BarStyle::Minimal => "=-",
            BarStyle::Sparkles => "━─",
        };
        ProgressStyle::with_template(template)
            .unwrap_or_else(|_| ProgressStyle::default_bar())
            .progress_chars(chars)
    }
}
