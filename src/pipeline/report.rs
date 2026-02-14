use tracing::info;

use crate::error::Result;
use crate::pipeline::execute::ExecuteResult;
use crate::state::tracker::{IssueStatus, SharedState};
use crate::task::ForgeIssue;

pub fn report(issue: &ForgeIssue, result: &ExecuteResult, state: &SharedState) -> Result<()> {
	let repo_name = &issue.repo_name;
	let branch = issue.branch_name();

	match result {
		ExecuteResult::Success { .. } => {}

		ExecuteResult::TestFailure { commits, .. } => {
			info!("test failure: {issue} with {commits} commits");
			info!("task {issue}: tests failed, branch {branch} left as-is");
			state.lock().unwrap().set_status(
				repo_name,
				issue.number,
				&issue.title,
				IssueStatus::TestFailure,
			)?;
		}

		ExecuteResult::Unclear(reason) => {
			info!("unclear result: {issue}: {reason}");
			state
				.lock()
				.unwrap()
				.set_error(repo_name, issue.number, reason)?;
		}

		ExecuteResult::Error(error) => {
			info!("error: {issue}: {error}");
			state
				.lock()
				.unwrap()
				.set_error(repo_name, issue.number, error)?;
		}
	}

	Ok(())
}
