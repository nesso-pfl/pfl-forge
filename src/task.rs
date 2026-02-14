use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ForgeTask {
  #[serde(skip_serializing, default)]
  pub id: String,
  pub title: String,
  pub body: String,
  #[serde(default)]
  pub labels: Vec<String>,
  #[serde(skip_serializing, default = "Utc::now")]
  pub created_at: DateTime<Utc>,
}

impl ForgeTask {
  pub fn branch_name(&self) -> String {
    format!("forge/{}", self.id)
  }

  pub fn worktree_path(&self, worktree_dir: &str) -> std::path::PathBuf {
    std::path::PathBuf::from(worktree_dir).join(format!("forge/{}", self.id))
  }
}

impl std::fmt::Display for ForgeTask {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    write!(f, "{}: {}", self.id, self.title)
  }
}
