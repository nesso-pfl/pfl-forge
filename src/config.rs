use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::error::{ForgeError, Result};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
  #[serde(default = "default_base_branch")]
  pub base_branch: String,
  #[serde(default = "default_parallel_workers")]
  pub parallel_workers: usize,
  #[serde(default)]
  pub models: ModelSettings,
  #[serde(default = "default_implement_tools")]
  pub implement_tools: Vec<String>,
  #[serde(default = "default_poll_interval")]
  pub poll_interval_secs: u64,
  #[serde(default = "default_analyze_tools")]
  pub analyze_tools: Vec<String>,
  #[serde(default = "default_worktree_dir")]
  pub worktree_dir: String,
  #[serde(default = "default_worker_timeout")]
  pub worker_timeout_secs: u64,
  #[serde(default = "default_analyze_timeout")]
  pub analyze_timeout_secs: u64,
  #[serde(default = "default_max_review_retries")]
  pub max_review_retries: u32,
  #[serde(default)]
  pub worktree_setup: Vec<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub mcp_config: Option<String>,
  #[serde(default = "default_memory_server")]
  pub memory_server: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelSettings {
  #[serde(default = "default_analyze_model")]
  pub analyze: String,
  #[serde(default = "default_implement_model")]
  pub implement: String,
  #[serde(default = "default_implement_complex_model")]
  pub implement_complex: String,
  #[serde(default = "default_review_model")]
  pub review: String,
  #[serde(default = "default_reflect_model")]
  pub reflect: String,
  #[serde(default = "default_skill_model")]
  pub skill: String,
  #[serde(default = "default_audit_model")]
  pub audit: String,
}

impl Default for ModelSettings {
  fn default() -> Self {
    Self {
      analyze: default_analyze_model(),
      implement: default_implement_model(),
      implement_complex: default_implement_complex_model(),
      review: default_review_model(),
      reflect: default_reflect_model(),
      skill: default_skill_model(),
      audit: default_audit_model(),
    }
  }
}

fn default_base_branch() -> String {
  "main".to_string()
}
fn default_parallel_workers() -> usize {
  4
}
fn default_implement_tools() -> Vec<String> {
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
fn default_worker_timeout() -> u64 {
  1200
}
fn default_analyze_timeout() -> u64 {
  600
}
fn default_max_review_retries() -> u32 {
  2
}
fn default_analyze_tools() -> Vec<String> {
  vec![
    "Read".into(),
    "Glob".into(),
    "Grep".into(),
    "WebSearch".into(),
    "WebFetch".into(),
  ]
}
fn default_memory_server() -> String {
  "memory-pfl".to_string()
}
fn default_analyze_model() -> String {
  "opus".to_string()
}
fn default_implement_model() -> String {
  "sonnet".to_string()
}
fn default_implement_complex_model() -> String {
  "opus".to_string()
}
fn default_review_model() -> String {
  "sonnet".to_string()
}
fn default_reflect_model() -> String {
  "sonnet".to_string()
}
fn default_skill_model() -> String {
  "sonnet".to_string()
}
fn default_audit_model() -> String {
  "opus".to_string()
}

impl Config {
  pub fn load(path: &std::path::Path) -> Result<Self> {
    if !path.exists() {
      return Err(ForgeError::ConfigNotFound(path.to_path_buf()));
    }
    let content = std::fs::read_to_string(path)?;
    let mut config: Config = serde_yaml::from_str(&content)?;
    config.resolve_mcp_config()?;
    Ok(config)
  }

  /// Resolve `mcp_config` to an existing path.
  /// 1. If explicitly set → use that path
  /// 2. Fallback to `{CWD}/.claude/mcp.json`
  /// 3. If global `~/.claude.json` has `mcpServers` → no local config needed
  /// 4. Otherwise → error
  fn resolve_mcp_config(&mut self) -> Result<()> {
    if let Some(ref explicit) = self.mcp_config {
      let p = PathBuf::from(explicit);
      if p.exists() {
        self.mcp_config = Some(p.to_string_lossy().into_owned());
        return Ok(());
      }
      return Err(ForgeError::Config(format!(
        "mcp_config not found: {explicit}"
      )));
    }

    let local_fallback = Self::repo_path().join(".claude/mcp.json");
    if local_fallback.exists() {
      self.mcp_config = Some(local_fallback.to_string_lossy().into_owned());
      return Ok(());
    }

    if Self::has_global_mcp_servers() {
      // mcp_config stays None — Claude CLI picks up global config automatically
      return Ok(());
    }

    Err(ForgeError::Config(
      "MCP config required but not found. Set mcp_config in pfl-forge.yaml, create .claude/mcp.json, or configure mcpServers in ~/.claude.json".into(),
    ))
  }

  /// Check if `~/.claude.json` has a non-empty `mcpServers` object.
  fn has_global_mcp_servers() -> bool {
    let Ok(home) = std::env::var("HOME") else {
      return false;
    };
    let path = PathBuf::from(home).join(".claude.json");
    let Ok(content) = std::fs::read_to_string(&path) else {
      return false;
    };
    let Ok(val) = serde_json::from_str::<serde_json::Value>(&content) else {
      return false;
    };
    val
      .get("mcpServers")
      .and_then(|v| v.as_object())
      .is_some_and(|m| !m.is_empty())
  }

  pub fn repo_path() -> PathBuf {
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn 空のyamlで有効なデフォルト設定が生成される() {
    let yaml = "{}";
    let config: Config = serde_yaml::from_str(yaml).unwrap();
    assert_eq!(config.parallel_workers, 4);
    assert_eq!(config.implement_tools.len(), 6);
    assert_eq!(config.base_branch, "main");
    assert_eq!(config.max_review_retries, 2);
    assert_eq!(config.memory_server, "memory-pfl");
  }

  #[test]
  fn mcp_config指定パスが存在すればresolveに成功する() {
    let dir = tempfile::tempdir().unwrap();
    let mcp_path = dir.path().join("mcp.json");
    std::fs::write(&mcp_path, "{}").unwrap();

    let yaml = format!("mcp_config: {}", mcp_path.display());
    let config_path = dir.path().join("pfl-forge.yaml");
    std::fs::write(&config_path, &yaml).unwrap();

    let config = Config::load(&config_path).unwrap();
    assert!(config.mcp_config.is_some());
  }

  #[test]
  fn mcp_config指定パスが不在ならエラーを返す() {
    let dir = tempfile::tempdir().unwrap();
    let yaml = "mcp_config: /nonexistent/mcp.json";
    let config_path = dir.path().join("pfl-forge.yaml");
    std::fs::write(&config_path, yaml).unwrap();

    let err = Config::load(&config_path).unwrap_err();
    assert!(err.to_string().contains("mcp_config not found"));
  }

  #[test]
  fn mcp_config省略時にローカルもグローバルもなければエラーを返す() {
    let dir = tempfile::tempdir().unwrap();
    let config_path = dir.path().join("pfl-forge.yaml");
    std::fs::write(&config_path, "{}").unwrap();

    // Hide global config by unsetting HOME
    let orig_home = std::env::var("HOME").ok();
    unsafe { std::env::set_var("HOME", dir.path()) };

    let result = Config::load(&config_path);

    // Restore HOME
    if let Some(h) = orig_home {
      unsafe { std::env::set_var("HOME", h) };
    }

    assert!(result.is_err());
    let msg = result.unwrap_err().to_string();
    assert!(msg.contains("MCP config required"));
  }

  #[test]
  fn mcp_config省略時にグローバルmcpServersがあればokを返す() {
    let dir = tempfile::tempdir().unwrap();
    let config_path = dir.path().join("pfl-forge.yaml");
    std::fs::write(&config_path, "{}").unwrap();

    // Create fake global ~/.claude.json with mcpServers
    let fake_home = tempfile::tempdir().unwrap();
    let claude_json = fake_home.path().join(".claude.json");
    std::fs::write(
      &claude_json,
      r#"{"mcpServers":{"test":{"type":"stdio","command":"echo"}}}"#,
    )
    .unwrap();

    let orig_home = std::env::var("HOME").ok();
    unsafe { std::env::set_var("HOME", fake_home.path()) };

    let result = Config::load(&config_path);

    if let Some(h) = orig_home {
      unsafe { std::env::set_var("HOME", h) };
    }

    let config = result.unwrap();
    assert!(config.mcp_config.is_none());
  }
}
