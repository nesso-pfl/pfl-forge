use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tracing::info;

use crate::agents::analyze::AnalysisResult;
use crate::claude::model;
use crate::error::Result;
use crate::task::Issue;

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum WorkStatus {
  #[default]
  Pending,
  Executing,
  Completed,
  Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
  pub id: String,
  pub title: String,
  pub body: String,
  #[serde(default)]
  pub status: WorkStatus,
  pub complexity: String,
  pub plan: String,
  pub relevant_files: Vec<String>,
  pub implementation_steps: Vec<String>,
  pub context: String,
}

impl Task {
  pub fn from_analysis(issue: &Issue, deep: &AnalysisResult) -> Self {
    Self {
      id: issue.id.clone(),
      title: issue.title.clone(),
      body: issue.body.clone(),
      status: WorkStatus::Pending,
      complexity: deep.complexity.clone(),
      plan: deep.plan.clone(),
      relevant_files: deep.relevant_files.clone(),
      implementation_steps: deep.implementation_steps.clone(),
      context: deep.context.clone(),
    }
  }

  pub fn complexity(&self) -> model::Complexity {
    self.complexity.parse().unwrap_or(model::Complexity::Medium)
  }
}

fn work_dir(repo_path: &Path) -> PathBuf {
  repo_path.join(".forge").join("work")
}

fn task_filename(task_id: &str, index: u32) -> String {
  format!("{task_id}-{index:03}.yaml")
}

pub fn write_tasks(repo_path: &Path, issue: &Issue, deep: &AnalysisResult) -> Result<Vec<PathBuf>> {
  let dir = work_dir(repo_path);
  std::fs::create_dir_all(&dir)?;

  let task = Task::from_analysis(issue, deep);
  let path = dir.join(task_filename(&issue.id, 1));
  let content = serde_yaml::to_string(&task)?;
  std::fs::write(&path, content)?;

  info!("wrote task: {}", path.display());
  Ok(vec![path])
}

pub fn set_task_status(path: &Path, status: WorkStatus) -> Result<()> {
  let content = std::fs::read_to_string(path)?;
  let mut task: Task = serde_yaml::from_str(&content)?;
  task.status = status;
  let content = serde_yaml::to_string(&task)?;
  std::fs::write(path, content)?;
  Ok(())
}

pub fn write_task_yaml(worktree_path: &Path, task: &Task) -> Result<()> {
  let forge_dir = worktree_path.join(".forge");
  std::fs::create_dir_all(&forge_dir)?;
  let content = serde_yaml::to_string(task)?;
  std::fs::write(forge_dir.join("task.yaml"), content)?;
  Ok(())
}
