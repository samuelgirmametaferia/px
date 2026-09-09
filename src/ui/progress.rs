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
