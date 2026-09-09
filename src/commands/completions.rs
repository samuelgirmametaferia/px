//! `px completions <shell>` — shell completion scripts via clap_complete.

use clap::CommandFactory;
use clap_complete::{Shell, generate};

use crate::app::App;
use crate::cli::Cli;
use crate::error::{PxError, PxResult};

pub fn run(_app: App, shell: &str) -> PxResult<()> {
    let shell: Shell = shell.to_lowercase().parse().map_err(|_| {
        PxError::User(format!(
            "unknown shell '{shell}' (try: bash, zsh, fish, elvish, powershell)"
        ))
    })?;
    let mut cmd = Cli::command();
    generate(shell, &mut cmd, "px", &mut std::io::stdout());
    Ok(())
}
