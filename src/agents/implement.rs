use std::path::Path;
use std::time::Duration;

use tracing::info;

use crate::claude::runner::ClaudeRunner;
use crate::prompt;
use crate::task::ForgeTask;

pub fn run(
  forge_task: &ForgeTask,
  runner: &ClaudeRunner,
  selected_model: &str,
  worktree_path: &Path,
  timeout: Option<Duration>,
) -> Result<String, crate::error::ForgeError> {
  let prompt = format!(
    r#"## Task {id}: {title}

{body}"#,
    id = forge_task.id,
    title = forge_task.title,
    body = forge_task.body,
  );

  info!("implementing: {forge_task}");
  runner.run_prompt(
    &prompt,
    prompt::IMPLEMENT,
    selected_model,
    worktree_path,
    timeout,
  )
}
