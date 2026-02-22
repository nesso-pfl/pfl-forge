use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::claude::runner::ClaudeMetadata;
use crate::error::Result;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
  Success,
  Failed,
  Escalated,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepResult {
  pub step: String,
  pub duration_secs: u64,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub metadata: Option<ClaudeMetadata>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryEntry {
  pub intent_id: String,
  pub intent_type: Option<String>,
  pub intent_risk: Option<String>,
  pub title: String,
  pub flow: Vec<String>,
  #[serde(default)]
  pub step_results: Vec<StepResult>,
  pub outcome: Outcome,
  pub failure_reason: Option<String>,
  #[serde(default)]
  pub observations: Vec<String>,
  pub created_at: Option<String>,
}

fn history_dir(repo_path: &Path) -> std::path::PathBuf {
  repo_path.join(".forge").join("knowledge").join("history")
}

pub fn write(repo_path: &Path, entry: &HistoryEntry) -> Result<()> {
  let dir = history_dir(repo_path);
  std::fs::create_dir_all(&dir)?;
  let filename = format!("{}.yaml", entry.intent_id);
  let content = serde_yaml::to_string(entry)?;
  std::fs::write(dir.join(filename), content)?;
  Ok(())
}

pub fn load(repo_path: &Path, intent_id: &str) -> Result<HistoryEntry> {
  let path = history_dir(repo_path).join(format!("{intent_id}.yaml"));
  let content = std::fs::read_to_string(&path)?;
  let entry: HistoryEntry = serde_yaml::from_str(&content)?;
  Ok(entry)
}
