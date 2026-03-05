use std::path::Path;

use crate::config::Config;
use crate::error::Result;
use crate::intent::registry::{Intent, IntentStatus};
use crate::prompt;

pub fn launch(_config: &Config, model: Option<&str>, repo_path: &Path) -> Result<()> {
  let mut cmd = std::process::Command::new("claude");
  cmd
    .arg("--append-system-prompt")
    .arg(prompt::OPERATOR)
    .arg("--allowedTools")
    .arg("Bash");

  if let Some(m) = model {
    cmd.arg("--model").arg(m);
  }

  let initial_message = build_initial_message(repo_path);
  cmd.arg(initial_message);

  use std::os::unix::process::CommandExt;
  let err = cmd.exec();
  Err(crate::error::ForgeError::Claude(format!(
    "exec failed: {err}"
  )))
}

pub fn build_initial_message(repo_path: &Path) -> String {
  let intents_dir = repo_path.join(".forge").join("intents");
  let intents = Intent::fetch_all(&intents_dir).unwrap_or_default();

  if intents.is_empty() {
    return "pfl-forge is ready. No intents found.".to_string();
  }

  let mut msg = String::from("pfl-forge is ready.\n\n## State Summary\n\n");

  // Count by status
  let mut proposed = 0usize;
  let mut approved = 0usize;
  let mut done = 0usize;
  let mut blocked = 0usize;
  let mut error = 0usize;

  for i in &intents {
    match i.status {
      IntentStatus::Proposed => proposed += 1,
      IntentStatus::Approved => approved += 1,
      IntentStatus::Done => done += 1,
      IntentStatus::Blocked => blocked += 1,
      IntentStatus::Error => error += 1,
    }
  }

  msg.push_str(&format!(
    "Total: {} intents (proposed: {}, approved: {}, done: {}, blocked: {}, error: {})\n",
    intents.len(),
    proposed,
    approved,
    done,
    blocked,
    error,
  ));

  // Inbox: proposed, blocked, error, needs_clarification
  let inbox: Vec<&Intent> = intents
    .iter()
    .filter(|i| {
      matches!(
        i.status,
        IntentStatus::Proposed | IntentStatus::Blocked | IntentStatus::Error
      ) || i.needs_clarification()
    })
    .collect();

  if !inbox.is_empty() {
    msg.push_str("\n## Inbox\n\n");
    for i in &inbox {
      let status = format!("{:?}", i.status).to_lowercase();
      let clarification = if i.needs_clarification() {
        " [needs clarification]"
      } else {
        ""
      };
      msg.push_str(&format!(
        "- **{}** ({}{}) — {}\n",
        i.id(),
        status,
        clarification,
        i.title
      ));
      for c in &i.clarifications {
        if c.answer.is_none() {
          msg.push_str(&format!("  - Q: {}\n", c.question));
        }
      }
    }
  }

  msg
}
