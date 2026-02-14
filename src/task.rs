use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ForgeIssue {
  pub id: String,
  pub title: String,
  pub body: String,
  pub labels: Vec<String>,
  pub created_at: DateTime<Utc>,
}

impl ForgeIssue {
  pub fn branch_name(&self) -> String {
    format!("forge/{}", self.id)
  }

  pub fn worktree_path(&self, worktree_dir: &str) -> std::path::PathBuf {
    std::path::PathBuf::from(worktree_dir).join(format!("forge/{}", self.id))
  }
}

impl std::fmt::Display for ForgeIssue {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    write!(f, "{}: {}", self.id, self.title)
  }
}
