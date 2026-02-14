use std::path::Path;
use std::time::Duration;

use tracing::info;

use crate::agents::review::ReviewResult;
use crate::claude::runner::ClaudeRunner;
use crate::prompt;
use crate::task::ForgeTask;

pub fn run(
  forge_task: &ForgeTask,
  runner: &ClaudeRunner,
  selected_model: &str,
  worktree_path: &Path,
  timeout: Option<Duration>,
  review_feedback: Option<&ReviewResult>,
) -> Result<String, crate::error::ForgeError> {
  let mut prompt = format!(
    r#"## Task {id}: {title}

{body}"#,
    id = forge_task.id,
    title = forge_task.title,
    body = forge_task.body,
  );

  if let Some(review) = review_feedback {
    prompt.push_str("\n\n## Previous Review Feedback\n\nThe previous implementation was rejected. Address the following:\n");
    if !review.issues.is_empty() {
      prompt.push_str("\n### Issues\n");
      for issue in &review.issues {
        prompt.push_str(&format!("- {issue}\n"));
      }
    }
    if !review.suggestions.is_empty() {
      prompt.push_str("\n### Suggestions\n");
      for suggestion in &review.suggestions {
        prompt.push_str(&format!("- {suggestion}\n"));
      }
    }
  }

  info!("implementing: {forge_task}");
  runner.run_prompt(
    &prompt,
    prompt::IMPLEMENT,
    selected_model,
    worktree_path,
    timeout,
  )
}
