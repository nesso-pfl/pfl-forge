use std::path::Path;
use std::time::Duration;

use tracing::{error, info};

use crate::claude::runner::ClaudeRunner;
use crate::config::RepoConfig;
use crate::error::Result;
use crate::git;
use crate::task::ForgeIssue;
use crate::pipeline::triage::Task;
use crate::prompt;

pub fn write_task_yaml(worktree_path: &Path, task: &Task) -> Result<()> {
  let forge_dir = worktree_path.join(".forge");
  std::fs::create_dir_all(&forge_dir)?;
  let content = serde_yaml::to_string(task)?;
  std::fs::write(forge_dir.join("task.yaml"), content)?;
  Ok(())
}

pub fn ensure_gitignore_forge(worktree_path: &Path) -> Result<()> {
  let gitignore = worktree_path.join(".gitignore");
  if gitignore.exists() {
    let content = std::fs::read_to_string(&gitignore)?;
    if content.lines().any(|line| line.trim() == ".forge/") {
      return Ok(());
    }
    let suffix = if content.ends_with('\n') { "" } else { "\n" };
    std::fs::write(&gitignore, format!("{content}{suffix}.forge/\n"))?;
  } else {
    std::fs::write(&gitignore, ".forge/\n")?;
  }
  Ok(())
}

#[derive(Debug)]
pub enum ExecuteResult {
  Success { commits: u32 },
  TestFailure { commits: u32, output: String },
  Unclear(String),
  Error(String),
}

pub fn execute(
  issue: &ForgeIssue,
  task: &Task,
  repo_config: &RepoConfig,
  runner: &ClaudeRunner,
  model_settings: &crate::config::ModelSettings,
  worktree_dir: &str,
  worker_timeout_secs: u64,
) -> Result<ExecuteResult> {
  let branch = issue.branch_name();
  let repo_path = &repo_config.path;

  // Create worktree
  let worktree_path =
    git::worktree::create(repo_path, worktree_dir, &branch, &repo_config.base_branch)?;

  info!("executing in worktree: {}", worktree_path.display());

  // Write task data and ensure .gitignore
  write_task_yaml(&worktree_path, task)?;
  ensure_gitignore_forge(&worktree_path)?;

  // Check Docker if required
  if repo_config.docker_required {
    if let Err(e) = check_docker(&worktree_path) {
      error!("docker check failed: {e}");
      return Ok(ExecuteResult::Error(format!("Docker not running: {e}")));
    }
  }

  // Select model based on complexity
  let complexity = task.complexity();
  let selected_model = complexity.select_model(model_settings);

  // Build the worker prompt
  let prompt = build_worker_prompt(issue, repo_config);

  // Run Claude Code Worker
  let timeout = Some(Duration::from_secs(worker_timeout_secs));
  let result = runner.run_prompt(
    &prompt,
    prompt::WORKER,
    selected_model,
    &worktree_path,
    timeout,
  );

  match result {
    Ok(output) => {
      // Check if there are commits
      let commits =
        git::branch::commit_count(&worktree_path, &repo_config.base_branch, "HEAD").unwrap_or(0);

      if commits == 0 {
        info!("no commits produced");
        return Ok(ExecuteResult::Unclear(
          "Worker completed but produced no commits".into(),
        ));
      }

      info!("{commits} commit(s) produced");

      // Run tests
      match run_tests(&worktree_path, &repo_config.test_command) {
        Ok(true) => Ok(ExecuteResult::Success { commits }),
        Ok(false) => Ok(ExecuteResult::TestFailure { commits, output }),
        Err(e) => Ok(ExecuteResult::TestFailure {
          commits,
          output: format!("Test execution error: {e}"),
        }),
      }
    }
    Err(e) => Ok(ExecuteResult::Error(e.to_string())),
  }
}

fn build_worker_prompt(issue: &ForgeIssue, repo_config: &RepoConfig) -> String {
  format!(
    r#"## Issue #{number}: {title}

{body}

## Test Command

`{test_command}`"#,
    number = issue.number,
    title = issue.title,
    body = issue.body,
    test_command = repo_config.test_command,
  )
}

fn check_docker(worktree_path: &Path) -> Result<()> {
  let output = std::process::Command::new("docker")
    .args(["compose", "ps", "--status", "running"])
    .current_dir(worktree_path)
    .output()?;

  if !output.status.success() {
    return Err(crate::error::ForgeError::Git(
      "docker compose is not running".into(),
    ));
  }
  Ok(())
}

pub fn run_tests(worktree_path: &Path, test_command: &str) -> Result<bool> {
  info!("running tests: {test_command}");

  let parts: Vec<&str> = test_command.split_whitespace().collect();
  let (cmd, args) = parts.split_first().expect("test_command is non-empty");

  let output = std::process::Command::new(cmd)
    .args(args)
    .current_dir(worktree_path)
    .output()?;

  Ok(output.status.success())
}
