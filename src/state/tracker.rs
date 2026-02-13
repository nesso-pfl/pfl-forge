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
    pub repo: String,
    pub number: u64,
    pub title: String,
    pub status: IssueStatus,
    pub branch: Option<String>,
    pub pr_number: Option<u64>,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum IssueStatus {
    Pending,
    Triaging,
    Skipped,
    NeedsClarification,
    Executing,
    Success,
    TestFailure,
    Error,
    PrCreated,
}

impl IssueStatus {
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            IssueStatus::PrCreated
                | IssueStatus::Skipped
                | IssueStatus::NeedsClarification
                | IssueStatus::Error
                | IssueStatus::TestFailure
        )
    }

    pub fn is_resumable(&self) -> bool {
        matches!(
            self,
            IssueStatus::Triaging
                | IssueStatus::Executing
                | IssueStatus::Error
                | IssueStatus::TestFailure
        )
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

    fn issue_key(repo: &str, number: u64) -> String {
        format!("{repo}#{number}")
    }

    pub fn get(&self, repo: &str, number: u64) -> Option<&IssueState> {
        self.state.issues.get(&Self::issue_key(repo, number))
    }

    pub fn into_shared(self) -> SharedState {
        Arc::new(Mutex::new(self))
    }

    pub fn is_processed(&self, repo: &str, number: u64) -> bool {
        self.get(repo, number)
            .is_some_and(|s| matches!(s.status, IssueStatus::PrCreated | IssueStatus::Skipped))
    }

    pub fn is_terminal(&self, repo: &str, number: u64) -> bool {
        self.get(repo, number).is_some_and(|s| s.status.is_terminal())
    }

    pub fn is_resumable(&self, repo: &str, number: u64) -> bool {
        self.get(repo, number).is_some_and(|s| s.status.is_resumable())
    }

    pub fn resumable_issues(&self) -> Vec<(String, u64)> {
        self.state
            .issues
            .values()
            .filter(|s| s.status.is_resumable())
            .map(|s| (s.repo.clone(), s.number))
            .collect()
    }

    pub fn needs_clarification_issues(&self) -> Vec<(String, u64)> {
        self.state
            .issues
            .values()
            .filter(|s| s.status == IssueStatus::NeedsClarification)
            .map(|s| (s.repo.clone(), s.number))
            .collect()
    }

    pub fn set_status(
        &mut self,
        repo: &str,
        number: u64,
        title: &str,
        status: IssueStatus,
    ) -> Result<()> {
        let key = Self::issue_key(repo, number);
        let entry = self.state.issues.entry(key).or_insert_with(|| IssueState {
            repo: repo.to_string(),
            number,
            title: title.to_string(),
            status: IssueStatus::Pending,
            branch: None,
            pr_number: None,
            started_at: None,
            completed_at: None,
            error: None,
        });

        info!("{repo}#{number}: {:?} -> {status:?}", entry.status);
        entry.status = status;
        self.save()
    }

    pub fn set_branch(&mut self, repo: &str, number: u64, branch: &str) -> Result<()> {
        let key = Self::issue_key(repo, number);
        if let Some(entry) = self.state.issues.get_mut(&key) {
            entry.branch = Some(branch.to_string());
            self.save()?;
        }
        Ok(())
    }

    pub fn set_pr(&mut self, repo: &str, number: u64, pr_number: u64) -> Result<()> {
        let key = Self::issue_key(repo, number);
        if let Some(entry) = self.state.issues.get_mut(&key) {
            entry.pr_number = Some(pr_number);
            entry.status = IssueStatus::PrCreated;
            entry.completed_at = Some(Utc::now());
            self.save()?;
        }
        Ok(())
    }

    pub fn set_error(&mut self, repo: &str, number: u64, error: &str) -> Result<()> {
        let key = Self::issue_key(repo, number);
        if let Some(entry) = self.state.issues.get_mut(&key) {
            entry.status = IssueStatus::Error;
            entry.error = Some(error.to_string());
            entry.completed_at = Some(Utc::now());
            self.save()?;
        }
        Ok(())
    }

    pub fn reset_to_pending(&mut self, repo: &str, number: u64) -> Result<()> {
        let key = Self::issue_key(repo, number);
        if let Some(entry) = self.state.issues.get_mut(&key) {
            info!("{repo}#{number}: {:?} -> Pending (reset)", entry.status);
            entry.status = IssueStatus::Pending;
            entry.error = None;
            self.save()?;
        }
        Ok(())
    }

    pub fn set_started(&mut self, repo: &str, number: u64) -> Result<()> {
        let key = Self::issue_key(repo, number);
        if let Some(entry) = self.state.issues.get_mut(&key) {
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
                IssueStatus::Success | IssueStatus::PrCreated => summary.completed += 1,
                IssueStatus::Skipped | IssueStatus::NeedsClarification => summary.skipped += 1,
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
            .set_status("owner/repo", 1, "Test issue", IssueStatus::Executing)
            .unwrap();
        tracker.set_branch("owner/repo", 1, "forge/issue-1").unwrap();

        // Reload and verify
        let tracker2 = StateTracker::load(&path).unwrap();
        let state = tracker2.get("owner/repo", 1).unwrap();
        assert_eq!(state.status, IssueStatus::Executing);
        assert_eq!(state.branch.as_deref(), Some("forge/issue-1"));
    }

    #[test]
    fn test_is_terminal() {
        assert!(IssueStatus::PrCreated.is_terminal());
        assert!(IssueStatus::Skipped.is_terminal());
        assert!(IssueStatus::NeedsClarification.is_terminal());
        assert!(IssueStatus::Error.is_terminal());
        assert!(IssueStatus::TestFailure.is_terminal());
        assert!(!IssueStatus::Pending.is_terminal());
        assert!(!IssueStatus::Triaging.is_terminal());
        assert!(!IssueStatus::Executing.is_terminal());
        assert!(!IssueStatus::Success.is_terminal());
    }

    #[test]
    fn test_is_resumable() {
        assert!(IssueStatus::Triaging.is_resumable());
        assert!(IssueStatus::Executing.is_resumable());
        assert!(IssueStatus::Error.is_resumable());
        assert!(IssueStatus::TestFailure.is_resumable());
        assert!(!IssueStatus::Pending.is_resumable());
        assert!(!IssueStatus::PrCreated.is_resumable());
        assert!(!IssueStatus::Skipped.is_resumable());
        assert!(!IssueStatus::Success.is_resumable());
    }

    #[test]
    fn test_is_processed() {
        let tmp = NamedTempFile::new().unwrap();
        let mut tracker = StateTracker::load(tmp.path()).unwrap();

        tracker
            .set_status("owner/repo", 1, "Test", IssueStatus::PrCreated)
            .unwrap();
        assert!(tracker.is_processed("owner/repo", 1));

        tracker
            .set_status("owner/repo", 2, "Test2", IssueStatus::Executing)
            .unwrap();
        assert!(!tracker.is_processed("owner/repo", 2));
    }
}
