use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{ForgeError, Result};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub repos: Vec<RepoConfig>,
    #[serde(default)]
    pub settings: Settings,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoConfig {
    pub name: String,
    pub path: PathBuf,
    pub github: String,
    #[serde(default = "default_test_command")]
    pub test_command: String,
    #[serde(default)]
    pub docker_required: bool,
    #[serde(default = "default_issue_label")]
    pub issue_label: String,
    #[serde(default = "default_base_branch")]
    pub base_branch: String,
    #[serde(default)]
    pub extra_tools: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    #[serde(default = "default_parallel_workers")]
    pub parallel_workers: usize,
    #[serde(default = "default_auto_merge")]
    pub auto_merge: AutoMergePolicy,
    #[serde(default)]
    pub models: ModelSettings,
    #[serde(default = "default_worker_tools")]
    pub worker_tools: Vec<String>,
    #[serde(default = "default_poll_interval")]
    pub poll_interval_secs: u64,
    #[serde(default = "default_worktree_dir")]
    pub worktree_dir: String,
    #[serde(default = "default_state_file")]
    pub state_file: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelSettings {
    #[serde(default = "default_triage_model")]
    pub triage: String,
    #[serde(default = "default_model")]
    pub default: String,
    #[serde(default = "default_complex_model")]
    pub complex: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum AutoMergePolicy {
    Never,
    TestsPassNoConflict,
    Always,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            parallel_workers: default_parallel_workers(),
            auto_merge: default_auto_merge(),
            models: ModelSettings::default(),
            worker_tools: default_worker_tools(),
            poll_interval_secs: default_poll_interval(),
            worktree_dir: default_worktree_dir(),
            state_file: default_state_file(),
        }
    }
}

impl Default for ModelSettings {
    fn default() -> Self {
        Self {
            triage: default_triage_model(),
            default: default_model(),
            complex: default_complex_model(),
        }
    }
}

fn default_test_command() -> String {
    "cargo test".to_string()
}
fn default_issue_label() -> String {
    "forge".to_string()
}
fn default_base_branch() -> String {
    "main".to_string()
}
fn default_parallel_workers() -> usize {
    4
}
fn default_auto_merge() -> AutoMergePolicy {
    AutoMergePolicy::TestsPassNoConflict
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
    std::env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."))
        .join(".pfl-forge")
        .join("state.yaml")
}
fn default_triage_model() -> String {
    "haiku".to_string()
}
fn default_model() -> String {
    "sonnet".to_string()
}
fn default_complex_model() -> String {
    "opus".to_string()
}

impl Config {
    pub fn load(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Err(ForgeError::ConfigNotFound(path.to_path_buf()));
        }
        let content = std::fs::read_to_string(path)?;
        let config: Config = serde_yaml::from_str(&content)?;
        config.validate()?;
        Ok(config)
    }

    fn validate(&self) -> Result<()> {
        if self.repos.is_empty() {
            return Err(ForgeError::Config("no repos configured".into()));
        }
        for repo in &self.repos {
            if !repo.path.exists() {
                return Err(ForgeError::Config(format!(
                    "repo path does not exist: {}",
                    repo.path.display()
                )));
            }
            if !repo.github.contains('/') {
                return Err(ForgeError::Config(format!(
                    "github must be in owner/repo format: {}",
                    repo.github
                )));
            }
        }
        Ok(())
    }

    pub fn find_repo(&self, name: &str) -> Option<&RepoConfig> {
        self.repos.iter().find(|r| r.name == name)
    }
}

impl RepoConfig {
    pub fn all_tools(&self, base_tools: &[String]) -> Vec<String> {
        let mut tools: Vec<String> = base_tools.to_vec();
        for tool in &self.extra_tools {
            if !tools.contains(tool) {
                tools.push(tool.clone());
            }
        }
        tools
    }

    pub fn owner_repo(&self) -> (&str, &str) {
        self.github.split_once('/').expect("validated in Config::validate")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_settings() {
        let settings = Settings::default();
        assert_eq!(settings.parallel_workers, 4);
        assert_eq!(settings.auto_merge, AutoMergePolicy::TestsPassNoConflict);
        assert_eq!(settings.worker_tools.len(), 6);
    }

    #[test]
    fn test_repo_all_tools() {
        let repo = RepoConfig {
            name: "test".into(),
            path: PathBuf::from("/tmp"),
            github: "owner/repo".into(),
            test_command: "cargo test".into(),
            docker_required: false,
            issue_label: "forge".into(),
            base_branch: "main".into(),
            extra_tools: vec!["WebSearch".into()],
        };
        let base = vec!["Bash".into(), "Read".into()];
        let all = repo.all_tools(&base);
        assert_eq!(all, vec!["Bash", "Read", "WebSearch"]);
    }

    #[test]
    fn test_owner_repo() {
        let repo = RepoConfig {
            name: "test".into(),
            path: PathBuf::from("/tmp"),
            github: "nesso-pfl/pokeca-backend".into(),
            test_command: "cargo test".into(),
            docker_required: false,
            issue_label: "forge".into(),
            base_branch: "main".into(),
            extra_tools: vec![],
        };
        assert_eq!(repo.owner_repo(), ("nesso-pfl", "pokeca-backend"));
    }
}
