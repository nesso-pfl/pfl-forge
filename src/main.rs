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

enum TriageOutcome {
	Tasks(Vec<(PathBuf, ForgeIssue, String)>),
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

	let mut task_entries: Vec<(PathBuf, ForgeIssue, String)> = Vec::new();

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

	for (task_path, issue, repo_config_name) in task_entries {
		let sem = semaphore.clone();
		let state = state.clone();
		let config = config.clone();

		exec_set.spawn(async move {
			let _permit = sem.acquire().await.expect("semaphore closed");
			execute_task(task_path, issue, repo_config_name, &config, &state).await
		});
	}

	while let Some(result) = exec_set.join_next().await {
		match result {
			Ok(Ok(output)) => {
				let repo_config = config
					.find_repo(&output.repo_config_name)
					.expect("repo config should exist");

				if matches!(output.result, ExecuteResult::Success { .. }) {
					if let Err(e) =
						pipeline::integrate::integrate_one(&output, repo_config, config, &state).await
					{
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
	let repo_config = config
		.find_repo(&issue.repo_name)
		.expect("issue repo should be in config");
	let repo_name = issue.repo_name.clone();
	let repo_config_name = issue.repo_name.clone();

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

	let repo_path = repo_config.path.clone();
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

	let entries: Vec<(PathBuf, ForgeIssue, String)> = task_paths
		.into_iter()
		.map(|p| (p, issue.clone(), repo_config_name.clone()))
		.collect();

	Ok(TriageOutcome::Tasks(entries))
}

async fn execute_task(
	task_path: PathBuf,
	issue: ForgeIssue,
	repo_config_name: String,
	config: &Config,
	state: &SharedState,
) -> Result<WorkerOutput> {
	let repo_config = config
		.find_repo(&repo_config_name)
		.expect("repo config should exist");
	let repo_name = issue.repo_name.clone();

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

	let tools = repo_config.all_tools(&config.settings.worker_tools);
	let exec_runner = ClaudeRunner::new(tools);
	let issue_clone = issue.clone();
	let task_clone = task.clone();
	let repo_config_clone = repo_config.clone();
	let models = config.settings.models.clone();
	let worktree_dir = config.settings.worktree_dir.clone();
	let worker_timeout_secs = config.settings.worker_timeout_secs;

	let exec_result = tokio::task::spawn_blocking(move || {
		pipeline::execute::execute(
			&issue_clone,
			&task_clone,
			&repo_config_clone,
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
		let _ = pipeline::clarification::cleanup_clarification(&repo_config.path, issue.number);
	}

	Ok(WorkerOutput {
		issue,
		result: exec_result,
		repo_config_name,
		task,
		task_path,
	})
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
		info!("polling for new tasks...");

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
		let err = issue_state
			.error
			.as_ref()
			.map(|e| format!(" ({e})"))
			.unwrap_or_default();
		println!("  {key}: {status}{err}");
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
					if state.is_processed(&repo.name, num) {
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
		let (name, _) =
			pipeline::clarification::find_repo_for_issue(&repos, number).ok_or_else(|| {
				crate::error::ForgeError::Config(format!("no clarification found for issue #{number}"))
			})?;
		config.find_repo(name).unwrap()
	};

	pipeline::clarification::write_answer(&repo.path, number, text)?;

	let mut state = StateTracker::load(&config.settings.state_file)?;
	state.reset_to_pending(&repo.name, number)?;

	println!("Answered clarification for #{number} and reset to pending.");
	Ok(())
}

fn cmd_parent(config: &Config, repo_filter: Option<&str>, model: Option<&str>) -> Result<()> {
	let state = StateTracker::load(&config.settings.state_file)?;

	let system_prompt = parent_prompt::build_system_prompt(config);
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

	if let Some(filter) = repo_filter {
		let repo = config
			.find_repo(filter)
			.ok_or_else(|| crate::error::ForgeError::Config(format!("repo not found: {filter}")))?;
		cmd.current_dir(&repo.path);
	}

	cmd.arg(&initial_message);

	use std::os::unix::process::CommandExt;
	let err = cmd.exec();
	Err(crate::error::ForgeError::Claude(format!(
		"exec failed: {err}"
	)))
}
