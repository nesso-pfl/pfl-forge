use std::path::Path;
use std::process::Command;

use serde::{Deserialize, Serialize};
use tracing::info;

use crate::claude::model;
use crate::claude::runner::ClaudeRunner;
use crate::config::Config;
use crate::error::{ForgeError, Result};
use crate::github::issue::ForgeIssue;
use crate::pipeline::triage::DeepTriageResult;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewResult {
    pub approved: bool,
    pub issues: Vec<String>,
    pub suggestions: Vec<String>,
}

pub fn review(
    issue: &ForgeIssue,
    deep: &DeepTriageResult,
    config: &Config,
    runner: &ClaudeRunner,
    worktree_path: &Path,
    base_branch: &str,
) -> Result<ReviewResult> {
    let review_model = model::resolve(&config.settings.models.default);

    let diff = get_diff(worktree_path, base_branch)?;

    let prompt = format!(
        r#"You are a code review agent. Review the following diff for a GitHub issue implementation.

## Issue #{number}: {title}

{body}

## Implementation Plan

{plan}

## Diff

```
{diff}
```

## Review Criteria

1. Does the implementation satisfy the issue requirements?
2. Does the code follow existing patterns and conventions?
3. Are there any obvious bugs or security issues?
4. Is the implementation consistent with the plan?

Respond with ONLY a JSON object (no markdown, no explanation):
{{
  "approved": <boolean - true if the code is acceptable>,
  "issues": ["<list of problems found, empty if approved>"],
  "suggestions": ["<list of improvement suggestions, can be empty>"]
}}"#,
        number = issue.number,
        title = issue.title,
        body = issue.body,
        plan = deep.plan,
        diff = truncate_diff(&diff, 50000),
    );

    info!("reviewing: {issue}");
    let result: ReviewResult = runner.run_json(&prompt, review_model, worktree_path)?;

    info!(
        "review: approved={}, {} issues, {} suggestions",
        result.approved,
        result.issues.len(),
        result.suggestions.len(),
    );

    Ok(result)
}

fn get_diff(worktree_path: &Path, base_branch: &str) -> Result<String> {
    let output = Command::new("git")
        .args(["diff", &format!("origin/{base_branch}...HEAD")])
        .current_dir(worktree_path)
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(ForgeError::Git(format!("diff failed: {stderr}")));
    }

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

fn truncate_diff(diff: &str, max_len: usize) -> &str {
    if diff.len() <= max_len {
        diff
    } else {
        &diff[..max_len]
    }
}
