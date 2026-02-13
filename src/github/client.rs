use octocrab::Octocrab;
use tracing::info;

use crate::error::{ForgeError, Result};
use crate::github::issue::ForgeIssue;

pub struct GitHubClient {
    octocrab: Octocrab,
}

impl GitHubClient {
    pub fn new() -> Result<Self> {
        let token = std::env::var("GITHUB_TOKEN")
            .map_err(|_| ForgeError::GitHub("GITHUB_TOKEN not set".into()))?;

        let octocrab = Octocrab::builder()
            .personal_token(token)
            .build()
            .map_err(|e| ForgeError::GitHub(format!("failed to build octocrab: {e}")))?;

        Ok(Self { octocrab })
    }

    pub async fn fetch_issues(
        &self,
        owner: &str,
        repo: &str,
        label: &str,
    ) -> Result<Vec<ForgeIssue>> {
        info!("fetching issues for {owner}/{repo} with label={label}");

        let issues = self
            .octocrab
            .issues(owner, repo)
            .list()
            .labels(&[label.to_string()])
            .state(octocrab::params::State::Open)
            .per_page(30)
            .send()
            .await?;

        let forge_issues: Vec<ForgeIssue> = issues
            .items
            .into_iter()
            .filter(|i| i.pull_request.is_none())
            .map(|i| ForgeIssue {
                number: i.number,
                title: i.title,
                body: i.body.unwrap_or_default(),
                labels: i.labels.iter().map(|l| l.name.clone()).collect(),
                repo_name: repo.to_string(),
                owner: owner.to_string(),
                repo: repo.to_string(),
                created_at: i.created_at,
            })
            .collect();

        info!("found {} issues", forge_issues.len());
        Ok(forge_issues)
    }

    pub async fn create_pr(
        &self,
        owner: &str,
        repo: &str,
        title: &str,
        body: &str,
        head: &str,
        base: &str,
    ) -> Result<u64> {
        info!("creating PR: {title} ({head} -> {base})");

        let pr = self
            .octocrab
            .pulls(owner, repo)
            .create(title, head, base)
            .body(body)
            .send()
            .await?;

        let pr_number = pr.number;
        info!("created PR #{pr_number}");
        Ok(pr_number)
    }

    pub async fn add_comment(
        &self,
        owner: &str,
        repo: &str,
        issue_number: u64,
        body: &str,
    ) -> Result<()> {
        info!("adding comment to {owner}/{repo}#{issue_number}");

        self.octocrab
            .issues(owner, repo)
            .create_comment(issue_number, body)
            .await?;

        Ok(())
    }

    pub async fn add_label(
        &self,
        owner: &str,
        repo: &str,
        issue_number: u64,
        labels: &[String],
    ) -> Result<()> {
        info!("adding labels {labels:?} to {owner}/{repo}#{issue_number}");

        self.octocrab
            .issues(owner, repo)
            .add_labels(issue_number, labels)
            .await?;

        Ok(())
    }

    pub async fn remove_label(
        &self,
        owner: &str,
        repo: &str,
        issue_number: u64,
        label: &str,
    ) -> Result<()> {
        info!("removing label {label} from {owner}/{repo}#{issue_number}");

        self.octocrab
            .issues(owner, repo)
            .remove_label(issue_number, label)
            .await
            .map_err(|e| ForgeError::GitHub(format!("remove label failed: {e}")))?;

        Ok(())
    }
}
