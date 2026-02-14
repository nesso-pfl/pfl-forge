use std::path::Path;
use std::process::Command;

use tracing::info;

use crate::error::{ForgeError, Result};

pub fn commit_count(repo_path: &Path, base_branch: &str, branch: &str) -> Result<u32> {
	let output = Command::new("git")
		.args([
			"rev-list",
			"--count",
			&format!("origin/{base_branch}..{branch}"),
		])
		.current_dir(repo_path)
		.output()?;

	if !output.status.success() {
		let stderr = String::from_utf8_lossy(&output.stderr);
		return Err(ForgeError::Git(format!("rev-list failed: {stderr}")));
	}

	let count_str = String::from_utf8_lossy(&output.stdout).trim().to_string();
	count_str
		.parse()
		.map_err(|e| ForgeError::Git(format!("failed to parse commit count: {e}")))
}

pub fn rebase(worktree_path: &Path, base_branch: &str) -> Result<()> {
	info!("fetching origin/{base_branch}");
	let fetch = Command::new("git")
		.args(["fetch", "origin", base_branch])
		.current_dir(worktree_path)
		.output()?;

	if !fetch.status.success() {
		let stderr = String::from_utf8_lossy(&fetch.stderr);
		return Err(ForgeError::Git(format!("fetch failed: {stderr}")));
	}

	info!("rebasing onto origin/{base_branch}");
	let rebase = Command::new("git")
		.args(["rebase", &format!("origin/{base_branch}")])
		.current_dir(worktree_path)
		.output()?;

	if !rebase.status.success() {
		let stderr = String::from_utf8_lossy(&rebase.stderr);
		// Abort the failed rebase
		let _ = Command::new("git")
			.args(["rebase", "--abort"])
			.current_dir(worktree_path)
			.output();
		return Err(ForgeError::Git(format!("rebase failed: {stderr}")));
	}

	Ok(())
}
