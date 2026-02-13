mod claude;
mod config;
mod error;
mod git;
mod github;
mod parent_prompt;
mod pipeline;
mod state;

use std::path::PathBuf;
use std::sync::Arc;

use clap::{Parser, Subcommand};
use tokio::sync::Semaphore;
use tokio::task::JoinSet;
use tracing::{error, info, warn};

use crate::claude::runner::ClaudeRunner;
use crate::config::Config;
use crate::error::Result;
use crate::github::client::GitHubClient;
use crate::github::issue::ForgeIssue;
use crate::pipeline::execute::ExecuteResult;
use crate::pipeline::integrate::WorkerOutput;
use crate::pipeline::triage::{self, ConsultationOutcome, DeepTriageResult};
use crate::state::tracker::{IssueStatus, SharedState, StateTracker};

#[derive(Parser)]
#[command(name = "pfl-forge", about = "Multi-agent issue processor powered by Claude Code")]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    /// Path to config file
    #[arg(short, long, default_value = "pfl-forge.yaml")]
    config: PathBuf,
}

#[derive(Subcommand)]
enum Commands {
    /// Process issues: fetch, triage, execute, report
    Run {
        /// Only triage, don't execute
        #[arg(long)]
        dry_run: bool,

        /// Process only a specific repo
        #[arg(long)]
        repo: Option<String>,

        /// Resume failed/interrupted issues
        #[arg(long)]
        resume: bool,
    },
    /// Watch for new issues and process them periodically
    Watch {
        /// Process only a specific repo
        #[arg(long)]
        repo: Option<String>,
    },
    /// Show current processing status
    Status,
    /// Clean up worktrees for completed issues
    Clean {
        /// Specific repo to clean
        #[arg(long)]
        repo: Option<String>,
    },
    /// List pending clarifications
    Clarifications,
    /// Answer a clarification question
    Answer {
        /// Issue number
        number: u64,
        /// Answer text
        text: String,
        /// Specific repo (auto-detected if omitted)
        #[arg(long)]
        repo: Option<String>,
    },
    /// Launch parent agent (interactive Claude Code session)
    Parent {
        /// Process only a specific repo
        #[arg(long)]
        repo: Option<String>,
        /// Claude model to use
        #[arg(long)]
        model: Option<String>,
    },
}

enum ProcessOutcome {
    Output(WorkerOutput),
    Skipped,
    NeedsClarification {
        issue: ForgeIssue,
        message: String,
        deep_result: DeepTriageResult,
        repo_path: PathBuf,
    },
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let cli = Cli::parse();

    if let Err(e) = run(cli).await {
        error!("{e}");
        std::process::exit(1);
    }
}

async fn run(cli: Cli) -> Result<()> {
    let config = Config::load(&cli.config)?;

    match cli.command {
        Commands::Run {
            dry_run,
            repo,
            resume,
        } => cmd_run(&config, dry_run, repo.as_deref(), resume).await,
        Commands::Watch { repo } => cmd_watch(&config, repo.as_deref()).await,
        Commands::Status => cmd_status(&config),
        Commands::Clean { repo } => cmd_clean(&config, repo.as_deref()),
        Commands::Clarifications => cmd_clarifications(&config),
        Commands::Answer { number, text, repo } => cmd_answer(&config, number, repo.as_deref(), &text),
        Commands::Parent { repo, model } => cmd_parent(&config, repo.as_deref(), model.as_deref()),
    }
}

async fn cmd_run(
    config: &Config,
    dry_run: bool,
    repo_filter: Option<&str>,
    resume: bool,
) -> Result<()> {
    let github = GitHubClient::new()?;
    let state = StateTracker::load(&config.settings.state_file)?.into_shared();

    // Fetch new issues
    let mut issues = {
        let s = state.lock().unwrap();
        pipeline::fetch::fetch_issues(config, &github, &s).await?
    };

    // Fetch resumable issues if --resume
    if resume {
        let resumable = {
            let s = state.lock().unwrap();
            pipeline::fetch::fetch_resumable_issues(config, &github, &s).await?
        };
        for issue in resumable {
            if !issues.iter().any(|i| i.number == issue.number && i.full_repo() == issue.full_repo()) {
                issues.push(issue);
            }
        }
    }

    // Fetch issues that received clarification answers
    {
        let clarified = {
            let s = state.lock().unwrap();
            pipeline::fetch::fetch_clarified_issues(config, &github, &s).await?
        };
        for issue in clarified {
            if !issues.iter().any(|i| i.number == issue.number && i.full_repo() == issue.full_repo()) {
                issues.push(issue);
            }
        }
    }

    // Apply repo filter
    if let Some(filter) = repo_filter {
        issues.retain(|i| i.repo_name == filter);
    }

    if issues.is_empty() {
        info!("no issues to process");
        return Ok(());
    }

    info!("processing {} issue(s)", issues.len());

    if dry_run {
        return cmd_run_dry(config, &issues).await;
    }

    // Parallel phase: triage → execute
    let semaphore = Arc::new(Semaphore::new(config.settings.parallel_workers));
    let mut join_set = JoinSet::new();

    for issue in issues {
        let sem = semaphore.clone();
        let state = state.clone();
        let config = config.clone();

        join_set.spawn(async move {
            let _permit = sem.acquire().await.expect("semaphore closed");
            process_issue(issue, &config, &state).await
        });
    }

    // Streaming integration: process each result as it completes
    while let Some(result) = join_set.join_next().await {
        match result {
            Ok(Ok(ProcessOutcome::Output(output))) => {
                let repo_config = config
                    .find_repo(&output.repo_config_name)
                    .expect("repo config should exist");

                if matches!(output.result, ExecuteResult::Success { .. }) {
                    if let Err(e) =
                        pipeline::integrate::integrate_one(&output, repo_config, config, &github, &state).await
                    {
                        error!("integration failed for {}: {e}", output.issue);
                        state
                            .lock()
                            .unwrap()
                            .set_error(&output.issue.full_repo(), output.issue.number, &e.to_string())?;
                    }
                } else {
                    if let Err(e) = pipeline::report::report(
                        &output.issue,
                        &output.result,
                        repo_config,
                        &github,
                        &state,
                        &config.settings.worktree_dir,
                    )
                    .await
                    {
                        error!("report failed for {}: {e}", output.issue);
                    }
                }
            }
            Ok(Ok(ProcessOutcome::NeedsClarification {
                issue,
                message,
                deep_result,
                repo_path,
            })) => {
                if let Err(e) = pipeline::clarification::write_clarification(
                    &repo_path,
                    &issue,
                    &deep_result,
                    &message,
                ) {
                    error!("failed to write clarification for {issue}: {e}");
                }
            }
            Ok(Ok(ProcessOutcome::Skipped)) => {}
            Ok(Err(e)) => error!("worker error: {e}"),
            Err(e) => error!("task join error: {e}"),
        }
    }

    let summary = state.lock().unwrap().summary();
    info!("run complete: {summary}");

    Ok(())
}

async fn process_issue(
    issue: ForgeIssue,
    config: &Config,
    state: &SharedState,
) -> Result<ProcessOutcome> {
    let repo_config = config
        .find_repo(&issue.repo_name)
        .expect("issue repo should be in config");
    let full_repo = issue.full_repo();
    let repo_config_name = issue.repo_name.clone();

    {
        let mut s = state.lock().unwrap();
        s.set_status(&full_repo, issue.number, &issue.title, IssueStatus::Triaging)?;
        s.set_started(&full_repo, issue.number)?;
    }

    // Check for clarification context from previous NeedsClarification
    let repo_path = repo_config.path.clone();
    let clarification_ctx = pipeline::clarification::check_clarification(&repo_path, issue.number)?;

    // Deep Triage (sonnet, read-only tools)
    let deep_runner = ClaudeRunner::new(config.settings.triage_tools.clone());
    let issue_clone = issue.clone();
    let config_clone = config.clone();
    let repo_path_clone = repo_path.clone();
    let deep_result = tokio::task::spawn_blocking(move || {
        triage::deep_triage(
            &issue_clone,
            &config_clone,
            &deep_runner,
            &repo_path_clone,
            clarification_ctx.as_ref(),
        )
    })
    .await
    .map_err(|e| crate::error::ForgeError::Claude(format!("spawn_blocking: {e}")))??;

    // If deep triage is insufficient, consult
    let deep_result = if deep_result.is_sufficient() {
        deep_result
    } else {
        info!("deep triage insufficient for {issue}, consulting...");
        let consult_runner = ClaudeRunner::new(config.settings.triage_tools.clone());
        let issue_clone = issue.clone();
        let deep_clone = deep_result.clone();
        let config_clone = config.clone();
        let repo_path_clone = repo_path.clone();
        let outcome = tokio::task::spawn_blocking(move || {
            triage::consult(
                &issue_clone,
                &deep_clone,
                &config_clone,
                &consult_runner,
                &repo_path_clone,
            )
        })
        .await
        .map_err(|e| crate::error::ForgeError::Claude(format!("spawn_blocking: {e}")))??;

        match outcome {
            ConsultationOutcome::Resolved(result) => result,
            ConsultationOutcome::NeedsClarification(message) => {
                info!("consultation needs clarification for {issue}");
                state.lock().unwrap().set_status(
                    &full_repo,
                    issue.number,
                    &issue.title,
                    IssueStatus::NeedsClarification,
                )?;
                return Ok(ProcessOutcome::NeedsClarification {
                    issue,
                    message,
                    deep_result,
                    repo_path,
                });
            }
        }
    };

    // Execute
    {
        let mut s = state.lock().unwrap();
        s.set_status(&full_repo, issue.number, &issue.title, IssueStatus::Executing)?;
        s.set_branch(&full_repo, issue.number, &issue.branch_name())?;
    }

    let tools = repo_config.all_tools(&config.settings.worker_tools);
    let exec_runner = ClaudeRunner::new(tools);
    let issue_clone = issue.clone();
    let deep_clone = deep_result.clone();
    let repo_config_clone = repo_config.clone();
    let models = config.settings.models.clone();
    let worktree_dir = config.settings.worktree_dir.clone();

    let exec_result = tokio::task::spawn_blocking(move || {
        pipeline::execute::execute(
            &issue_clone,
            &deep_clone,
            &repo_config_clone,
            &exec_runner,
            &models,
            &worktree_dir,
        )
    })
    .await
    .map_err(|e| crate::error::ForgeError::Claude(format!("spawn_blocking: {e}")))??;

    // Update state for success and clean up clarification files
    if matches!(exec_result, ExecuteResult::Success { .. }) {
        state.lock().unwrap().set_status(
            &full_repo,
            issue.number,
            &issue.title,
            IssueStatus::Success,
        )?;
        let _ = pipeline::clarification::cleanup_clarification(&repo_path, issue.number);
    }

    Ok(ProcessOutcome::Output(WorkerOutput {
        issue,
        result: exec_result,
        repo_config_name,
        deep_triage: deep_result,
    }))
}

async fn cmd_run_dry(config: &Config, issues: &[ForgeIssue]) -> Result<()> {
    let deep_runner = ClaudeRunner::new(config.settings.triage_tools.clone());

    for issue in issues {
        let repo_config = config
            .find_repo(&issue.repo_name)
            .expect("issue repo should be in config");

        let deep = triage::deep_triage(issue, config, &deep_runner, &repo_config.path, None)?;

        println!("--- {} ---", issue);
        println!("Complexity: {}", deep.complexity);
        println!("Plan:       {}", deep.plan);
        println!("Files:      {}", deep.relevant_files.join(", "));
        println!("Steps:      {}", deep.implementation_steps.len());
        println!("Sufficient: {}", deep.is_sufficient());
        println!();
    }

    Ok(())
}

async fn cmd_watch(config: &Config, repo_filter: Option<&str>) -> Result<()> {
    let interval = std::time::Duration::from_secs(config.settings.poll_interval_secs);

    loop {
        info!("polling for new issues...");

        if let Err(e) = cmd_run(config, false, repo_filter, true).await {
            warn!("run error (will retry): {e}");
        }

        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                info!("shutting down");
                return Ok(());
            }
            _ = tokio::time::sleep(interval) => {}
        }
    }
}

fn cmd_status(config: &Config) -> Result<()> {
    let state = StateTracker::load(&config.settings.state_file)?;
    let summary = state.summary();

    println!("pfl-forge status");
    println!("================");
    println!("{summary}");
    println!();

    for (key, issue_state) in state.all_issues() {
        let status = format!("{:?}", issue_state.status);
        let pr = issue_state
            .pr_number
            .map(|n| format!(" -> PR #{n}"))
            .unwrap_or_default();
        let err = issue_state
            .error
            .as_ref()
            .map(|e| format!(" ({e})"))
            .unwrap_or_default();
        println!("  {key}: {status}{pr}{err}");
    }

    Ok(())
}

fn cmd_clean(config: &Config, repo_filter: Option<&str>) -> Result<()> {
    let state = StateTracker::load(&config.settings.state_file)?;

    for repo in &config.repos {
        if let Some(filter) = repo_filter {
            if repo.name != filter {
                continue;
            }
        }

        let worktrees = git::worktree::list(&repo.path)?;
        let worktree_prefix = format!(
            "{}/forge/",
            repo.path.join(&config.settings.worktree_dir).display()
        );

        for wt in &worktrees {
            if !wt.starts_with(&worktree_prefix) {
                continue;
            }

            if let Some(num_str) = wt.rsplit("issue-").next() {
                if let Ok(num) = num_str.parse::<u64>() {
                    let (owner, repo_name) = repo.owner_repo();
                    let full_repo = format!("{owner}/{repo_name}");

                    if state.is_processed(&full_repo, num) {
                        info!("cleaning worktree: {wt}");
                        git::worktree::remove(&repo.path, std::path::Path::new(wt))?;
                    }
                }
            }
        }
    }

    Ok(())
}

fn cmd_clarifications(config: &Config) -> Result<()> {
    let repos: Vec<(String, &std::path::Path)> = config
        .repos
        .iter()
        .map(|r| (r.name.clone(), r.path.as_path()))
        .collect();

    let pending = pipeline::clarification::list_pending_clarifications(&repos)?;

    if pending.is_empty() {
        println!("No pending clarifications.");
        return Ok(());
    }

    for c in &pending {
        println!("=== {} #{} ===", c.repo_name, c.issue_number);
        println!("{}", c.content);
        println!();
    }

    Ok(())
}

fn cmd_answer(config: &Config, number: u64, repo_filter: Option<&str>, text: &str) -> Result<()> {
    let repo = if let Some(filter) = repo_filter {
        config
            .find_repo(filter)
            .ok_or_else(|| crate::error::ForgeError::Config(format!("repo not found: {filter}")))?
    } else {
        let repos: Vec<(&str, &std::path::Path)> = config
            .repos
            .iter()
            .map(|r| (r.name.as_str(), r.path.as_path()))
            .collect();
        let (name, _) = pipeline::clarification::find_repo_for_issue(&repos, number)
            .ok_or_else(|| {
                crate::error::ForgeError::Config(format!(
                    "no clarification found for issue #{number}"
                ))
            })?;
        config.find_repo(name).unwrap()
    };

    pipeline::clarification::write_answer(&repo.path, number, text)?;

    let (owner, repo_name) = repo.owner_repo();
    let full_repo = format!("{owner}/{repo_name}");
    let mut state = StateTracker::load(&config.settings.state_file)?;
    state.reset_to_pending(&full_repo, number)?;

    println!("Answered clarification for #{number} and reset to pending.");
    Ok(())
}

fn cmd_parent(config: &Config, repo_filter: Option<&str>, model: Option<&str>) -> Result<()> {
    let state = StateTracker::load(&config.settings.state_file)?;

    let system_prompt = parent_prompt::build_system_prompt(config);
    let initial_message = parent_prompt::build_initial_message(config, &state)?;

    let mut cmd = std::process::Command::new("claude");
    cmd.arg("--append-system-prompt")
        .arg(&system_prompt)
        .arg("--allowedTools")
        .arg("Bash");

    if let Some(m) = model {
        cmd.arg("--model").arg(m);
    }

    if let Some(filter) = repo_filter {
        let repo = config
            .find_repo(filter)
            .ok_or_else(|| crate::error::ForgeError::Config(format!("repo not found: {filter}")))?;
        cmd.current_dir(&repo.path);
    }

    cmd.arg(&initial_message);

    use std::os::unix::process::CommandExt;
    let err = cmd.exec();
    Err(crate::error::ForgeError::Claude(format!("exec failed: {err}")))
}
