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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeepTriageResult {
    pub plan: String,
    pub relevant_files: Vec<String>,
    pub implementation_steps: Vec<String>,
    pub context: String,
}

impl DeepTriageResult {
    pub fn is_sufficient(&self) -> bool {
        !self.relevant_files.is_empty()
            && !self.implementation_steps.is_empty()
            && !self.plan.is_empty()
    }
}

pub enum ConsultationOutcome {
    Resolved(DeepTriageResult),
    NeedsClarification(String),
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
  "summary": "<one-line summary of what needs to be done>"
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

pub fn deep_triage(
    issue: &ForgeIssue,
    triage: &TriageResult,
    config: &Config,
    runner: &ClaudeRunner,
    repo_path: &std::path::Path,
) -> Result<DeepTriageResult> {
    let deep_model = model::resolve(&config.settings.models.triage_deep);

    let prompt = format!(
        r#"You are a deep triage agent. Explore this repository's codebase to create a detailed implementation plan for the following issue.

Repository: {repo}
Issue #{number}: {title}

{body}

Quick triage summary: {summary}
Complexity: {complexity}

## Instructions

1. Use Read, Glob, and Grep to explore the codebase
2. Identify the relevant files that need to be modified
3. Determine the implementation steps needed
4. Understand the surrounding code context and patterns

Respond with ONLY a JSON object (no markdown, no explanation):
{{
  "plan": "<detailed implementation plan>",
  "relevant_files": ["<file paths that need modification>"],
  "implementation_steps": ["<ordered list of concrete implementation steps>"],
  "context": "<relevant codebase context: patterns, conventions, dependencies>"
}}"#,
        repo = issue.full_repo(),
        number = issue.number,
        title = issue.title,
        body = issue.body,
        summary = triage.summary,
        complexity = triage.complexity,
    );

    info!("deep triaging: {issue}");
    let result: DeepTriageResult = runner.run_json(&prompt, deep_model, repo_path)?;

    info!(
        "deep triage: {} relevant files, {} steps, sufficient={}",
        result.relevant_files.len(),
        result.implementation_steps.len(),
        result.is_sufficient(),
    );

    Ok(result)
}

pub fn consult(
    issue: &ForgeIssue,
    triage: &TriageResult,
    deep_result: &DeepTriageResult,
    config: &Config,
    runner: &ClaudeRunner,
    repo_path: &std::path::Path,
) -> Result<ConsultationOutcome> {
    let complex_model = model::resolve(&config.settings.models.complex);

    let prompt = format!(
        r#"You are a senior consulting agent. A deep triage agent attempted to analyze this issue but produced insufficient results. Your job is to explore the codebase yourself, fill in the gaps, and produce a complete implementation plan.

Repository: {repo}
Issue #{number}: {title}

{body}

Quick triage summary: {summary}

## Previous deep triage attempt (insufficient):
- Plan: {prev_plan}
- Relevant files found: {prev_files}
- Steps: {prev_steps}
- Context: {prev_context}

## Instructions

1. Use Read, Glob, and Grep to explore the codebase and fill in missing information
2. If you can produce a complete implementation plan, respond with a "resolved" result
3. If the issue is genuinely unclear or impossible to plan, respond with a "needs_clarification" result

Respond with ONLY a JSON object (no markdown, no explanation):

If resolved:
{{
  "status": "resolved",
  "plan": "<detailed implementation plan>",
  "relevant_files": ["<file paths>"],
  "implementation_steps": ["<ordered steps>"],
  "context": "<codebase context>"
}}

If needs clarification:
{{
  "status": "needs_clarification",
  "message": "<what information is missing and what questions to ask>"
}}"#,
        repo = issue.full_repo(),
        number = issue.number,
        title = issue.title,
        body = issue.body,
        summary = triage.summary,
        prev_plan = deep_result.plan,
        prev_files = deep_result.relevant_files.join(", "),
        prev_steps = deep_result.implementation_steps.join("; "),
        prev_context = deep_result.context,
    );

    info!("consulting on: {issue}");
    let raw: serde_json::Value = runner.run_json(&prompt, complex_model, repo_path)?;

    let status = raw
        .get("status")
        .and_then(|v| v.as_str())
        .unwrap_or("needs_clarification");

    if status == "resolved" {
        let result: DeepTriageResult = serde_json::from_value(raw)
            .map_err(|e| crate::error::ForgeError::Claude(format!("consultation parse: {e}")))?;
        info!("consultation resolved with {} files", result.relevant_files.len());
        Ok(ConsultationOutcome::Resolved(result))
    } else {
        let message = raw
            .get("message")
            .and_then(|v| v.as_str())
            .unwrap_or("Unable to determine implementation plan")
            .to_string();
        info!("consultation needs clarification: {message}");
        Ok(ConsultationOutcome::NeedsClarification(message))
    }
}
