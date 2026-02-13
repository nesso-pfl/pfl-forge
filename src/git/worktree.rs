use std::path::{Path, PathBuf};
use std::process::Command;

use tracing::{debug, info};

use crate::error::{ForgeError, Result};

pub fn create(
    repo_path: &Path,
    worktree_dir: &str,
    branch: &str,
    base_branch: &str,
) -> Result<PathBuf> {
    let worktree_path = repo_path.join(worktree_dir).join(branch);

    if worktree_path.exists() {
        info!("worktree already exists: {}", worktree_path.display());
        return Ok(worktree_path);
    }

    if let Some(parent) = worktree_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    // Fetch latest base branch
    debug!("fetching latest {base_branch}");
    let fetch_output = Command::new("git")
        .args(["fetch", "origin", base_branch])
        .current_dir(repo_path)
        .output()?;

    if !fetch_output.status.success() {
        let stderr = String::from_utf8_lossy(&fetch_output.stderr);
        debug!("fetch warning (non-fatal): {stderr}");
    }

    info!("creating worktree: {}", worktree_path.display());
    let output = Command::new("git")
        .args([
            "worktree",
            "add",
            "-b",
            branch,
            worktree_path.to_str().unwrap(),
            &format!("origin/{base_branch}"),
        ])
        .current_dir(repo_path)
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        // Branch may already exist, try without -b
        if stderr.contains("already exists") {
            debug!("branch {branch} already exists, creating worktree without -b");
            let output2 = Command::new("git")
                .args([
                    "worktree",
                    "add",
                    worktree_path.to_str().unwrap(),
                    branch,
                ])
                .current_dir(repo_path)
                .output()?;

            if !output2.status.success() {
                let stderr2 = String::from_utf8_lossy(&output2.stderr);
                return Err(ForgeError::Git(format!("worktree add failed: {stderr2}")));
            }
        } else {
            return Err(ForgeError::Git(format!("worktree add failed: {stderr}")));
        }
    }

    Ok(worktree_path)
}

pub fn remove(repo_path: &Path, worktree_path: &Path) -> Result<()> {
    info!("removing worktree: {}", worktree_path.display());

    let output = Command::new("git")
        .args([
            "worktree",
            "remove",
            "--force",
            worktree_path.to_str().unwrap(),
        ])
        .current_dir(repo_path)
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(ForgeError::Git(format!("worktree remove failed: {stderr}")));
    }

    Ok(())
}

pub fn list(repo_path: &Path) -> Result<Vec<String>> {
    let output = Command::new("git")
        .args(["worktree", "list", "--porcelain"])
        .current_dir(repo_path)
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(ForgeError::Git(format!("worktree list failed: {stderr}")));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let worktrees: Vec<String> = stdout
        .lines()
        .filter_map(|line| line.strip_prefix("worktree "))
        .map(|s| s.to_string())
        .collect();

    Ok(worktrees)
}
