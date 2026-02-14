use tracing::{info, warn};

use crate::agents::review::{self, ReviewResult};
use crate::claude::runner::ClaudeRunner;
use crate::config::Config;
use crate::error::Result;
use crate::git;
use crate::pipeline::execute::ExecuteResult;
use crate::pipeline::work::Task;
use crate::state::tracker::{SharedState, TaskStatus};
use crate::task::ForgeTask;

fn write_review_yaml(worktree_path: &std::path::Path, result: &ReviewResult) -> Result<()> {
  let forge_dir = worktree_path.join(".forge");
  std::fs::create_dir_all(&forge_dir)?;
  let content = serde_yaml::to_string(result)?;
  std::fs::write(forge_dir.join("review.yaml"), content)?;
  Ok(())
}

pub struct ImplementOutput {
  pub forge_task: ForgeTask,
  pub result: ExecuteResult,
  pub task: Task,
  pub task_path: std::path::PathBuf,
}

pub async fn integrate_one(
  output: &ImplementOutput,
  config: &Config,
  state: &SharedState,
) -> Result<()> {
  let forge_task = &output.forge_task;
  let branch = forge_task.branch_name();
  let worktree_path = forge_task.worktree_path(&config.worktree_dir);
  let base_branch = config.base_branch.clone();

  // Rebase onto latest base branch
  info!("rebasing {forge_task} onto {base_branch}");
  let wt = worktree_path.clone();
  let bb = base_branch.clone();
  let rebase_result = tokio::task::spawn_blocking(move || git::branch::rebase(&wt, &bb))
    .await
    .map_err(|e| crate::error::ForgeError::Git(format!("spawn_blocking: {e}")))?;

  if let Err(e) = rebase_result {
    warn!("rebase conflict for {forge_task}: {e}");
    info!("task {forge_task}: rebase conflict, branch {branch} left as-is");
    state
      .lock()
      .unwrap()
      .set_status(&forge_task.id, &forge_task.title, TaskStatus::Error)?;
    return Ok(());
  }

  // Review
  info!("reviewing {forge_task}");
  let review_runner = ClaudeRunner::new(config.triage_tools.clone());
  let task_clone2 = forge_task.clone();
  let task_clone = output.task.clone();
  let config_clone = config.clone();
  let wt = worktree_path.clone();
  let bb = base_branch.clone();
  let review_result = tokio::task::spawn_blocking(move || {
    review::review(
      &task_clone2,
      &task_clone,
      &config_clone,
      &review_runner,
      &wt,
      &bb,
    )
  })
  .await
  .map_err(|e| crate::error::ForgeError::Claude(format!("spawn_blocking: {e}")))?;

  // Write review result to .forge/review.yaml
  if let Ok(ref result) = review_result {
    if let Err(e) = write_review_yaml(&worktree_path, result) {
      warn!("failed to write review.yaml: {e}");
    }
  }

  match review_result {
    Ok(result) if !result.approved => {
      info!("task {forge_task}: review rejected, branch {branch} left for manual review");
      state
        .lock()
        .unwrap()
        .set_status(&forge_task.id, &forge_task.title, TaskStatus::Error)?;
      return Ok(());
    }
    Err(e) => {
      warn!("review failed for {forge_task}, proceeding anyway: {e}");
    }
    _ => {
      info!("review approved for {forge_task}");
    }
  }

  info!("task {forge_task} completed, branch {branch} available locally");
  Ok(())
}
