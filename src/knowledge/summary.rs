use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::Result;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ExecutionSummary {
  pub intent_id: String,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub analyze: Option<AnalyzeSummary>,
  #[serde(default)]
  pub tasks: Vec<TaskSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnalyzeSummary {
  pub complexity: String,
  pub plan: String,
  pub relevant_files: Vec<String>,
  pub task_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskSummary {
  pub task_id: String,
  #[serde(default)]
  pub commits: Vec<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub review: Option<ReviewSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewSummary {
  pub approved: bool,
  #[serde(default)]
  pub issues: Vec<String>,
  #[serde(default)]
  pub suggestions: Vec<String>,
}

fn logs_dir(repo_path: &Path) -> std::path::PathBuf {
  repo_path.join(".forge").join("knowledge").join("logs")
}

pub fn write(repo_path: &Path, summary: &ExecutionSummary) -> Result<()> {
  let dir = logs_dir(repo_path);
  std::fs::create_dir_all(&dir)?;
  let filename = format!("{}.yaml", summary.intent_id);
  let content = serde_yaml::to_string(summary)?;
  std::fs::write(dir.join(filename), content)?;
  Ok(())
}

pub fn load(repo_path: &Path, intent_id: &str) -> Result<ExecutionSummary> {
  let path = logs_dir(repo_path).join(format!("{intent_id}.yaml"));
  let content = std::fs::read_to_string(&path)?;
  let summary: ExecutionSummary = serde_yaml::from_str(&content)?;
  Ok(summary)
}
