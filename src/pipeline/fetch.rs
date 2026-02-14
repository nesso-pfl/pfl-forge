use tracing::info;

use crate::config::{Config, RepoConfig};
use crate::error::Result;
use crate::pipeline::clarification;
use crate::state::tracker::StateTracker;
use crate::task::ForgeIssue;

#[derive(serde::Deserialize)]
struct LocalTask {
	title: String,
	body: String,
	#[serde(default)]
	labels: Vec<String>,
}

pub fn fetch_local_tasks(config: &Config, state: &StateTracker) -> Result<Vec<ForgeIssue>> {
	let mut all = Vec::new();

	for repo in &config.repos {
		let tasks = load_local_tasks(repo, state)?;
		all.extend(tasks);
	}

	info!("local tasks: {}", all.len());
	Ok(all)
}

fn load_local_tasks(repo: &RepoConfig, state: &StateTracker) -> Result<Vec<ForgeIssue>> {
	let tasks_dir = repo.path.join(".forge/tasks");
	if !tasks_dir.exists() {
		return Ok(Vec::new());
	}

	let mut issues = Vec::new();
	let mut entries: Vec<_> = std::fs::read_dir(&tasks_dir)?
		.filter_map(|e| e.ok())
		.collect();
	entries.sort_by_key(|e| e.file_name());

	for entry in entries {
		let path = entry.path();
		if path.extension().and_then(|e| e.to_str()) != Some("yaml") {
			continue;
		}

		let stem = path
			.file_stem()
			.and_then(|s| s.to_str())
			.unwrap_or_default();
		let number: u64 = match stem.parse() {
			Ok(n) => n,
			Err(_) => continue,
		};

		if state.is_terminal(&repo.name, number) {
			info!("skipping terminal local task: {}#{number}", repo.name);
			continue;
		}

		let content = std::fs::read_to_string(&path)?;
		let task: LocalTask = serde_yaml::from_str(&content)?;

		issues.push(ForgeIssue {
			number,
			title: task.title,
			body: task.body,
			labels: task.labels,
			repo_name: repo.name.clone(),
			created_at: chrono::Utc::now(),
		});
	}

	Ok(issues)
}

pub fn fetch_resumable_tasks(config: &Config, state: &StateTracker) -> Result<Vec<ForgeIssue>> {
	let resumable = state.resumable_issues();
	let mut issues = Vec::new();

	for (repo_name, number) in resumable {
		let Some(repo_config) = config.find_repo(&repo_name) else {
			info!("skipping resumable task {repo_name}#{number}: repo not in config");
			continue;
		};

		// Re-read the local task file
		let task_path = repo_config
			.path
			.join(".forge/tasks")
			.join(format!("{number}.yaml"));
		if !task_path.exists() {
			info!("skipping resumable task {repo_name}#{number}: task file not found");
			continue;
		}

		let content = std::fs::read_to_string(&task_path)?;
		let task: LocalTask = serde_yaml::from_str(&content)?;

		info!("resuming: {repo_name}#{number}");
		issues.push(ForgeIssue {
			number,
			title: task.title,
			body: task.body,
			labels: task.labels,
			repo_name: repo_name.clone(),
			created_at: chrono::Utc::now(),
		});
	}

	info!("resumable tasks: {}", issues.len());
	Ok(issues)
}

pub fn fetch_clarified_tasks(config: &Config, state: &StateTracker) -> Result<Vec<ForgeIssue>> {
	let needs_clarification = state.needs_clarification_issues();
	let mut issues = Vec::new();

	for (repo_name, number) in needs_clarification {
		let Some(repo_config) = config.find_repo(&repo_name) else {
			continue;
		};

		if clarification::check_clarification(&repo_config.path, number)?.is_none() {
			continue;
		}

		let task_path = repo_config
			.path
			.join(".forge/tasks")
			.join(format!("{number}.yaml"));
		if !task_path.exists() {
			continue;
		}

		let content = std::fs::read_to_string(&task_path)?;
		let task: LocalTask = serde_yaml::from_str(&content)?;

		info!("clarification answered, re-processing: {repo_name}#{number}");
		issues.push(ForgeIssue {
			number,
			title: task.title,
			body: task.body,
			labels: task.labels,
			repo_name: repo_name.clone(),
			created_at: chrono::Utc::now(),
		});
	}

	info!("clarified tasks: {}", issues.len());
	Ok(issues)
}
