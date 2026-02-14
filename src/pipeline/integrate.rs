use std::path::Path;

use tracing::{info, warn};

use crate::agents::review::ReviewResult;
use crate::error::Result;
use crate::git;
use crate::task::ForgeTask;

/// Rebase the task branch onto the base branch.
/// Returns Ok(true) on success, Ok(false) on conflict.
pub fn rebase(forge_task: &ForgeTask, worktree_path: &Path, base_branch: &str) -> Result<bool> {
  let branch = forge_task.branch_name();

  info!("rebasing {forge_task} onto {base_branch}");
  match git::branch::rebase(worktree_path, base_branch) {
    Ok(()) => Ok(true),
    Err(e) => {
      warn!("rebase conflict for {forge_task}: {e}");
      info!("task {forge_task}: rebase conflict, branch {branch} left as-is");
      Ok(false)
    }
  }
}

pub fn write_review_yaml(worktree_path: &Path, result: &ReviewResult) -> Result<()> {
  let forge_dir = worktree_path.join(".forge");
  std::fs::create_dir_all(&forge_dir)?;
  let content = serde_yaml::to_string(result)?;
  std::fs::write(forge_dir.join("review.yaml"), content)?;
  Ok(())
}
