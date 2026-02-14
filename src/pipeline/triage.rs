use std::time::Duration;

use serde::{Deserialize, Serialize};
use tracing::info;

use crate::claude::model;
use crate::claude::runner::ClaudeRunner;
use crate::config::Config;
use crate::error::Result;
use crate::github::issue::ForgeIssue;
use crate::pipeline::clarification::ClarificationContext;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeepTriageResult {
    pub complexity: String,
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

    pub fn complexity(&self) -> model::Complexity {
        self.complexity.parse().unwrap_or(model::Complexity::Medium)
    }
}

pub enum ConsultationOutcome {
    Resolved(DeepTriageResult),
    NeedsClarification(String),
}

pub fn deep_triage(
    issue: &ForgeIssue,
    config: &Config,
    runner: &ClaudeRunner,
    repo_path: &std::path::Path,
    clarification: Option<&ClarificationContext>,
) -> Result<DeepTriageResult> {
    let deep_model = model::resolve(&config.settings.models.triage_deep);

    let labels = issue.labels.join(", ");

    let clarification_section = if let Some(ctx) = clarification {
        format!(
            r#"

## Previous Analysis (from prior triage attempt)
Relevant files: {files}
Plan: {plan}
Context: {context}

## Clarification from maintainer
{answer}

Use the previous analysis as a starting point. The clarification above resolves
questions from the prior attempt. Update the plan accordingly."#,
            files = ctx.previous_analysis.relevant_files.join(", "),
            plan = ctx.previous_analysis.plan,
            context = ctx.previous_analysis.context,
            answer = ctx.answer,
        )
    } else {
        String::new()
    };

    let system_prompt = r#"You are a triage agent. Explore this repository's codebase to create a detailed implementation plan for the given issue.

Respond with ONLY a JSON object (no markdown):
{
  "complexity": "<low|medium|high>",
  "plan": "<detailed implementation plan>",
  "relevant_files": ["<file paths that need modification>"],
  "implementation_steps": ["<ordered list of concrete implementation steps>"],
  "context": "<relevant codebase context: patterns, conventions, dependencies>"
}"#;

    let prompt = format!(
        r#"Repository: {repo}
Issue #{number}: {title}
Labels: {labels}

{body}{clarification_section}"#,
        repo = issue.full_repo(),
        number = issue.number,
        title = issue.title,
        labels = labels,
        body = issue.body,
        clarification_section = clarification_section,
    );

    let timeout = Some(Duration::from_secs(config.settings.triage_timeout_secs));

    info!("deep triaging: {issue}");
    let result: DeepTriageResult =
        runner.run_json(&prompt, system_prompt, deep_model, repo_path, timeout)?;

    info!(
        "deep triage: complexity={}, {} relevant files, {} steps, sufficient={}",
        result.complexity,
        result.relevant_files.len(),
        result.implementation_steps.len(),
        result.is_sufficient(),
    );

    Ok(result)
}

pub fn consult(
    issue: &ForgeIssue,
    deep_result: &DeepTriageResult,
    config: &Config,
    runner: &ClaudeRunner,
    repo_path: &std::path::Path,
) -> Result<ConsultationOutcome> {
    let complex_model = model::resolve(&config.settings.models.complex);

    let system_prompt = r#"You are a senior consulting agent. A triage agent produced insufficient results for the given issue. Explore the codebase yourself, fill in the gaps, and produce a complete implementation plan. If the issue is genuinely unclear, respond with "needs_clarification".

Respond with ONLY a JSON object (no markdown):

If resolved:
{ "status": "resolved", "complexity": "...", "plan": "...", "relevant_files": [...], "implementation_steps": [...], "context": "..." }

If needs clarification:
{ "status": "needs_clarification", "message": "..." }"#;

    let prompt = format!(
        r#"Repository: {repo}
Issue #{number}: {title}

{body}

## Previous triage attempt (insufficient):
- Plan: {prev_plan}
- Relevant files: {prev_files}
- Steps: {prev_steps}
- Context: {prev_context}"#,
        repo = issue.full_repo(),
        number = issue.number,
        title = issue.title,
        body = issue.body,
        prev_plan = deep_result.plan,
        prev_files = deep_result.relevant_files.join(", "),
        prev_steps = deep_result.implementation_steps.join("; "),
        prev_context = deep_result.context,
    );

    let timeout = Some(Duration::from_secs(config.settings.triage_timeout_secs));

    info!("consulting on: {issue}");
    let raw: serde_json::Value =
        runner.run_json(&prompt, system_prompt, complex_model, repo_path, timeout)?;

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
