mod claude;
mod config;
mod error;
mod git;
mod github;
mod pipeline;
mod state;

use std::path::PathBuf;

use clap::{Parser, Subcommand};
use tracing::{error, info};

use crate::claude::runner::ClaudeRunner;
use crate::config::Config;
use crate::error::Result;
use crate::github::client::GitHubClient;
use crate::pipeline::triage;
use crate::state::tracker::{IssueStatus, StateTracker};

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
    },
    /// Show current processing status
    Status,
    /// Clean up worktrees for completed issues
    Clean {
        /// Specific repo to clean
        #[arg(long)]
        repo: Option<String>,
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
        Commands::Run { dry_run, repo } => cmd_run(&config, dry_run, repo.as_deref()).await,
        Commands::Status => cmd_status(&config),
        Commands::Clean { repo } => cmd_clean(&config, repo.as_deref()),
    }
}

async fn cmd_run(config: &Config, dry_run: bool, repo_filter: Option<&str>) -> Result<()> {
    let github = GitHubClient::new()?;
    let mut state = StateTracker::load(&config.settings.state_file)?;

    let runner = ClaudeRunner::new(config.settings.worker_tools.clone());

    // Fetch issues
    let issues = pipeline::fetch::fetch_issues(config, &github, &state).await?;

    if issues.is_empty() {
        info!("no new issues to process");
        return Ok(());
    }

    info!("processing {} issue(s)", issues.len());

    for issue in &issues {
        // Filter by repo if specified
        if let Some(filter) = repo_filter {
            if issue.repo_name != filter {
                continue;
            }
        }

        let repo_config = config
            .find_repo(&issue.repo_name)
            .expect("issue repo should be in config");

        let full_repo = issue.full_repo();

        // Triage
        state.set_status(&full_repo, issue.number, &issue.title, IssueStatus::Triaging)?;
        state.set_started(&full_repo, issue.number)?;

        let triage_result = triage::triage(issue, config, &runner, &repo_config.path)?;

        if triage_result.should_skip() {
            info!("skipping non-actionable: {issue}");
            state.set_status(&full_repo, issue.number, &issue.title, IssueStatus::Skipped)?;
            continue;
        }

        if triage_result.needs_clarification() {
            info!("needs clarification: {issue}");
            github
                .add_comment(
                    &issue.owner,
                    &issue.repo,
                    issue.number,
                    &format!(
                        "pfl-forge needs more information to process this issue.\n\nTriage summary: {}",
                        triage_result.summary
                    ),
                )
                .await?;
            github
                .add_label(
                    &issue.owner,
                    &issue.repo,
                    issue.number,
                    &["forge-needs-clarification".to_string()],
                )
                .await?;
            state.set_status(
                &full_repo,
                issue.number,
                &issue.title,
                IssueStatus::NeedsClarification,
            )?;
            continue;
        }

        if dry_run {
            println!("--- {} ---", issue);
            println!("Actionable: {}", triage_result.actionable);
            println!("Clarity:    {:?}", triage_result.clarity);
            println!("Complexity: {}", triage_result.complexity);
            println!("Summary:    {}", triage_result.summary);
            println!("Plan:       {}", triage_result.plan);
            println!();
            continue;
        }

        // Execute
        state.set_status(&full_repo, issue.number, &issue.title, IssueStatus::Executing)?;
        state.set_branch(&full_repo, issue.number, &issue.branch_name())?;

        let tools = repo_config.all_tools(&config.settings.worker_tools);
        let exec_runner = ClaudeRunner::new(tools);

        let exec_result = pipeline::execute::execute(
            issue,
            &triage_result,
            repo_config,
            &exec_runner,
            &config.settings.models,
            &config.settings.worktree_dir,
        )?;

        // Report
        pipeline::report::report(
            issue,
            &exec_result,
            repo_config,
            &github,
            &mut state,
            &config.settings.worktree_dir,
        )
        .await?;
    }

    let summary = state.summary();
    info!("run complete: {summary}");

    Ok(())
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

            // Extract issue number from path
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
