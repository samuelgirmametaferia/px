use thiserror::Error;

/// Every error px can surface to a user, with messages written for humans.
#[derive(Debug, Error)]
pub enum PxError {
    #[error("recipe error: {0}")]
    Recipe(String),

    #[error("no recipe found for this system — pass one with --recipe <path>")]
    NoRecipe,

    #[error("command failed: {cmd}\n{stderr}")]
    Command { cmd: String, stderr: String },

    #[error("network error: {0}")]
    Network(#[from] reqwest::Error),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("{0}")]
    User(String),

    #[error("cancelled")]
    Cancelled,

    #[error("not found: {0}")]
    NotFound(String),

    #[error("{0}")]
    Timeout(String),
}

pub type PxResult<T> = Result<T, PxError>;
