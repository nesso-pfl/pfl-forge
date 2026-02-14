use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub enum TaskSource {
  #[default]
  GitHub,
  Local,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ForgeIssue {
  pub number: u64,
  pub title: String,
  pub body: String,
  pub labels: Vec<String>,
  pub repo_name: String,
  pub owner: String,
  pub repo: String,
  pub created_at: DateTime<Utc>,
  #[serde(default)]
  pub source: TaskSource,
}

impl ForgeIssue {
  pub fn branch_name(&self) -> String {
    format!("forge/issue-{}", self.number)
  }

  pub fn worktree_path(&self, worktree_dir: &str) -> std::path::PathBuf {
    std::path::PathBuf::from(worktree_dir).join(format!("forge/issue-{}", self.number))
  }

  pub fn full_repo(&self) -> String {
    format!("{}/{}", self.owner, self.repo)
  }
}

impl std::fmt::Display for ForgeIssue {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    write!(f, "{}#{}: {}", self.full_repo(), self.number, self.title)
  }
}
