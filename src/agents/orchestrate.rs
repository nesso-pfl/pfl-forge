use crate::config::Config;
use crate::error::Result;
use crate::pipeline::clarification;
use crate::prompt;
use crate::state::tracker::StateTracker;

pub fn launch(config: &Config, model: Option<&str>) -> Result<()> {
  let state = StateTracker::load(&config.state_file)?;
  let initial_message = build_initial_message(config, &state)?;

  let mut cmd = std::process::Command::new("claude");
  cmd
    .arg("--append-system-prompt")
    .arg(prompt::ORCHESTRATE)
    .arg("--allowedTools")
    .arg("Bash");

  if let Some(m) = model {
    cmd.arg("--model").arg(m);
  }

  cmd.arg(&initial_message);

  use std::os::unix::process::CommandExt;
  let err = cmd.exec();
  Err(crate::error::ForgeError::Claude(format!(
    "exec failed: {err}"
  )))
}

fn build_initial_message(_config: &Config, state: &StateTracker) -> Result<String> {
  let summary = state.summary();

  let repo_path = Config::repo_path();
  let pending = clarification::list_pending_clarifications(&repo_path)?;

  let mut msg = format!("Current state: {summary}\n");

  if !pending.is_empty() {
    msg.push_str("\nThere are pending clarification questions:\n\n");
    for c in &pending {
      msg.push_str(&format!("### {}\n", c.task_id));
      let questions = extract_questions(&c.content);
      msg.push_str(&questions);
      msg.push('\n');
    }
    msg.push_str("\nPlease present these questions to the user and help resolve them.");
  } else {
    msg.push_str(
      "\nNo pending clarifications. You can run `pfl-forge run` to process new tasks or check status.",
    );
  }

  Ok(msg)
}

fn extract_questions(content: &str) -> String {
  let mut in_questions = false;
  let mut result = String::new();

  for line in content.lines() {
    if line.starts_with("## Questions") {
      in_questions = true;
      continue;
    }
    if in_questions && line.starts_with("## ") {
      break;
    }
    if in_questions {
      if !result.is_empty() {
        result.push('\n');
      }
      result.push_str(line);
    }
  }

  result
}
