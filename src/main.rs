mod agents;
mod claude;
mod config;
mod error;
mod git;
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

use crate::agents::analyze;
use crate::agents::architect::ArchitectOutcome;
use crate::agents::review::ReviewResult;
use crate::claude::runner::ClaudeRunner;
use crate::config::Config;
use crate::error::Result;
use crate::pipeline::execute::ExecuteResult;
use crate::pipeline::integrate::IntegrateResult;
use crate::pipeline::work::{self, WorkStatus};
use crate::state::tracker::{SharedState, StateTracker, TaskStatus};
use crate::task::ForgeTask;

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

  let tasks = {
    let s = state.lock().unwrap();
    pipeline::fetch::fetch_tasks(config, &s)?
  };

  if tasks.is_empty() {
    info!("no tasks to process");
    return Ok(());
  }

  info!("processing {} task(s)", tasks.len());

  if dry_run {
    return cmd_run_dry(config, &tasks).await;
  }

  let semaphore = Arc::new(Semaphore::new(config.parallel_workers));
  let mut task_set = JoinSet::new();

  for forge_task in tasks {
    let sem = semaphore.clone();
    let state = state.clone();
    let config = config.clone();

    task_set.spawn(async move { process_task(forge_task, &config, &state, &sem).await });
  }

  while let Some(result) = task_set.join_next().await {
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
  forge_task: ForgeTask,
  config: &Config,
  state: &SharedState,
  semaphore: &Semaphore,
) -> Result<()> {
  let repo_path = Config::repo_path();

  // --- Analyze phase ---
  {
    let mut s = state.lock().unwrap();
    s.set_status(&forge_task.id, &forge_task.title, TaskStatus::Triaging)?;
    s.set_started(&forge_task.id)?;
  }

  let analysis = {
    let _permit = semaphore.acquire().await.expect("semaphore closed");

    let clarification_ctx =
      pipeline::clarification::check_clarification(&repo_path, &forge_task.id)?;

    let analyze_runner = ClaudeRunner::new(config.triage_tools.clone());
    let task_clone = forge_task.clone();
    let config_clone = config.clone();
    let repo_path_clone = repo_path.clone();
    let analysis = tokio::task::spawn_blocking(move || {
      analyze::analyze(
        &task_clone,
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
      info!("analysis insufficient for {forge_task}, consulting architect...");
      let architect_runner = ClaudeRunner::new(config.triage_tools.clone());
      let task_clone = forge_task.clone();
      let analysis_clone = analysis.clone();
      let config_clone = config.clone();
      let repo_path_clone = repo_path.clone();
      let outcome = tokio::task::spawn_blocking(move || {
        agents::architect::resolve(
          &task_clone,
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
          info!("architect needs clarification for {forge_task}");
          state.lock().unwrap().set_status(
            &forge_task.id,
            &forge_task.title,
            TaskStatus::NeedsClarification,
          )?;
          if let Err(e) = pipeline::clarification::write_clarification(
            &repo_path,
            &forge_task,
            &analysis,
            &message,
          ) {
            error!("failed to write clarification for {forge_task}: {e}");
          }
          return Ok(());
        }
      }
    }
    // permit released here
  };

  // Write task YAML to .forge/work/
  let task_paths = work::write_tasks(&repo_path, &forge_task, &analysis)?;
  let task_path = &task_paths[0];

  let content = std::fs::read_to_string(task_path)?;
  let task: work::Task = serde_yaml::from_str(&content)?;

  // --- Execute + Integrate loop with review retries ---
  let max_attempts = config.max_review_retries + 1;
  let mut review_feedback: Option<ReviewResult> = None;

  for attempt in 0..max_attempts {
    if attempt > 0 {
      info!(
        "re-implementing {forge_task} (attempt {}/{})",
        attempt + 1,
        max_attempts
      );
    }

    // Execute phase
    {
      work::set_task_status(task_path, WorkStatus::Executing)?;
      {
        let mut s = state.lock().unwrap();
        s.set_status(&forge_task.id, &forge_task.title, TaskStatus::Executing)?;
        s.set_branch(&forge_task.id, &forge_task.branch_name())?;
      }

      let _permit = semaphore.acquire().await.expect("semaphore closed");

      let tools = config.worker_tools.clone();
      let exec_runner = ClaudeRunner::new(tools);
      let forge_task_clone = forge_task.clone();
      let task_clone = task.clone();
      let config_clone = config.clone();
      let models = config.models.clone();
      let worktree_dir = config.worktree_dir.clone();
      let worker_timeout_secs = config.worker_timeout_secs;
      let feedback_clone = review_feedback.clone();

      let exec_result = tokio::task::spawn_blocking(move || {
        pipeline::execute::execute(
          &forge_task_clone,
          &task_clone,
          &config_clone,
          &exec_runner,
          &models,
          &worktree_dir,
          worker_timeout_secs,
          feedback_clone.as_ref(),
        )
      })
      .await
      .map_err(|e| crate::error::ForgeError::Claude(format!("spawn_blocking: {e}")))??;

      // permit released here

      match exec_result {
        ExecuteResult::Success { .. } => {}
        _ => {
          let _ = work::set_task_status(task_path, WorkStatus::Failed);
          if let Err(e) = pipeline::report::report(&forge_task, &exec_result, state) {
            error!("report failed for {forge_task}: {e}");
          }
          return Ok(());
        }
      }
    }

    // Integrate phase (rebase + review)
    {
      state
        .lock()
        .unwrap()
        .set_status(&forge_task.id, &forge_task.title, TaskStatus::Reviewing)?;

      let _permit = semaphore.acquire().await.expect("semaphore closed");

      let integrate_result = pipeline::integrate::integrate_one(&forge_task, &task, config).await?;

      // permit released here

      match integrate_result {
        IntegrateResult::Approved => {
          let _ = work::set_task_status(task_path, WorkStatus::Completed);
          state.lock().unwrap().set_status(
            &forge_task.id,
            &forge_task.title,
            TaskStatus::Success,
          )?;
          let _ = pipeline::clarification::cleanup_clarification(&repo_path, &forge_task.id);
          return Ok(());
        }
        IntegrateResult::RebaseConflict => {
          let _ = work::set_task_status(task_path, WorkStatus::Failed);
          state
            .lock()
            .unwrap()
            .set_error(&forge_task.id, "rebase conflict")?;
          return Ok(());
        }
        IntegrateResult::ReviewRejected(result) => {
          if attempt + 1 < max_attempts {
            info!(
              "review rejected for {forge_task}, retrying ({} remaining)",
              max_attempts - attempt - 1
            );
            review_feedback = Some(result);
            continue;
          }
          let _ = work::set_task_status(task_path, WorkStatus::Failed);
          state
            .lock()
            .unwrap()
            .set_error(&forge_task.id, "review rejected after retries")?;
          return Ok(());
        }
        IntegrateResult::ReviewFailed(e) => {
          warn!("review failed for {forge_task}: {e}, proceeding as approved");
          let _ = work::set_task_status(task_path, WorkStatus::Completed);
          state.lock().unwrap().set_status(
            &forge_task.id,
            &forge_task.title,
            TaskStatus::Success,
          )?;
          let _ = pipeline::clarification::cleanup_clarification(&repo_path, &forge_task.id);
          return Ok(());
        }
      }
    }
  }

  Ok(())
}

async fn cmd_run_dry(config: &Config, tasks: &[ForgeTask]) -> Result<()> {
  let analyze_runner = ClaudeRunner::new(config.triage_tools.clone());
  let repo_path = Config::repo_path();

  for forge_task in tasks {
    let result = analyze::analyze(forge_task, config, &analyze_runner, &repo_path, None)?;

    println!("--- {} ---", forge_task);
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
  let task = ForgeTask {
    id: id.clone(),
    title: title.to_string(),
    body: body.to_string(),
    labels: labels.to_vec(),
    created_at: chrono::Utc::now(),
  };

  let path = tasks_dir.join(format!("{id}.yaml"));
  let yaml = serde_yaml::to_string(&task)?;
  std::fs::write(&path, &yaml)?;

  println!("Created task {id}");
  println!("{}", path.display());

  Ok(())
}
