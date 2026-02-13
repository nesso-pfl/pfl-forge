use serde::{Deserialize, Serialize};
use tracing::info;

use crate::claude::model;
use crate::claude::runner::ClaudeRunner;
use crate::config::Config;
use crate::error::Result;
use crate::github::issue::ForgeIssue;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TriageResult {
    pub actionable: bool,
    pub clarity: Clarity,
    pub complexity: String,
    pub summary: String,
    pub plan: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Clarity {
    Clear,
    Unclear,
    NeedsMoreInfo,
}

impl TriageResult {
    pub fn should_skip(&self) -> bool {
        !self.actionable
    }

    pub fn needs_clarification(&self) -> bool {
        self.clarity != Clarity::Clear
    }

    pub fn complexity(&self) -> model::Complexity {
        self.complexity.parse().unwrap_or(model::Complexity::Medium)
    }
}

pub fn triage(
    issue: &ForgeIssue,
    config: &Config,
    runner: &ClaudeRunner,
    repo_path: &std::path::Path,
) -> Result<TriageResult> {
    let triage_model = model::resolve(&config.settings.models.triage);

    let prompt = format!(
        r#"You are a triage agent. Analyze this GitHub issue and respond with ONLY a JSON object (no markdown, no explanation).

Repository: {repo}
Issue #{number}: {title}

{body}

Respond with this exact JSON structure:
{{
  "actionable": <boolean - can this be implemented as a code change?>,
  "clarity": "<clear|unclear|needs_more_info>",
  "complexity": "<low|medium|high>",
  "summary": "<one-line summary of what needs to be done>",
  "plan": "<brief implementation plan>"
}}"#,
        repo = issue.full_repo(),
        number = issue.number,
        title = issue.title,
        body = issue.body,
    );

    info!("triaging: {issue}");
    let result: TriageResult = runner.run_json(&prompt, triage_model, repo_path)?;

    info!(
        "triage result: actionable={}, clarity={:?}, complexity={}",
        result.actionable, result.clarity, result.complexity
    );

    Ok(result)
}
