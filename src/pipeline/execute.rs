use std::path::{Path, PathBuf};

use tracing::info;

use crate::config::Config;
use crate::error::Result;
use crate::git;
use crate::pipeline::work::Task;
use crate::task::ForgeTask;

pub fn write_task_yaml(worktree_path: &Path, task: &Task) -> Result<()> {
  let forge_dir = worktree_path.join(".forge");
  std::fs::create_dir_all(&forge_dir)?;
  let content = serde_yaml::to_string(task)?;
  std::fs::write(forge_dir.join("task.yaml"), content)?;
  Ok(())
}

pub fn ensure_gitignore_forge(worktree_path: &Path) -> Result<()> {
  let gitignore = worktree_path.join(".gitignore");
  if gitignore.exists() {
    let content = std::fs::read_to_string(&gitignore)?;
    if content.lines().any(|line| line.trim() == ".forge/") {
      return Ok(());
    }
    let suffix = if content.ends_with('\n') { "" } else { "\n" };
    std::fs::write(&gitignore, format!("{content}{suffix}.forge/\n"))?;
  } else {
    std::fs::write(&gitignore, ".forge/\n")?;
  }
  Ok(())
}

#[derive(Debug)]
pub enum ExecuteResult {
  Success { commits: u32 },
  Unclear(String),
  Error(String),
}

pub struct PrepareResult {
  pub worktree_path: PathBuf,
  pub selected_model: String,
}

/// Prepare worktree for implementation: create worktree, write task YAML, select model.
pub fn prepare(
  forge_task: &ForgeTask,
  task: &Task,
  config: &Config,
  worktree_dir: &str,
) -> Result<PrepareResult> {
  let branch = forge_task.branch_name();
  let repo_path = Config::repo_path();

  let worktree_path =
    git::worktree::create(&repo_path, worktree_dir, &branch, &config.base_branch)?;

  info!("prepared worktree: {}", worktree_path.display());

  write_task_yaml(&worktree_path, task)?;
  ensure_gitignore_forge(&worktree_path)?;

  let complexity = task.complexity();
  let selected_model = complexity.select_model(&config.models).to_string();

  Ok(PrepareResult {
    worktree_path,
    selected_model,
  })
}
