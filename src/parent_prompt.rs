use crate::config::Config;
use crate::error::Result;
use crate::pipeline::clarification;
use crate::state::tracker::StateTracker;

pub fn build_system_prompt(config: &Config) -> String {
  let repos_info: Vec<String> = config
    .repos
    .iter()
    .map(|r| {
      format!(
        "- {name}: path={path}, test=`{test}`, base={base}",
        name = r.name,
        path = r.path.display(),
        test = r.test_command,
        base = r.base_branch,
      )
    })
    .collect();

  format!(
    r#"You are the parent agent for pfl-forge, a multi-agent task processor.
You manage task processing by running pfl-forge CLI commands via Bash.

## Available commands

- `pfl-forge run` — Process pending tasks (fetch, triage, execute, integrate)
  - `--repo <name>` — Process only a specific repo
  - `--resume` — Resume failed/interrupted tasks
  - `--dry-run` — Triage only, don't execute
- `pfl-forge status` — Show current processing state
- `pfl-forge clarifications` — List unanswered clarification questions
- `pfl-forge answer <number> "<text>"` — Answer a clarification question
  - `--repo <name>` — Specify repo (auto-detected if omitted)
- `pfl-forge clean` — Clean up worktrees for completed tasks
- `pfl-forge watch` — Daemon mode: poll and process periodically

## Clarification workflow

When a worker cannot resolve a task due to ambiguity, it creates a clarification request.
Use `pfl-forge clarifications` to see pending questions, then discuss with the user and
use `pfl-forge answer <number> "<text>"` to record the answer.
After answering, the task is reset to pending and will be re-processed on the next `pfl-forge run`.

## Configured repos

{repos}

## Guidelines

- Always check `pfl-forge status` before running to understand the current state.
- Present clarification questions to the user in a clear, conversational way.
- After the user answers, record it with `pfl-forge answer` and run processing again.
- Report results back to the user clearly."#,
    repos = repos_info.join("\n"),
  )
}

pub fn build_initial_message(config: &Config, state: &StateTracker) -> Result<String> {
  let summary = state.summary();

  let repos: Vec<(String, &std::path::Path)> = config
    .repos
    .iter()
    .map(|r| (r.name.clone(), r.path.as_path()))
    .collect();
  let pending = clarification::list_pending_clarifications(&repos)?;

  let mut msg = format!("Current state: {summary}\n");

  if !pending.is_empty() {
    msg.push_str("\nThere are pending clarification questions:\n\n");
    for c in &pending {
      msg.push_str(&format!("### {} #{}\n", c.repo_name, c.issue_number));
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
