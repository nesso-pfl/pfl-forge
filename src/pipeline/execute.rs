use std::path::Path;
use std::time::Duration;

use tracing::info;

use crate::agents::worker;
use crate::claude::runner::ClaudeRunner;
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

pub fn execute(
  forge_task: &ForgeTask,
  task: &Task,
  config: &Config,
  runner: &ClaudeRunner,
  model_settings: &crate::config::ModelSettings,
  worktree_dir: &str,
  worker_timeout_secs: u64,
) -> Result<ExecuteResult> {
  let branch = forge_task.branch_name();
  let repo_path = Config::repo_path();

  // Create worktree
  let worktree_path =
    git::worktree::create(&repo_path, worktree_dir, &branch, &config.base_branch)?;

  info!("executing in worktree: {}", worktree_path.display());

  // Write task data and ensure .gitignore
  write_task_yaml(&worktree_path, task)?;
  ensure_gitignore_forge(&worktree_path)?;

  // Select model based on complexity
  let complexity = task.complexity();
  let selected_model = complexity.select_model(model_settings);

  // Run Claude Code Worker
  let timeout = Some(Duration::from_secs(worker_timeout_secs));
  let result = worker::run_worker(forge_task, runner, selected_model, &worktree_path, timeout);

  match result {
    Ok(_output) => {
      let commits =
        git::branch::commit_count(&worktree_path, &config.base_branch, "HEAD").unwrap_or(0);

      if commits == 0 {
        info!("no commits produced");
        return Ok(ExecuteResult::Unclear(
          "Worker completed but produced no commits".into(),
        ));
      }

      info!("{commits} commit(s) produced");
      Ok(ExecuteResult::Success { commits })
    }
    Err(e) => Ok(ExecuteResult::Error(e.to_string())),
  }
}
