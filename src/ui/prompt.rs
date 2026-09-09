use dialoguer::{Confirm, Input, Select};
use std::io::Write;

use crate::error::{PxError, PxResult};

/// All prompts default to the SAFE answer (No / first option).
/// Prompts are skipped under --yes by callers; this module only asks.
fn term() -> dialoguer::console::Term {
    dialoguer::console::Term::stderr()
}

/// Suspend every live spinner around an interactive prompt — a prompt drawn
/// behind a redrawing spinner is invisible, and invisible prompts are what
/// hangs look like.
macro_rules! suspended_prompt {
    ($body:expr) => {{
        crate::ui::spinner::suspend_all();
        let result = $body;
        crate::ui::spinner::resume_all();
        result
    }};
}

pub fn confirm(question: &str, default: bool) -> PxResult<bool> {
    suspended_prompt!(
        Confirm::with_theme(&dialoguer::theme::ColorfulTheme::default())
            .with_prompt(question.to_string())
            .default(default)
            .show_default(true)
            .interact_on(&term())
            .map_err(|_| PxError::Cancelled)
    )
}

pub fn select(question: &str, items: &[String]) -> PxResult<usize> {
    suspended_prompt!(
        Select::with_theme(&dialoguer::theme::ColorfulTheme::default())
            .with_prompt(question.to_string())
            .items(items)
            .default(0)
            .interact_on(&term())
            .map_err(|_| PxError::Cancelled)
    )
}

pub fn input(question: &str) -> PxResult<String> {
    suspended_prompt!(
        Input::with_theme(&dialoguer::theme::ColorfulTheme::default())
            .with_prompt(question.to_string())
            .interact_on(&term())
            .map_err(|_| PxError::Cancelled)
    )
}

/// Flush stdout before handing the terminal to a child process or dialoguer,
/// so px output and child output never interleave mid-line.
pub fn flush() {
    let _ = std::io::stdout().flush();
}
