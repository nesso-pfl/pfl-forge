use std::path::Path;
use std::time::Duration;

use tracing::{error, info};

use crate::claude::runner::ClaudeRunner;
use crate::config::RepoConfig;
use crate::error::Result;
use crate::git;
use crate::github::issue::ForgeIssue;
use crate::pipeline::triage::DeepTriageResult;

#[derive(Debug)]
pub enum ExecuteResult {
    Success { commits: u32 },
    TestFailure { commits: u32, output: String },
    Unclear(String),
    Error(String),
}

pub fn execute(
    issue: &ForgeIssue,
    deep: &DeepTriageResult,
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

    // Check Docker if required
    if repo_config.docker_required {
        if let Err(e) = check_docker(&worktree_path) {
            error!("docker check failed: {e}");
            return Ok(ExecuteResult::Error(format!("Docker not running: {e}")));
        }
    }

    // Select model based on complexity
    let complexity = deep.complexity();
    let selected_model = complexity.select_model(model_settings);

    // Build the worker prompt
    let prompt = build_worker_prompt(issue, deep, repo_config);

    // Run Claude Code Worker
    let timeout = Some(Duration::from_secs(worker_timeout_secs));
    let result = runner.run_prompt(&prompt, selected_model, &worktree_path, timeout);

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
    deep: &DeepTriageResult,
    repo_config: &RepoConfig,
) -> String {
    let files = deep
        .relevant_files
        .iter()
        .map(|f| format!("- {f}"))
        .collect::<Vec<_>>()
        .join("\n");

    let steps = deep
        .implementation_steps
        .iter()
        .enumerate()
        .map(|(i, s)| format!("{}. {s}", i + 1))
        .collect::<Vec<_>>()
        .join("\n");

    format!(
        r#"You are a coding agent. Implement the following GitHub issue according to the provided implementation plan.

## Issue #{number}: {title}

{body}

## Implementation Plan

{plan}

## Relevant Files

{files}

## Implementation Steps

{steps}

## Codebase Context

{context}

## Instructions

1. Follow the implementation plan and steps above
2. Modify the relevant files as described
3. Run the test command: `{test_command}`
4. Commit your changes with a descriptive message referencing the issue: "fix #{number}: <description>"
5. Make sure all tests pass before committing

Do NOT push to remote. Just commit locally."#,
        number = issue.number,
        title = issue.title,
        body = issue.body,
        plan = deep.plan,
        files = files,
        steps = steps,
        context = deep.context,
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
