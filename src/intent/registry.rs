use std::path::Path;

use serde::{Deserialize, Serialize};
use tracing::info;

use crate::error::Result;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Clarification {
  pub question: String,
  pub answer: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum IntentStatus {
  #[default]
  Proposed,
  Approved,
  Implementing,
  Done,
  Blocked,
  Error,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Intent {
  #[serde(skip_serializing, default)]
  file_stem: String,
  pub title: String,
  pub body: String,
  #[serde(rename = "type")]
  pub intent_type: Option<String>,
  pub source: String,
  pub risk: Option<String>,
  #[serde(default)]
  pub status: IntentStatus,
  pub parent: Option<String>,
  #[serde(default)]
  pub clarifications: Vec<Clarification>,
  pub created_at: Option<String>,
}

impl Intent {
  pub fn id(&self) -> &str {
    &self.file_stem
  }

  pub fn branch_name(&self) -> String {
    format!("forge/{}", self.file_stem)
  }

  pub fn needs_clarification(&self) -> bool {
    self.clarifications.iter().any(|c| c.answer.is_none())
  }

  pub fn fetch_all(intents_dir: &Path) -> Result<Vec<Intent>> {
    if !intents_dir.exists() {
      info!("intents: 0");
      return Ok(Vec::new());
    }

    let mut entries: Vec<_> = std::fs::read_dir(intents_dir)?
      .filter_map(|e| e.ok())
      .collect();
    entries.sort_by_key(|e| e.file_name());

    let mut intents = Vec::new();
    for entry in entries {
      let path = entry.path();
      if path.extension().and_then(|e| e.to_str()) != Some("yaml") {
        continue;
      }

      let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or_default()
        .to_string();

      if stem.is_empty() {
        continue;
      }

      let content = std::fs::read_to_string(&path)?;
      let mut intent: Intent = serde_yaml::from_str(&content)?;
      intent.file_stem = stem;

      intents.push(intent);
    }

    info!("intents: {}", intents.len());
    Ok(intents)
  }
}

impl std::fmt::Display for Intent {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    write!(f, "{}: {}", self.file_stem, self.title)
  }
}
