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
use crate::pipeline::triage::{self, ConsultationOutcome, DeepTriageResult, Task, WorkStatus};
use crate::pipeline::work;
use crate::state::tracker::{SharedState, StateTracker, TaskStatus};
use crate::task::ForgeTask;

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
    /// Issue ID
    id: String,
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
  Tasks(Vec<(PathBuf, ForgeTask)>),
  NeedsClarification {
    issue: ForgeTask,
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
    Commands::Run { dry_run } => cmd_run(&config, dry_run).await,
    Commands::Watch => cmd_watch(&config).await,
    Commands::Status => cmd_status(&config),
    Commands::Clean => cmd_clean(&config),
    Commands::Clarifications => cmd_clarifications(&config),
    Commands::Answer { id, text } => cmd_answer(&config, &id, &text),
    Commands::Parent { model } => cmd_parent(&config, model.as_deref()),
  }
}

async fn cmd_run(config: &Config, dry_run: bool) -> Result<()> {
  let state = StateTracker::load(&config.settings.state_file)?.into_shared();

  let issues = {
    let s = state.lock().unwrap();
    pipeline::fetch::fetch_tasks(config, &s)?
  };

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

  let mut task_entries: Vec<(PathBuf, ForgeTask)> = Vec::new();

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
            let _ = work::set_task_status(&output.task_path, WorkStatus::Failed);
            state
              .lock()
              .unwrap()
              .set_error(&output.issue.id, &e.to_string())?;
          } else {
            let _ = work::set_task_status(&output.task_path, WorkStatus::Completed);
          }
        } else {
          let _ = work::set_task_status(&output.task_path, WorkStatus::Failed);
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
  issue: ForgeTask,
  config: &Config,
  state: &SharedState,
) -> Result<TriageOutcome> {
  let repo_path = Config::repo_path();

  {
    let mut s = state.lock().unwrap();
    s.set_status(&issue.id, &issue.title, TaskStatus::Triaging)?;
    s.set_started(&issue.id)?;
  }

  let clarification_ctx = pipeline::clarification::check_clarification(&repo_path, &issue.id)?;

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
          &issue.id,
          &issue.title,
          TaskStatus::NeedsClarification,
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

  let entries: Vec<(PathBuf, ForgeTask)> =
    task_paths.into_iter().map(|p| (p, issue.clone())).collect();

  Ok(TriageOutcome::Tasks(entries))
}

async fn execute_task(
  task_path: PathBuf,
  issue: ForgeTask,
  config: &Config,
  state: &SharedState,
) -> Result<WorkerOutput> {
  let repo_path = Config::repo_path();

  // Read and lock task
  let content = std::fs::read_to_string(&task_path)?;
  let task: Task = serde_yaml::from_str(&content)?;
  work::set_task_status(&task_path, WorkStatus::Executing)?;

  {
    let mut s = state.lock().unwrap();
    s.set_status(&issue.id, &issue.title, TaskStatus::Executing)?;
    s.set_branch(&issue.id, &issue.branch_name())?;
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
    state
      .lock()
      .unwrap()
      .set_status(&issue.id, &issue.title, TaskStatus::Success)?;
    let _ = pipeline::clarification::cleanup_clarification(&repo_path, &issue.id);
  }

  Ok(WorkerOutput {
    issue,
    result: exec_result,
    task,
    task_path,
  })
}

async fn cmd_run_dry(config: &Config, issues: &[ForgeTask]) -> Result<()> {
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

    if let Err(e) = cmd_run(config, false).await {
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

  for (key, issue_state) in state.all_tasks() {
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

    if let Some(id) = wt.strip_prefix(&worktree_prefix) {
      if state.is_processed(id) {
        info!("cleaning worktree: {wt}");
        git::worktree::remove(&repo_path, std::path::Path::new(wt))?;
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
    println!("=== {} ===", c.task_id);
    println!("{}", c.content);
    println!();
  }

  Ok(())
}

fn cmd_answer(config: &Config, id: &str, text: &str) -> Result<()> {
  let repo_path = Config::repo_path();

  pipeline::clarification::write_answer(&repo_path, id, text)?;

  let mut state = StateTracker::load(&config.settings.state_file)?;
  state.reset_to_pending(id)?;

  println!("Answered clarification for {id} and reset to pending.");
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
