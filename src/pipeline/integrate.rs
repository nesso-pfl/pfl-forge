use tracing::{info, warn};

use crate::claude::runner::ClaudeRunner;
use crate::config::Config;
use crate::error::Result;
use crate::git;
use crate::pipeline::execute::ExecuteResult;
use crate::pipeline::review::{self, ReviewResult};
use crate::pipeline::triage::Task;
use crate::state::tracker::{IssueStatus, SharedState};
use crate::task::ForgeIssue;

fn write_review_yaml(worktree_path: &std::path::Path, result: &ReviewResult) -> Result<()> {
  let forge_dir = worktree_path.join(".forge");
  std::fs::create_dir_all(&forge_dir)?;
  let content = serde_yaml::to_string(result)?;
  std::fs::write(forge_dir.join("review.yaml"), content)?;
  Ok(())
}

pub struct WorkerOutput {
  pub issue: ForgeIssue,
  pub result: ExecuteResult,
  pub task: Task,
  pub task_path: std::path::PathBuf,
}

pub async fn integrate_one(
  output: &WorkerOutput,
  config: &Config,
  state: &SharedState,
) -> Result<()> {
  let issue = &output.issue;
  let repo_name = &issue.repo_name;
  let branch = issue.branch_name();
  let worktree_path = issue.worktree_path(&config.settings.worktree_dir);
  let base_branch = config.base_branch.clone();

  // Rebase onto latest base branch
  info!("rebasing {issue} onto {base_branch}");
  let wt = worktree_path.clone();
  let bb = base_branch.clone();
  let rebase_result = tokio::task::spawn_blocking(move || git::branch::rebase(&wt, &bb))
    .await
    .map_err(|e| crate::error::ForgeError::Git(format!("spawn_blocking: {e}")))?;

  if let Err(e) = rebase_result {
    warn!("rebase conflict for {issue}: {e}");
    info!("task {issue}: rebase conflict, branch {branch} left as-is");
    state
      .lock()
      .unwrap()
      .set_status(repo_name, issue.number, &issue.title, IssueStatus::Error)?;
    return Ok(());
  }

  // Re-run tests after rebase
  info!("re-running tests for {issue}");
  let wt = worktree_path.clone();
  let test_cmd = config.test_command.clone();
  let test_passed =
    tokio::task::spawn_blocking(move || crate::pipeline::execute::run_tests(&wt, &test_cmd))
      .await
      .map_err(|e| crate::error::ForgeError::Git(format!("spawn_blocking: {e}")))?;

  if !test_passed? {
    info!("task {issue}: tests failed after rebase, branch {branch} left as-is");
    state.lock().unwrap().set_status(
      repo_name,
      issue.number,
      &issue.title,
      IssueStatus::TestFailure,
    )?;
    return Ok(());
  }

  // Review
  info!("reviewing {issue}");
  let review_runner = ClaudeRunner::new(config.settings.triage_tools.clone());
  let issue_clone = issue.clone();
  let task_clone = output.task.clone();
  let config_clone = config.clone();
  let wt = worktree_path.clone();
  let bb = base_branch.clone();
  let review_result = tokio::task::spawn_blocking(move || {
    review::review(
      &issue_clone,
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
      info!("task {issue}: review rejected, branch {branch} left for manual review");
      state.lock().unwrap().set_status(
        repo_name,
        issue.number,
        &issue.title,
        IssueStatus::Error,
      )?;
      return Ok(());
    }
    Err(e) => {
      warn!("review failed for {issue}, proceeding anyway: {e}");
    }
    _ => {
      info!("review approved for {issue}");
    }
  }

  info!("task {issue} completed, branch {branch} available locally");
  Ok(())
}
