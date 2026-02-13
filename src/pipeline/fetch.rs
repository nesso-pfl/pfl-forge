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
            if state.is_processed(&full_repo, issue.number) {
                info!("skipping already processed: {issue}");
                continue;
            }
            all_issues.push(issue);
        }
    }

    info!("total new issues to process: {}", all_issues.len());
    Ok(all_issues)
}
