use tracing::info;

use crate::config::Config;
use crate::error::Result;
use crate::pipeline::clarification;
use crate::state::tracker::StateTracker;
use crate::task::ForgeIssue;

#[derive(serde::Deserialize)]
struct LocalTask {
  title: String,
  body: String,
  #[serde(default)]
  labels: Vec<String>,
}

pub fn fetch_local_tasks(_config: &Config, state: &StateTracker) -> Result<Vec<ForgeIssue>> {
  let repo_name = Config::repo_name();
  let repo_path = Config::repo_path();
  let tasks_dir = repo_path.join(".forge/tasks");
  if !tasks_dir.exists() {
    info!("local tasks: 0");
    return Ok(Vec::new());
  }

  let mut issues = Vec::new();
  let mut entries: Vec<_> = std::fs::read_dir(&tasks_dir)?
    .filter_map(|e| e.ok())
    .collect();
  entries.sort_by_key(|e| e.file_name());

  for entry in entries {
    let path = entry.path();
    if path.extension().and_then(|e| e.to_str()) != Some("yaml") {
      continue;
    }

    let stem = path
      .file_stem()
      .and_then(|s| s.to_str())
      .unwrap_or_default();
    let number: u64 = match stem.parse() {
      Ok(n) => n,
      Err(_) => continue,
    };

    if state.is_terminal(&repo_name, number) {
      info!("skipping terminal local task: {repo_name}#{number}");
      continue;
    }

    let content = std::fs::read_to_string(&path)?;
    let task: LocalTask = serde_yaml::from_str(&content)?;

    issues.push(ForgeIssue {
      number,
      title: task.title,
      body: task.body,
      labels: task.labels,
      repo_name: repo_name.clone(),
      created_at: chrono::Utc::now(),
    });
  }

  info!("local tasks: {}", issues.len());
  Ok(issues)
}

pub fn fetch_resumable_tasks(_config: &Config, state: &StateTracker) -> Result<Vec<ForgeIssue>> {
  let repo_name = Config::repo_name();
  let repo_path = Config::repo_path();
  let resumable = state.resumable_issues();
  let mut issues = Vec::new();

  for (r_name, number) in resumable {
    if r_name != repo_name {
      info!("skipping resumable task {r_name}#{number}: different repo");
      continue;
    }

    let task_path = repo_path
      .join(".forge/tasks")
      .join(format!("{number}.yaml"));
    if !task_path.exists() {
      info!("skipping resumable task {r_name}#{number}: task file not found");
      continue;
    }

    let content = std::fs::read_to_string(&task_path)?;
    let task: LocalTask = serde_yaml::from_str(&content)?;

    info!("resuming: {r_name}#{number}");
    issues.push(ForgeIssue {
      number,
      title: task.title,
      body: task.body,
      labels: task.labels,
      repo_name: repo_name.clone(),
      created_at: chrono::Utc::now(),
    });
  }

  info!("resumable tasks: {}", issues.len());
  Ok(issues)
}

pub fn fetch_clarified_tasks(_config: &Config, state: &StateTracker) -> Result<Vec<ForgeIssue>> {
  let repo_name = Config::repo_name();
  let repo_path = Config::repo_path();
  let needs_clarification = state.needs_clarification_issues();
  let mut issues = Vec::new();

  for (r_name, number) in needs_clarification {
    if r_name != repo_name {
      continue;
    }

    if clarification::check_clarification(&repo_path, number)?.is_none() {
      continue;
    }

    let task_path = repo_path
      .join(".forge/tasks")
      .join(format!("{number}.yaml"));
    if !task_path.exists() {
      continue;
    }

    let content = std::fs::read_to_string(&task_path)?;
    let task: LocalTask = serde_yaml::from_str(&content)?;

    info!("clarification answered, re-processing: {r_name}#{number}");
    issues.push(ForgeIssue {
      number,
      title: task.title,
      body: task.body,
      labels: task.labels,
      repo_name: repo_name.clone(),
      created_at: chrono::Utc::now(),
    });
  }

  info!("clarified tasks: {}", issues.len());
  Ok(issues)
}
