mod agents;
mod claude;
mod config;
mod error;
mod git;
mod prompt;
mod state;
mod task;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use clap::{Parser, Subcommand};
use tokio::sync::Semaphore;
use tokio::task::JoinSet;
use tracing::{error, info, warn};

use crate::agents::architect::ArchitectOutcome;
use crate::agents::review::ReviewResult;
use crate::agents::{analyze, implement, review};
use crate::claude::runner::ClaudeRunner;
use crate::config::Config;
use crate::error::Result;
use crate::state::tracker::{SharedState, StateTracker, TaskStatus};
use crate::task::work::{self, WorkStatus};
use crate::task::Issue;

#[derive(Parser)]
#[command(
  name = "pfl-forge",
  about = "Multi-agent task processor powered by Claude Code"
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
  /// Process tasks: fetch, triage, execute, report
  Run {
    /// Only triage, don't execute
    #[arg(long)]
    dry_run: bool,
  },
  /// Watch for new tasks and process them periodically
  Watch,
  /// Show current processing status
  Status,
  /// Clean up worktrees for completed tasks
  Clean,
  /// List pending clarifications
  Clarifications,
  /// Answer a clarification question
  Answer {
    /// Task ID
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
  /// Create a new task in .forge/tasks/
  Create {
    /// Task title
    title: String,
    /// Task body (description)
    body: String,
    /// Labels (comma-separated)
    #[arg(long, value_delimiter = ',')]
    labels: Vec<String>,
  },
}

#[derive(Debug)]
enum ExecuteResult {
  Success,
  Unclear(String),
  Error(String),
}

struct PrepareResult {
  worktree_path: PathBuf,
  selected_model: String,
}

fn prepare(
  issue: &Issue,
  task: &work::Task,
  config: &Config,
  worktree_dir: &str,
) -> Result<PrepareResult> {
  let branch = issue.branch_name();
  let repo_path = Config::repo_path();

  let worktree_path =
    git::worktree::create(&repo_path, worktree_dir, &branch, &config.base_branch)?;

  info!("prepared worktree: {}", worktree_path.display());

  work::write_task_yaml(&worktree_path, task)?;
  git::worktree::ensure_gitignore_forge(&worktree_path)?;

  let complexity = task.complexity();
  let selected_model = complexity.select_model(&config.models).to_string();

  Ok(PrepareResult {
    worktree_path,
    selected_model,
  })
}

fn write_review_yaml(worktree_path: &Path, result: &ReviewResult) -> Result<()> {
  let forge_dir = worktree_path.join(".forge");
  std::fs::create_dir_all(&forge_dir)?;
  let content = serde_yaml::to_string(result)?;
  std::fs::write(forge_dir.join("review.yaml"), content)?;
  Ok(())
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
    Commands::Parent { model } => agents::orchestrate::launch(&config, model.as_deref()),
    Commands::Create {
      title,
      body,
      labels,
    } => cmd_create(&title, &body, &labels),
  }
}

async fn cmd_run(config: &Config, dry_run: bool) -> Result<()> {
  let state = StateTracker::load(&config.state_file)?.into_shared();

  let issues = {
    let s = state.lock().unwrap();
    task::issue::fetch_tasks(config, &s)?
  };

  if issues.is_empty() {
    info!("no tasks to process");
    return Ok(());
  }

  info!("processing {} task(s)", issues.len());

  if dry_run {
    return cmd_run_dry(config, &issues).await;
  }

  let semaphore = Arc::new(Semaphore::new(config.parallel_workers));
  let mut join_set = JoinSet::new();

  for issue in issues {
    let sem = semaphore.clone();
    let state = state.clone();
    let config = config.clone();

    join_set.spawn(async move { process_task(issue, &config, &state, &sem).await });
  }

  while let Some(result) = join_set.join_next().await {
    match result {
      Ok(Err(e)) => error!("task error: {e}"),
      Err(e) => error!("task join error: {e}"),
      Ok(Ok(())) => {}
    }
  }

  let summary = state.lock().unwrap().summary();
  info!("run complete: {summary}");

  Ok(())
}

async fn process_task(
  issue: Issue,
  config: &Config,
  state: &SharedState,
  semaphore: &Semaphore,
) -> Result<()> {
  let repo_path = Config::repo_path();

  // --- Analyze phase ---
  {
    let mut s = state.lock().unwrap();
    s.set_status(&issue.id, &issue.title, TaskStatus::Triaging)?;
    s.set_started(&issue.id)?;
  }

  let analysis = {
    let _permit = semaphore.acquire().await.expect("semaphore closed");

    let clarification_ctx = task::clarification::check_clarification(&repo_path, &issue.id)?;

    let analyze_runner = ClaudeRunner::new(config.triage_tools.clone());
    let issue_clone = issue.clone();
    let config_clone = config.clone();
    let repo_path_clone = repo_path.clone();
    let analysis = tokio::task::spawn_blocking(move || {
      analyze::analyze(
        &issue_clone,
        &config_clone,
        &analyze_runner,
        &repo_path_clone,
        clarification_ctx.as_ref(),
      )
    })
    .await
    .map_err(|e| crate::error::ForgeError::Claude(format!("spawn_blocking: {e}")))??;

    if analysis.is_sufficient() {
      analysis
    } else {
      info!("analysis insufficient for {issue}, consulting architect...");
      let architect_runner = ClaudeRunner::new(config.triage_tools.clone());
      let issue_clone = issue.clone();
      let analysis_clone = analysis.clone();
      let config_clone = config.clone();
      let repo_path_clone = repo_path.clone();
      let outcome = tokio::task::spawn_blocking(move || {
        agents::architect::resolve(
          &issue_clone,
          &analysis_clone,
          &config_clone,
          &architect_runner,
          &repo_path_clone,
        )
      })
      .await
      .map_err(|e| crate::error::ForgeError::Claude(format!("spawn_blocking: {e}")))??;

      match outcome {
        ArchitectOutcome::Resolved(result) => result,
        ArchitectOutcome::NeedsClarification(message) => {
          info!("architect needs clarification for {issue}");
          state.lock().unwrap().set_status(
            &issue.id,
            &issue.title,
            TaskStatus::NeedsClarification,
          )?;
          if let Err(e) =
            task::clarification::write_clarification(&repo_path, &issue, &analysis, &message)
          {
            error!("failed to write clarification for {issue}: {e}");
          }
          return Ok(());
        }
      }
    }
    // permit released here
  };

  // Write task YAML to .forge/work/
  let work_paths = work::write_tasks(&repo_path, &issue, &analysis)?;
  let work_path = &work_paths[0];

  let content = std::fs::read_to_string(work_path)?;
  let task: work::Task = serde_yaml::from_str(&content)?;

  // --- Prepare worktree ---
  let prepare_result = {
    let issue_clone = issue.clone();
    let task_clone = task.clone();
    let config_clone = config.clone();
    let worktree_dir = config.worktree_dir.clone();
    tokio::task::spawn_blocking(move || {
      prepare(&issue_clone, &task_clone, &config_clone, &worktree_dir)
    })
    .await
    .map_err(|e| crate::error::ForgeError::Claude(format!("spawn_blocking: {e}")))??
  };

  // --- Implement + Review loop ---
  let max_attempts = config.max_review_retries + 1;
  let mut review_feedback: Option<ReviewResult> = None;

  for attempt in 0..max_attempts {
    if attempt > 0 {
      info!(
        "re-implementing {issue} (attempt {}/{})",
        attempt + 1,
        max_attempts
      );
    }

    // --- Implement ---
    {
      work::set_task_status(work_path, WorkStatus::Executing)?;
      {
        let mut s = state.lock().unwrap();
        s.set_status(&issue.id, &issue.title, TaskStatus::Executing)?;
        s.set_branch(&issue.id, &issue.branch_name())?;
      }

      let _permit = semaphore.acquire().await.expect("semaphore closed");

      let impl_runner = ClaudeRunner::new(config.worker_tools.clone());
      let issue_clone = issue.clone();
      let worktree_path = prepare_result.worktree_path.clone();
      let selected_model = prepare_result.selected_model.clone();
      let timeout_secs = config.worker_timeout_secs;
      let feedback_clone = review_feedback.clone();

      let impl_result = tokio::task::spawn_blocking(move || {
        implement::run(
          &issue_clone,
          &impl_runner,
          &selected_model,
          &worktree_path,
          Some(std::time::Duration::from_secs(timeout_secs)),
          feedback_clone.as_ref(),
        )
      })
      .await
      .map_err(|e| crate::error::ForgeError::Claude(format!("spawn_blocking: {e}")))?;

      // permit released here

      let exec_result = match impl_result {
        Ok(_output) => {
          let commits =
            git::branch::commit_count(&prepare_result.worktree_path, &config.base_branch, "HEAD")
              .unwrap_or(0);

          if commits == 0 {
            info!("no commits produced");
            ExecuteResult::Unclear("Worker completed but produced no commits".into())
          } else {
            info!("{commits} commit(s) produced");
            ExecuteResult::Success
          }
        }
        Err(e) => ExecuteResult::Error(e.to_string()),
      };

      match &exec_result {
        ExecuteResult::Success => {}
        ExecuteResult::Unclear(reason) => {
          let _ = work::set_task_status(work_path, WorkStatus::Failed);
          info!("unclear result: {issue}: {reason}");
          state.lock().unwrap().set_error(&issue.id, reason)?;
          return Ok(());
        }
        ExecuteResult::Error(err) => {
          let _ = work::set_task_status(work_path, WorkStatus::Failed);
          info!("error: {issue}: {err}");
          state.lock().unwrap().set_error(&issue.id, err)?;
          return Ok(());
        }
      }
    }

    // --- Rebase ---
    {
      let wt = prepare_result.worktree_path.clone();
      let bb = config.base_branch.clone();
      let label = issue.to_string();
      let rebase_ok =
        tokio::task::spawn_blocking(move || git::branch::try_rebase(&wt, &bb, &label))
          .await
          .map_err(|e| crate::error::ForgeError::Git(format!("spawn_blocking: {e}")))??;

      if !rebase_ok {
        let _ = work::set_task_status(work_path, WorkStatus::Failed);
        state
          .lock()
          .unwrap()
          .set_error(&issue.id, "rebase conflict")?;
        return Ok(());
      }
    }

    // --- Review ---
    {
      state
        .lock()
        .unwrap()
        .set_status(&issue.id, &issue.title, TaskStatus::Reviewing)?;

      let _permit = semaphore.acquire().await.expect("semaphore closed");

      let review_runner = ClaudeRunner::new(config.triage_tools.clone());
      let issue_clone = issue.clone();
      let task_clone = task.clone();
      let config_clone = config.clone();
      let wt = prepare_result.worktree_path.clone();
      let bb = config.base_branch.clone();
      let review_result = tokio::task::spawn_blocking(move || {
        review::review(
          &issue_clone,
          &task_clone,
          &config_clone,
          &review_runner,
          &wt,
          &bb,
        )
      })
      .await
      .map_err(|e| crate::error::ForgeError::Claude(format!("spawn_blocking: {e}")))?;

      // permit released here

      if let Ok(ref result) = review_result {
        if let Err(e) = write_review_yaml(&prepare_result.worktree_path, result) {
          warn!("failed to write review.yaml: {e}");
        }
      }

      match review_result {
        Ok(result) if result.approved => {
          info!(
            "task {issue} completed, branch {} available locally",
            issue.branch_name()
          );
          let _ = work::set_task_status(work_path, WorkStatus::Completed);
          state
            .lock()
            .unwrap()
            .set_status(&issue.id, &issue.title, TaskStatus::Success)?;
          let _ = task::clarification::cleanup_clarification(&repo_path, &issue.id);
          return Ok(());
        }
        Ok(result) => {
          if attempt + 1 < max_attempts {
            info!(
              "review rejected for {issue}, retrying ({} remaining)",
              max_attempts - attempt - 1
            );
            review_feedback = Some(result);
            continue;
          }
          let _ = work::set_task_status(work_path, WorkStatus::Failed);
          state
            .lock()
            .unwrap()
            .set_error(&issue.id, "review rejected after retries")?;
          return Ok(());
        }
        Err(e) => {
          warn!("review failed for {issue}: {e}, proceeding as approved");
          let _ = work::set_task_status(work_path, WorkStatus::Completed);
          state
            .lock()
            .unwrap()
            .set_status(&issue.id, &issue.title, TaskStatus::Success)?;
          let _ = task::clarification::cleanup_clarification(&repo_path, &issue.id);
          return Ok(());
        }
      }
    }
  }

  Ok(())
}

async fn cmd_run_dry(config: &Config, issues: &[Issue]) -> Result<()> {
  let analyze_runner = ClaudeRunner::new(config.triage_tools.clone());
  let repo_path = Config::repo_path();

  for issue in issues {
    let result = analyze::analyze(issue, config, &analyze_runner, &repo_path, None)?;

    println!("--- {} ---", issue);
    println!("Complexity: {}", result.complexity);
    println!("Plan:       {}", result.plan);
    println!("Files:      {}", result.relevant_files.join(", "));
    println!("Steps:      {}", result.implementation_steps.len());
    println!("Sufficient: {}", result.is_sufficient());
    println!();
  }

  Ok(())
}

async fn cmd_watch(config: &Config) -> Result<()> {
  let interval = std::time::Duration::from_secs(config.poll_interval_secs);

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
  let state = StateTracker::load(&config.state_file)?;
  let summary = state.summary();

  println!("pfl-forge status");
  println!("================");
  println!("{summary}");
  println!();

  for (key, task_state) in state.all_tasks() {
    let status = format!("{:?}", task_state.status);
    let err = task_state
      .error
      .as_ref()
      .map(|e| format!(" ({e})"))
      .unwrap_or_default();
    println!("  {key}: {status}{err}");
  }

  Ok(())
}

fn cmd_clean(config: &Config) -> Result<()> {
  let state = StateTracker::load(&config.state_file)?;
  let repo_path = Config::repo_path();

  let worktrees = git::worktree::list(&repo_path)?;
  let worktree_prefix = format!("{}/forge/", repo_path.join(&config.worktree_dir).display());

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
  let pending = task::clarification::list_pending_clarifications(&repo_path)?;

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

  task::clarification::write_answer(&repo_path, id, text)?;

  let mut state = StateTracker::load(&config.state_file)?;
  state.reset_to_pending(id)?;

  println!("Answered clarification for {id} and reset to pending.");
  Ok(())
}

fn cmd_create(title: &str, body: &str, labels: &[String]) -> Result<()> {
  let repo_path = Config::repo_path();
  let tasks_dir = repo_path.join(".forge/tasks");
  std::fs::create_dir_all(&tasks_dir)?;

  let id = uuid::Uuid::new_v4().to_string();
  let issue = Issue {
    id: id.clone(),
    title: title.to_string(),
    body: body.to_string(),
    labels: labels.to_vec(),
  };

  let path = tasks_dir.join(format!("{id}.yaml"));
  let yaml = serde_yaml::to_string(&issue)?;
  std::fs::write(&path, &yaml)?;

  println!("Created task {id}");
  println!("{}", path.display());

  Ok(())
}
