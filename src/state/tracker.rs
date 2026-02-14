use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tracing::info;

use crate::error::Result;

pub type SharedState = Arc<Mutex<StateTracker>>;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StateFile {
  #[serde(default)]
  pub issues: HashMap<String, IssueState>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IssueState {
  pub id: String,
  pub title: String,
  pub status: IssueStatus,
  pub branch: Option<String>,
  pub started_at: Option<DateTime<Utc>>,
  pub completed_at: Option<DateTime<Utc>>,
  pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum IssueStatus {
  Pending,
  Triaging,
  NeedsClarification,
  Executing,
  Success,
  TestFailure,
  Error,
}

impl IssueStatus {
  pub fn is_terminal(&self) -> bool {
    matches!(self, IssueStatus::Success | IssueStatus::NeedsClarification)
  }
}

pub struct StateTracker {
  path: PathBuf,
  state: StateFile,
}

impl StateTracker {
  pub fn load(path: &Path) -> Result<Self> {
    let state = if path.exists() {
      let content = std::fs::read_to_string(path)?;
      serde_yaml::from_str(&content)?
    } else {
      StateFile {
        issues: HashMap::new(),
      }
    };

    Ok(Self {
      path: path.to_path_buf(),
      state,
    })
  }

  pub fn save(&self) -> Result<()> {
    if let Some(parent) = self.path.parent() {
      std::fs::create_dir_all(parent)?;
    }
    let content = serde_yaml::to_string(&self.state)?;
    std::fs::write(&self.path, content)?;
    Ok(())
  }

  pub fn get(&self, id: &str) -> Option<&IssueState> {
    self.state.issues.get(id)
  }

  pub fn into_shared(self) -> SharedState {
    Arc::new(Mutex::new(self))
  }

  pub fn is_processed(&self, id: &str) -> bool {
    self
      .get(id)
      .is_some_and(|s| matches!(s.status, IssueStatus::Success))
  }

  pub fn is_terminal(&self, id: &str) -> bool {
    self.get(id).is_some_and(|s| s.status.is_terminal())
  }

  pub fn set_status(&mut self, id: &str, title: &str, status: IssueStatus) -> Result<()> {
    let entry = self
      .state
      .issues
      .entry(id.to_string())
      .or_insert_with(|| IssueState {
        id: id.to_string(),
        title: title.to_string(),
        status: IssueStatus::Pending,
        branch: None,
        started_at: None,
        completed_at: None,
        error: None,
      });

    info!("{id}: {:?} -> {status:?}", entry.status);
    entry.status = status;
    self.save()
  }

  pub fn set_branch(&mut self, id: &str, branch: &str) -> Result<()> {
    if let Some(entry) = self.state.issues.get_mut(id) {
      entry.branch = Some(branch.to_string());
      self.save()?;
    }
    Ok(())
  }

  pub fn set_error(&mut self, id: &str, error: &str) -> Result<()> {
    if let Some(entry) = self.state.issues.get_mut(id) {
      entry.status = IssueStatus::Error;
      entry.error = Some(error.to_string());
      entry.completed_at = Some(Utc::now());
      self.save()?;
    }
    Ok(())
  }

  pub fn reset_to_pending(&mut self, id: &str) -> Result<()> {
    if let Some(entry) = self.state.issues.get_mut(id) {
      info!("{id}: {:?} -> Pending (reset)", entry.status);
      entry.status = IssueStatus::Pending;
      entry.error = None;
      self.save()?;
    }
    Ok(())
  }

  pub fn set_started(&mut self, id: &str) -> Result<()> {
    if let Some(entry) = self.state.issues.get_mut(id) {
      entry.started_at = Some(Utc::now());
      self.save()?;
    }
    Ok(())
  }

  pub fn summary(&self) -> StateSummary {
    let mut summary = StateSummary::default();
    for state in self.state.issues.values() {
      match state.status {
        IssueStatus::Pending => summary.pending += 1,
        IssueStatus::Triaging | IssueStatus::Executing => summary.in_progress += 1,
        IssueStatus::Success => summary.completed += 1,
        IssueStatus::NeedsClarification => summary.skipped += 1,
        IssueStatus::TestFailure | IssueStatus::Error => summary.failed += 1,
      }
    }
    summary
  }

  pub fn all_issues(&self) -> &HashMap<String, IssueState> {
    &self.state.issues
  }
}

#[derive(Debug, Default)]
pub struct StateSummary {
  pub pending: usize,
  pub in_progress: usize,
  pub completed: usize,
  pub skipped: usize,
  pub failed: usize,
}

impl std::fmt::Display for StateSummary {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    write!(
      f,
      "pending={}, in_progress={}, completed={}, skipped={}, failed={}",
      self.pending, self.in_progress, self.completed, self.skipped, self.failed
    )
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use tempfile::NamedTempFile;

  #[test]
  fn test_state_tracker_roundtrip() {
    let tmp = NamedTempFile::new().unwrap();
    let path = tmp.path().to_path_buf();

    let mut tracker = StateTracker::load(&path).unwrap();
    tracker
      .set_status("abc123", "Test issue", IssueStatus::Executing)
      .unwrap();
    tracker.set_branch("abc123", "forge/abc123").unwrap();

    let tracker2 = StateTracker::load(&path).unwrap();
    let state = tracker2.get("abc123").unwrap();
    assert_eq!(state.status, IssueStatus::Executing);
    assert_eq!(state.branch.as_deref(), Some("forge/abc123"));
  }

  #[test]
  fn test_is_terminal() {
    assert!(IssueStatus::Success.is_terminal());
    assert!(IssueStatus::NeedsClarification.is_terminal());
    assert!(!IssueStatus::TestFailure.is_terminal());
    assert!(!IssueStatus::Error.is_terminal());
    assert!(!IssueStatus::Pending.is_terminal());
    assert!(!IssueStatus::Triaging.is_terminal());
    assert!(!IssueStatus::Executing.is_terminal());
  }

  #[test]
  fn test_is_processed() {
    let tmp = NamedTempFile::new().unwrap();
    let mut tracker = StateTracker::load(tmp.path()).unwrap();

    tracker
      .set_status("id1", "Test", IssueStatus::Success)
      .unwrap();
    assert!(tracker.is_processed("id1"));

    tracker
      .set_status("id2", "Test2", IssueStatus::Executing)
      .unwrap();
    assert!(!tracker.is_processed("id2"));
  }
}
