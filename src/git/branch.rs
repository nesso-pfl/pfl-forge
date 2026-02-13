use std::path::Path;
use std::process::Command;

use tracing::info;

use crate::error::{ForgeError, Result};

pub fn push(repo_path: &Path, branch: &str) -> Result<()> {
    info!("pushing branch {branch}");

    let output = Command::new("git")
        .args(["push", "-u", "origin", branch])
        .current_dir(repo_path)
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(ForgeError::Git(format!("push failed: {stderr}")));
    }

    Ok(())
}

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

pub fn delete_branch(repo_path: &Path, branch: &str) -> Result<()> {
    let output = Command::new("git")
        .args(["branch", "-D", branch])
        .current_dir(repo_path)
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(ForgeError::Git(format!("branch delete failed: {stderr}")));
    }

    Ok(())
}
