pub mod clarification;
pub mod fetch;
pub mod work;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ForgeTask {
  #[serde(skip_serializing, default)]
  pub id: String,
  pub title: String,
  pub body: String,
  #[serde(default)]
  pub labels: Vec<String>,
}

impl ForgeTask {
  pub fn branch_name(&self) -> String {
    format!("forge/{}", self.id)
  }
}

impl std::fmt::Display for ForgeTask {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    write!(f, "{}: {}", self.id, self.title)
  }
}
