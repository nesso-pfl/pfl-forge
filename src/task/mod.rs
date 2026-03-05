use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tracing::info;

use crate::agent::analyze::TaskSpec;
use crate::claude::model;
use crate::error::Result;
use crate::intent::registry::Intent;

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum WorkStatus {
  #[default]
  Pending,
  Implementing,
  Completed,
  Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
  pub id: String,
  pub title: String,
  pub intent_id: String,
  #[serde(default)]
  pub status: WorkStatus,
  pub complexity: String,
  pub plan: String,
  pub relevant_files: Vec<String>,
  pub implementation_steps: Vec<String>,
  pub context: String,
  #[serde(default)]
  pub depends_on: Vec<String>,
}

impl Task {
  pub fn from_spec(intent: &Intent, spec: &TaskSpec) -> Self {
    let id = if spec.id.is_empty() {
      intent.id().to_string()
    } else {
      spec.id.clone()
    };
    let title = if spec.title.is_empty() {
      intent.title.clone()
    } else {
      spec.title.clone()
    };
    Self {
      id,
      title,
      intent_id: intent.id().to_string(),
      status: WorkStatus::Pending,
      complexity: spec.complexity.clone(),
      plan: spec.plan.clone(),
      relevant_files: spec.relevant_files.clone(),
      implementation_steps: spec.implementation_steps.clone(),
      context: spec.context.clone(),
      depends_on: spec.depends_on.clone(),
    }
  }

  pub fn complexity(&self) -> model::Complexity {
    self.complexity.parse().unwrap_or(model::Complexity::Medium)
  }
}

fn tasks_dir(repo_path: &Path) -> PathBuf {
  repo_path.join(".forge").join("tasks")
}

fn tasks_file(repo_path: &Path, intent_id: &str) -> PathBuf {
  tasks_dir(repo_path).join(format!("{intent_id}.yaml"))
}

/// Write all tasks to `.forge/tasks/{intent_id}.yaml` in the main repo.
pub fn write_all_tasks(repo_path: &Path, intent_id: &str, tasks: &[Task]) -> Result<()> {
  let dir = tasks_dir(repo_path);
  std::fs::create_dir_all(&dir)?;
  let path = dir.join(format!("{intent_id}.yaml"));
  let content = serde_yaml::to_string(tasks)?;
  std::fs::write(&path, content)?;
  info!("wrote {} tasks to {}", tasks.len(), path.display());
  Ok(())
}

/// Read all tasks from `.forge/tasks/{intent_id}.yaml`.
pub fn read_all_tasks(repo_path: &Path, intent_id: &str) -> Result<Vec<Task>> {
  let path = tasks_file(repo_path, intent_id);
  let content = std::fs::read_to_string(&path)?;
  let tasks: Vec<Task> = serde_yaml::from_str(&content)?;
  Ok(tasks)
}

/// Check if tasks file exists for the given intent.
pub fn tasks_exist(repo_path: &Path, intent_id: &str) -> bool {
  tasks_file(repo_path, intent_id).exists()
}
