use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use std::time::Duration;

/// Named spinners for parallel work (e.g. searching repo + AUR + GitHub at
/// once). Each spinner collapses to a one-line result via `finish_with`.
pub struct Spinners {
    multi: MultiProgress,
    style_on: bool,
}

impl Default for Spinners {
    fn default() -> Self {
        Self::new(true)
    }
}

impl Spinners {
    pub fn new(style_on: bool) -> Self {
        Spinners {
            multi: MultiProgress::new(),
            style_on,
        }
    }

    pub fn add(&self, label: &str) -> ProgressBar {
        let pb = self.multi.add(ProgressBar::new_spinner());
        let template = if self.style_on {
            "{spinner:.magenta} {msg}"
        } else {
            "{msg}"
        };
        pb.set_style(
            ProgressStyle::with_template(template)
                .unwrap_or_else(|_| ProgressStyle::default_spinner())
                .tick_chars("⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏"),
        );
        pb.set_message(format!("{label}…"));
        pb.enable_steady_tick(Duration::from_millis(80));
        pb
    }
}

/// A single indeterminate spinner for one long task. All spinners share one
/// MultiProgress so sequential/parallel spinners stack on separate lines
/// instead of painting over each other.
pub fn one(label: &str) -> ProgressBar {
    static MULTI: std::sync::OnceLock<MultiProgress> = std::sync::OnceLock::new();
    let multi = MULTI.get_or_init(MultiProgress::new);
    let pb = multi.add(ProgressBar::new_spinner());
    pb.set_style(
        ProgressStyle::with_template("{spinner:.magenta} {msg}")
            .unwrap_or_else(|_| ProgressStyle::default_spinner())
            .tick_chars("⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏"),
    );
    pb.set_message(label.to_string());
    pb.enable_steady_tick(Duration::from_millis(80));
    pb
}

pub fn finish_ok(pb: &ProgressBar, msg: String) {
    pb.set_style(
        ProgressStyle::with_template("{prefix:.green} {msg}")
            .unwrap_or_else(|_| ProgressStyle::default_spinner()),
    );
    pb.set_prefix("✔".to_string());
    pb.finish_with_message(msg);
}

pub fn finish_warn(pb: &ProgressBar, msg: String) {
    pb.set_style(
        ProgressStyle::with_template("{prefix:.yellow} {msg}")
            .unwrap_or_else(|_| ProgressStyle::default_spinner()),
    );
    pb.set_prefix("⚠".to_string());
    pb.finish_with_message(msg);
}

pub fn finish_err(pb: &ProgressBar, msg: String) {
    pb.set_style(
        ProgressStyle::with_template("{prefix:.red} {msg}")
            .unwrap_or_else(|_| ProgressStyle::default_spinner()),
    );
    pb.set_prefix("✘".to_string());
    pb.finish_with_message(msg);
}
