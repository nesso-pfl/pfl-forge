use tracing::info;

use crate::config::Config;
use crate::error::Result;
use crate::github::client::GitHubClient;
use crate::github::issue::ForgeIssue;
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
