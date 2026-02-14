use tracing::info;

use crate::config::{Config, RepoConfig};
use crate::error::Result;
use crate::github::client::GitHubClient;
use crate::github::issue::{ForgeIssue, TaskSource};
use crate::pipeline::clarification;
use crate::state::tracker::StateTracker;

pub async fn fetch_issues(
  config: &Config,
  github: &GitHubClient,
  state: &StateTracker,
) -> Result<Vec<ForgeIssue>> {
  let mut all_issues = Vec::new();

  for repo in &config.repos {
    let (owner, repo_name) = repo.owner_repo();

    let issues = github
      .fetch_issues(owner, repo_name, &repo.issue_label)
      .await?;

    for issue in issues {
      let full_repo = issue.full_repo();
      if state.is_terminal(&full_repo, issue.number) {
        info!("skipping terminal state: {issue}");
        continue;
      }
      all_issues.push(issue);
    }
  }

  info!("total new issues to process: {}", all_issues.len());
  Ok(all_issues)
}

pub async fn fetch_resumable_issues(
  config: &Config,
  github: &GitHubClient,
  state: &StateTracker,
) -> Result<Vec<ForgeIssue>> {
  let resumable = state.resumable_issues();
  let mut issues = Vec::new();

  for (full_repo, number) in resumable {
    let repo_config = config.repos.iter().find(|r| {
      let (owner, name) = r.owner_repo();
      format!("{owner}/{name}") == full_repo
    });
    let Some(repo_config) = repo_config else {
      info!("skipping resumable issue {full_repo}#{number}: repo not in config");
      continue;
    };

    let (owner, repo_name) = repo_config.owner_repo();
    match github.fetch_issue(owner, repo_name, number).await {
      Ok(issue) => {
        info!("resuming: {issue}");
        issues.push(issue);
      }
      Err(e) => {
        info!("failed to fetch resumable issue {full_repo}#{number}: {e}");
      }
    }
  }

  info!("resumable issues: {}", issues.len());
  Ok(issues)
}

pub async fn fetch_clarified_issues(
  config: &Config,
  github: &GitHubClient,
  state: &StateTracker,
) -> Result<Vec<ForgeIssue>> {
  let needs_clarification = state.needs_clarification_issues();
  let mut issues = Vec::new();

  for (full_repo, number) in needs_clarification {
    let repo_config = config.repos.iter().find(|r| {
      let (owner, name) = r.owner_repo();
      format!("{owner}/{name}") == full_repo
    });
    let Some(repo_config) = repo_config else {
      continue;
    };

    if clarification::check_clarification(&repo_config.path, number)?.is_none() {
      continue;
    }

    let (owner, repo_name) = repo_config.owner_repo();
    match github.fetch_issue(owner, repo_name, number).await {
      Ok(issue) => {
        info!("clarification answered, re-processing: {issue}");
        issues.push(issue);
      }
      Err(e) => {
        info!("failed to fetch clarified issue {full_repo}#{number}: {e}");
      }
    }
  }

  info!("clarified issues: {}", issues.len());
  Ok(issues)
}

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

  let (owner, repo_name) = repo.owner_repo();
  let full_repo = format!("{owner}/{repo_name}");

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

    if state.is_terminal(&full_repo, number) {
      info!("skipping terminal local task: {full_repo}#{number}");
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
      owner: owner.to_string(),
      repo: repo_name.to_string(),
      created_at: chrono::Utc::now(),
      source: TaskSource::Local,
    });
  }

  Ok(issues)
}
