use std::path::Path;
use std::time::Duration;

use tracing::info;

use crate::agent::review::ReviewResult;
use crate::claude::runner::{Claude, SessionMode};
use crate::intent::registry::Intent;
use crate::prompt;
use crate::task::Task;

pub fn run(
  intent: &Intent,
  task: &Task,
  runner: &impl Claude,
  selected_model: &str,
  worktree_path: &Path,
  timeout: Option<Duration>,
  review_feedback: Option<&ReviewResult>,
  session: &SessionMode,
) -> Result<String, crate::error::ForgeError> {
  let mut prompt = format!(
    "## Intent: {title}\n\n{body}\n\n## Task: {task_title}\n\n\
     **Complexity:** {complexity}\n\n\
     **Plan:**\n{plan}\n\n\
     **Relevant files:**\n{files}\n\n\
     **Steps:**\n{steps}",
    title = intent.title,
    body = intent.body,
    task_title = task.title,
    complexity = task.complexity,
    plan = task.plan,
    files = task
      .relevant_files
      .iter()
      .map(|f| format!("- {f}"))
      .collect::<Vec<_>>()
      .join("\n"),
    steps = task
      .implementation_steps
      .iter()
      .enumerate()
      .map(|(i, s)| format!("{}. {s}", i + 1))
      .collect::<Vec<_>>()
      .join("\n"),
  );

  if !task.context.is_empty() {
    prompt.push_str(&format!("\n\n**Context:**\n{}", task.context));
  }

  // Include clarifications if present
  if !intent.clarifications.is_empty() {
    let answered: Vec<_> = intent
      .clarifications
      .iter()
      .filter_map(|c| c.answer.as_ref().map(|a| (&c.question, a)))
      .collect();
    if !answered.is_empty() {
      prompt.push_str("\n\n## Clarifications\n");
      for (q, a) in answered {
        prompt.push_str(&format!("\n**Q:** {q}\n**A:** {a}\n"));
      }
    }
  }

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

  info!("implementing: {intent}");
  runner.run_prompt(
    &prompt,
    prompt::IMPLEMENT,
    selected_model,
    worktree_path,
    timeout,
    session,
  )
}
