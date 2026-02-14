use crate::config::Config;
use crate::error::Result;
use crate::pipeline::clarification;
use crate::state::tracker::StateTracker;

pub fn build_initial_message(_config: &Config, state: &StateTracker) -> Result<String> {
  let summary = state.summary();

  let repo_path = Config::repo_path();
  let pending = clarification::list_pending_clarifications(&repo_path)?;

  let mut msg = format!("Current state: {summary}\n");

  if !pending.is_empty() {
    msg.push_str("\nThere are pending clarification questions:\n\n");
    for c in &pending {
      msg.push_str(&format!("### {}\n", c.issue_id));
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
