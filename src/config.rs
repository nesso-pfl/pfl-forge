use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::error::{ForgeError, Result};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
  #[serde(default = "default_test_command")]
  pub test_command: String,
  #[serde(default)]
  pub docker_required: bool,
  #[serde(default = "default_base_branch")]
  pub base_branch: String,
  #[serde(default)]
  pub extra_tools: Vec<String>,
  #[serde(default)]
  pub settings: Settings,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
  #[serde(default = "default_parallel_workers")]
  pub parallel_workers: usize,
  #[serde(default)]
  pub models: ModelSettings,
  #[serde(default = "default_worker_tools")]
  pub worker_tools: Vec<String>,
  #[serde(default = "default_poll_interval")]
  pub poll_interval_secs: u64,
  #[serde(default = "default_triage_tools")]
  pub triage_tools: Vec<String>,
  #[serde(default = "default_worktree_dir")]
  pub worktree_dir: String,
  #[serde(default = "default_state_file")]
  pub state_file: PathBuf,
  #[serde(default = "default_worker_timeout")]
  pub worker_timeout_secs: u64,
  #[serde(default = "default_triage_timeout")]
  pub triage_timeout_secs: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelSettings {
  #[serde(default = "default_triage_model")]
  pub triage: String,
  #[serde(default = "default_triage_deep_model")]
  pub triage_deep: String,
  #[serde(default = "default_model")]
  pub default: String,
  #[serde(default = "default_complex_model")]
  pub complex: String,
}

impl Default for Settings {
  fn default() -> Self {
    Self {
      parallel_workers: default_parallel_workers(),
      models: ModelSettings::default(),
      worker_tools: default_worker_tools(),
      triage_tools: default_triage_tools(),
      poll_interval_secs: default_poll_interval(),
      worktree_dir: default_worktree_dir(),
      state_file: default_state_file(),
      worker_timeout_secs: default_worker_timeout(),
      triage_timeout_secs: default_triage_timeout(),
    }
  }
}

impl Default for ModelSettings {
  fn default() -> Self {
    Self {
      triage: default_triage_model(),
      triage_deep: default_triage_deep_model(),
      default: default_model(),
      complex: default_complex_model(),
    }
  }
}

fn default_test_command() -> String {
  "cargo test".to_string()
}
fn default_base_branch() -> String {
  "main".to_string()
}
fn default_parallel_workers() -> usize {
  4
}
fn default_worker_tools() -> Vec<String> {
  vec![
    "Bash".into(),
    "Read".into(),
    "Write".into(),
    "Edit".into(),
    "Glob".into(),
    "Grep".into(),
  ]
}
fn default_poll_interval() -> u64 {
  300
}
fn default_worktree_dir() -> String {
  ".pfl-worktrees".to_string()
}
fn default_state_file() -> PathBuf {
  PathBuf::from(".forge/state.yaml")
}
fn default_worker_timeout() -> u64 {
  1200
}
fn default_triage_timeout() -> u64 {
  600
}
fn default_triage_tools() -> Vec<String> {
  vec!["Read".into(), "Glob".into(), "Grep".into()]
}
fn default_triage_model() -> String {
  "haiku".to_string()
}
fn default_triage_deep_model() -> String {
  "sonnet".to_string()
}
fn default_model() -> String {
  "sonnet".to_string()
}
fn default_complex_model() -> String {
  "opus".to_string()
}

impl Config {
  pub fn load(path: &std::path::Path) -> Result<Self> {
    if !path.exists() {
      return Err(ForgeError::ConfigNotFound(path.to_path_buf()));
    }
    let content = std::fs::read_to_string(path)?;
    let config: Config = serde_yaml::from_str(&content)?;
    Ok(config)
  }

  pub fn repo_path() -> PathBuf {
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
  }

  pub fn all_tools(&self) -> Vec<String> {
    let mut tools: Vec<String> = self.settings.worker_tools.clone();
    for tool in &self.extra_tools {
      if !tools.contains(tool) {
        tools.push(tool.clone());
      }
    }
    tools
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_default_settings() {
    let settings = Settings::default();
    assert_eq!(settings.parallel_workers, 4);
    assert_eq!(settings.worker_tools.len(), 6);
  }

  #[test]
  fn test_all_tools() {
    let config = Config {
      test_command: "cargo test".into(),
      docker_required: false,
      base_branch: "main".into(),
      extra_tools: vec!["WebSearch".into()],
      settings: Settings::default(),
    };
    let all = config.all_tools();
    assert_eq!(all.len(), 7);
    assert!(all.contains(&"WebSearch".to_string()));
  }
}
