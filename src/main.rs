mod claude;
mod config;
mod error;
mod git;
mod parent_prompt;
mod pipeline;
mod prompt;
mod state;
mod task;

use std::path::PathBuf;
use std::sync::Arc;

use clap::{Parser, Subcommand};
use tokio::sync::Semaphore;
use tokio::task::JoinSet;
use tracing::{error, info, warn};

use crate::claude::runner::ClaudeRunner;
use crate::config::Config;
use crate::error::Result;
use crate::pipeline::execute::ExecuteResult;
use crate::pipeline::integrate::WorkerOutput;
use crate::pipeline::triage::{self, ConsultationOutcome, DeepTriageResult, Task, TaskStatus};
use crate::pipeline::work;
use crate::state::tracker::{IssueStatus, SharedState, StateTracker};
use crate::task::ForgeIssue;

#[derive(Parser)]
#[command(
  name = "pfl-forge",
  about = "Multi-agent issue processor powered by Claude Code"
)]
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

    /// Resume failed/interrupted issues
    #[arg(long)]
    resume: bool,
  },
  /// Watch for new issues and process them periodically
  Watch,
  /// Show current processing status
  Status,
  /// Clean up worktrees for completed issues
  Clean,
  /// List pending clarifications
  Clarifications,
  /// Answer a clarification question
  Answer {
    /// Issue number
    number: u64,
    /// Answer text
    text: String,
  },
  /// Launch parent agent (interactive Claude Code session)
  Parent {
    /// Claude model to use
    #[arg(long)]
    model: Option<String>,
  },
}

enum TriageOutcome {
  Tasks(Vec<(PathBuf, ForgeIssue)>),
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
    Commands::Run { dry_run, resume } => cmd_run(&config, dry_run, resume).await,
    Commands::Watch => cmd_watch(&config).await,
    Commands::Status => cmd_status(&config),
    Commands::Clean => cmd_clean(&config),
    Commands::Clarifications => cmd_clarifications(&config),
    Commands::Answer { number, text } => cmd_answer(&config, number, &text),
    Commands::Parent { model } => cmd_parent(&config, model.as_deref()),
  }
}

async fn cmd_run(config: &Config, dry_run: bool, resume: bool) -> Result<()> {
  let state = StateTracker::load(&config.settings.state_file)?.into_shared();

  // Fetch local tasks
  let mut issues = {
    let s = state.lock().unwrap();
    pipeline::fetch::fetch_local_tasks(config, &s)?
  };

  // Fetch resumable tasks if --resume
  if resume {
    let resumable = {
      let s = state.lock().unwrap();
      pipeline::fetch::fetch_resumable_tasks(config, &s)?
    };
    for issue in resumable {
      if !issues
        .iter()
        .any(|i| i.number == issue.number && i.repo_name == issue.repo_name)
      {
        issues.push(issue);
      }
    }
  }

  // Fetch tasks that received clarification answers
  {
    let clarified = {
      let s = state.lock().unwrap();
      pipeline::fetch::fetch_clarified_tasks(config, &s)?
    };
    for issue in clarified {
      if !issues
        .iter()
        .any(|i| i.number == issue.number && i.repo_name == issue.repo_name)
      {
        issues.push(issue);
      }
    }
  }

  if issues.is_empty() {
    info!("no issues to process");
    return Ok(());
  }

  info!("processing {} issue(s)", issues.len());

  if dry_run {
    return cmd_run_dry(config, &issues).await;
  }

  // Phase 1: Triage (parallel per issue)
  let semaphore = Arc::new(Semaphore::new(config.settings.parallel_workers));
  let mut triage_set = JoinSet::new();

  for issue in issues {
    let sem = semaphore.clone();
    let state = state.clone();
    let config = config.clone();

    triage_set.spawn(async move {
      let _permit = sem.acquire().await.expect("semaphore closed");
      triage_issue(issue, &config, &state).await
    });
  }

  let mut task_entries: Vec<(PathBuf, ForgeIssue)> = Vec::new();

  while let Some(result) = triage_set.join_next().await {
    match result {
      Ok(Ok(TriageOutcome::Tasks(entries))) => {
        task_entries.extend(entries);
      }
      Ok(Ok(TriageOutcome::NeedsClarification {
        issue,
        message,
        deep_result,
        repo_path,
      })) => {
        if let Err(e) =
          pipeline::clarification::write_clarification(&repo_path, &issue, &deep_result, &message)
        {
          error!("failed to write clarification for {issue}: {e}");
        }
      }
      Ok(Err(e)) => error!("triage error: {e}"),
      Err(e) => error!("triage join error: {e}"),
    }
  }

  if task_entries.is_empty() {
    info!("no tasks to execute");
    let summary = state.lock().unwrap().summary();
    info!("run complete: {summary}");
    return Ok(());
  }

  info!("executing {} task(s)", task_entries.len());

  // Phase 2: Execute (parallel per task) + Phase 3: Streaming integration
  let mut exec_set = JoinSet::new();

  for (task_path, issue) in task_entries {
    let sem = semaphore.clone();
    let state = state.clone();
    let config = config.clone();

    exec_set.spawn(async move {
      let _permit = sem.acquire().await.expect("semaphore closed");
      execute_task(task_path, issue, &config, &state).await
    });
  }

  while let Some(result) = exec_set.join_next().await {
    match result {
      Ok(Ok(output)) => {
        if matches!(output.result, ExecuteResult::Success { .. }) {
          if let Err(e) = pipeline::integrate::integrate_one(&output, config, &state).await {
            error!("integration failed for {}: {e}", output.issue);
            let _ = work::set_task_status(&output.task_path, TaskStatus::Failed);
            state.lock().unwrap().set_error(
              &output.issue.repo_name,
              output.issue.number,
              &e.to_string(),
            )?;
          } else {
            let _ = work::set_task_status(&output.task_path, TaskStatus::Completed);
          }
        } else {
          let _ = work::set_task_status(&output.task_path, TaskStatus::Failed);
          if let Err(e) = pipeline::report::report(&output.issue, &output.result, &state) {
            error!("report failed for {}: {e}", output.issue);
          }
        }
      }
      Ok(Err(e)) => error!("execute error: {e}"),
      Err(e) => error!("execute join error: {e}"),
    }
  }

  let summary = state.lock().unwrap().summary();
  info!("run complete: {summary}");

  Ok(())
}

async fn triage_issue(
  issue: ForgeIssue,
  config: &Config,
  state: &SharedState,
) -> Result<TriageOutcome> {
  let repo_name = issue.repo_name.clone();
  let repo_path = Config::repo_path();

  {
    let mut s = state.lock().unwrap();
    s.set_status(
      &repo_name,
      issue.number,
      &issue.title,
      IssueStatus::Triaging,
    )?;
    s.set_started(&repo_name, issue.number)?;
  }

  let clarification_ctx = pipeline::clarification::check_clarification(&repo_path, issue.number)?;

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
          &repo_name,
          issue.number,
          &issue.title,
          IssueStatus::NeedsClarification,
        )?;
        return Ok(TriageOutcome::NeedsClarification {
          issue,
          message,
          deep_result,
          repo_path,
        });
      }
    }
  };

  // Write task YAML to .forge/work/
  let task_paths = work::write_tasks(&repo_path, &issue, &deep_result)?;

  let entries: Vec<(PathBuf, ForgeIssue)> =
    task_paths.into_iter().map(|p| (p, issue.clone())).collect();

  Ok(TriageOutcome::Tasks(entries))
}

async fn execute_task(
  task_path: PathBuf,
  issue: ForgeIssue,
  config: &Config,
  state: &SharedState,
) -> Result<WorkerOutput> {
  let repo_name = issue.repo_name.clone();
  let repo_path = Config::repo_path();

  // Read and lock task
  let content = std::fs::read_to_string(&task_path)?;
  let task: Task = serde_yaml::from_str(&content)?;
  work::set_task_status(&task_path, TaskStatus::Executing)?;

  {
    let mut s = state.lock().unwrap();
    s.set_status(
      &repo_name,
      issue.number,
      &issue.title,
      IssueStatus::Executing,
    )?;
    s.set_branch(&repo_name, issue.number, &issue.branch_name())?;
  }

  let tools = config.all_tools();
  let exec_runner = ClaudeRunner::new(tools);
  let issue_clone = issue.clone();
  let task_clone = task.clone();
  let config_clone = config.clone();
  let models = config.settings.models.clone();
  let worktree_dir = config.settings.worktree_dir.clone();
  let worker_timeout_secs = config.settings.worker_timeout_secs;

  let exec_result = tokio::task::spawn_blocking(move || {
    pipeline::execute::execute(
      &issue_clone,
      &task_clone,
      &config_clone,
      &exec_runner,
      &models,
      &worktree_dir,
      worker_timeout_secs,
    )
  })
  .await
  .map_err(|e| crate::error::ForgeError::Claude(format!("spawn_blocking: {e}")))??;

  if matches!(exec_result, ExecuteResult::Success { .. }) {
    state.lock().unwrap().set_status(
      &repo_name,
      issue.number,
      &issue.title,
      IssueStatus::Success,
    )?;
    let _ = pipeline::clarification::cleanup_clarification(&repo_path, issue.number);
  }

  Ok(WorkerOutput {
    issue,
    result: exec_result,
    task,
    task_path,
  })
}

async fn cmd_run_dry(config: &Config, issues: &[ForgeIssue]) -> Result<()> {
  let deep_runner = ClaudeRunner::new(config.settings.triage_tools.clone());
  let repo_path = Config::repo_path();

  for issue in issues {
    let deep = triage::deep_triage(issue, config, &deep_runner, &repo_path, None)?;

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

async fn cmd_watch(config: &Config) -> Result<()> {
  let interval = std::time::Duration::from_secs(config.settings.poll_interval_secs);

  loop {
    info!("polling for new tasks...");

    if let Err(e) = cmd_run(config, false, true).await {
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
    let err = issue_state
      .error
      .as_ref()
      .map(|e| format!(" ({e})"))
      .unwrap_or_default();
    println!("  {key}: {status}{err}");
  }

  Ok(())
}

fn cmd_clean(config: &Config) -> Result<()> {
  let state = StateTracker::load(&config.settings.state_file)?;
  let repo_name = Config::repo_name();
  let repo_path = Config::repo_path();

  let worktrees = git::worktree::list(&repo_path)?;
  let worktree_prefix = format!(
    "{}/forge/",
    repo_path.join(&config.settings.worktree_dir).display()
  );

  for wt in &worktrees {
    if !wt.starts_with(&worktree_prefix) {
      continue;
    }

    if let Some(num_str) = wt.rsplit("issue-").next() {
      if let Ok(num) = num_str.parse::<u64>() {
        if state.is_processed(&repo_name, num) {
          info!("cleaning worktree: {wt}");
          git::worktree::remove(&repo_path, std::path::Path::new(wt))?;
        }
      }
    }
  }

  Ok(())
}

fn cmd_clarifications(_config: &Config) -> Result<()> {
  let repo_path = Config::repo_path();
  let pending = pipeline::clarification::list_pending_clarifications(&repo_path)?;

  if pending.is_empty() {
    println!("No pending clarifications.");
    return Ok(());
  }

  for c in &pending {
    println!("=== #{} ===", c.issue_number);
    println!("{}", c.content);
    println!();
  }

  Ok(())
}

fn cmd_answer(config: &Config, number: u64, text: &str) -> Result<()> {
  let repo_name = Config::repo_name();
  let repo_path = Config::repo_path();

  pipeline::clarification::write_answer(&repo_path, number, text)?;

  let mut state = StateTracker::load(&config.settings.state_file)?;
  state.reset_to_pending(&repo_name, number)?;

  println!("Answered clarification for #{number} and reset to pending.");
  Ok(())
}

fn cmd_parent(config: &Config, model: Option<&str>) -> Result<()> {
  let state = StateTracker::load(&config.settings.state_file)?;

  let system_prompt = crate::prompt::PARENT;
  let initial_message = parent_prompt::build_initial_message(config, &state)?;

  let mut cmd = std::process::Command::new("claude");
  cmd
    .arg("--append-system-prompt")
    .arg(&system_prompt)
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
