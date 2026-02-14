use std::path::{Path, PathBuf};

use tracing::info;

use crate::error::Result;
use crate::pipeline::triage::{DeepTriageResult, Task, WorkStatus};
use crate::task::ForgeTask;

fn work_dir(repo_path: &Path) -> PathBuf {
  repo_path.join(".forge").join("work")
}

fn task_filename(task_id: &str, index: u32) -> String {
  format!("{task_id}-{index:03}.yaml")
}

pub fn write_tasks(
  repo_path: &Path,
  forge_task: &ForgeTask,
  deep: &DeepTriageResult,
) -> Result<Vec<PathBuf>> {
  let dir = work_dir(repo_path);
  std::fs::create_dir_all(&dir)?;

  let task = Task::from_triage(forge_task, deep);
  let path = dir.join(task_filename(&forge_task.id, 1));
  let content = serde_yaml::to_string(&task)?;
  std::fs::write(&path, content)?;

  info!("wrote task: {}", path.display());
  Ok(vec![path])
}

pub fn read_pending_tasks(repo_path: &Path) -> Result<Vec<(PathBuf, Task)>> {
  let dir = work_dir(repo_path);
  if !dir.exists() {
    return Ok(Vec::new());
  }

  let mut tasks = Vec::new();
  for entry in std::fs::read_dir(&dir)? {
    let entry = entry?;
    let path = entry.path();
    if path.extension().and_then(|e| e.to_str()) != Some("yaml") {
      continue;
    }
    let content = std::fs::read_to_string(&path)?;
    let task: Task = serde_yaml::from_str(&content)?;
    if task.status == WorkStatus::Pending {
      tasks.push((path, task));
    }
  }

  Ok(tasks)
}

pub fn set_task_status(path: &Path, status: WorkStatus) -> Result<()> {
  let content = std::fs::read_to_string(path)?;
  let mut task: Task = serde_yaml::from_str(&content)?;
  task.status = status;
  let content = serde_yaml::to_string(&task)?;
  std::fs::write(path, content)?;
  Ok(())
}
