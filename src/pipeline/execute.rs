use std::path::Path;

use tracing::{error, info};

use crate::claude::runner::ClaudeRunner;
use crate::config::RepoConfig;
use crate::error::Result;
use crate::git;
use crate::github::issue::ForgeIssue;
use crate::pipeline::triage::TriageResult;

#[derive(Debug)]
pub enum ExecuteResult {
    Success { commits: u32 },
    TestFailure { commits: u32, output: String },
    Unclear(String),
    Error(String),
}

pub fn execute(
    issue: &ForgeIssue,
    triage: &TriageResult,
    repo_config: &RepoConfig,
    runner: &ClaudeRunner,
    model_settings: &crate::config::ModelSettings,
    worktree_dir: &str,
) -> Result<ExecuteResult> {
    let branch = issue.branch_name();
    let repo_path = &repo_config.path;

    // Create worktree
    let worktree_path =
        git::worktree::create(repo_path, worktree_dir, &branch, &repo_config.base_branch)?;

    info!("executing in worktree: {}", worktree_path.display());

    // Check Docker if required
    if repo_config.docker_required {
        if let Err(e) = check_docker(&worktree_path) {
            error!("docker check failed: {e}");
            return Ok(ExecuteResult::Error(format!("Docker not running: {e}")));
        }
    }

    // Select model based on complexity
    let complexity = triage.complexity();
    let selected_model = complexity.select_model(model_settings);

    // Build the worker prompt
    let prompt = build_worker_prompt(issue, triage, repo_config);

    // Run Claude Code Worker
    let result = runner.run_prompt(&prompt, selected_model, &worktree_path);

    match result {
        Ok(output) => {
            // Check if there are commits
            let commits = git::branch::commit_count(
                &worktree_path,
                &repo_config.base_branch,
                "HEAD",
            )
            .unwrap_or(0);

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
                Ok(false) => Ok(ExecuteResult::TestFailure {
                    commits,
                    output,
                }),
                Err(e) => Ok(ExecuteResult::TestFailure {
                    commits,
                    output: format!("Test execution error: {e}"),
                }),
            }
        }
        Err(e) => Ok(ExecuteResult::Error(e.to_string())),
    }
}

fn build_worker_prompt(
    issue: &ForgeIssue,
    triage: &TriageResult,
    repo_config: &RepoConfig,
) -> String {
    format!(
        r#"You are a coding agent. Implement the following GitHub issue.

## Issue #{number}: {title}

{body}

## Triage Analysis
- Summary: {summary}
- Plan: {plan}

## Instructions
1. Read and understand the relevant code in this repository
2. Implement the changes needed to resolve this issue
3. Run the test command: `{test_command}`
4. Commit your changes with a descriptive message referencing the issue: "fix #{number}: <description>"
5. Make sure all tests pass before committing

Do NOT push to remote. Just commit locally."#,
        number = issue.number,
        title = issue.title,
        body = issue.body,
        summary = triage.summary,
        plan = triage.plan,
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
