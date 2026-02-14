use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum ForgeError {
  #[error("config error: {0}")]
  Config(String),

  #[error("config file not found: {0}")]
  ConfigNotFound(PathBuf),

  #[error("github error: {0}")]
  GitHub(String),

  #[error("git error: {0}")]
  Git(String),

  #[error("claude execution error: {0}")]
  Claude(String),

  #[error("timeout: {0}")]
  Timeout(String),

  #[error("state error: {0}")]
  State(String),

  #[error("io error: {0}")]
  Io(#[from] std::io::Error),

  #[error("yaml error: {0}")]
  Yaml(#[from] serde_yaml::Error),

  #[error("json error: {0}")]
  Json(#[from] serde_json::Error),

  #[error("octocrab error: {0}")]
  Octocrab(#[from] octocrab::Error),
}

pub type Result<T> = std::result::Result<T, ForgeError>;
