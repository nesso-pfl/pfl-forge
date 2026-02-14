use std::path::Path;
use std::process::Command;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tracing::info;

use crate::claude::model;
use crate::claude::runner::ClaudeRunner;
use crate::config::Config;
use crate::error::{ForgeError, Result};
use crate::pipeline::triage::Task;
use crate::prompt;
use crate::task::ForgeTask;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewResult {
  pub approved: bool,
  pub issues: Vec<String>,
  pub suggestions: Vec<String>,
}

pub fn review(
  forge_task: &ForgeTask,
  task: &Task,
  config: &Config,
  runner: &ClaudeRunner,
  worktree_path: &Path,
  base_branch: &str,
) -> Result<ReviewResult> {
  let review_model = model::resolve(&config.settings.models.default);

  let diff = get_diff(worktree_path, base_branch)?;

  let prompt = format!(
    r#"## Issue {id}: {title}

{body}

## Implementation Plan

{plan}

## Diff

```
{diff}
```"#,
    id = forge_task.id,
    title = forge_task.title,
    body = forge_task.body,
    plan = task.plan,
    diff = truncate_diff(&diff, 50000),
  );

  let timeout = Some(Duration::from_secs(config.settings.triage_timeout_secs));

  info!("reviewing: {forge_task}");
  let result: ReviewResult = runner.run_json(
    &prompt,
    prompt::REVIEW,
    review_model,
    worktree_path,
    timeout,
  )?;

  info!(
    "review: approved={}, {} issues, {} suggestions",
    result.approved,
    result.issues.len(),
    result.suggestions.len(),
  );

  Ok(result)
}

fn get_diff(worktree_path: &Path, base_branch: &str) -> Result<String> {
  let output = Command::new("git")
    .args(["diff", &format!("origin/{base_branch}...HEAD")])
    .current_dir(worktree_path)
    .output()?;

  if !output.status.success() {
    let stderr = String::from_utf8_lossy(&output.stderr);
    return Err(ForgeError::Git(format!("diff failed: {stderr}")));
  }

  Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

fn truncate_diff(diff: &str, max_len: usize) -> &str {
  if diff.len() <= max_len {
    diff
  } else {
    &diff[..max_len]
  }
}
