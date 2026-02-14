use std::time::Duration;

use serde::{Deserialize, Serialize};
use tracing::info;

use crate::claude::model;
use crate::claude::runner::ClaudeRunner;
use crate::config::Config;
use crate::error::Result;
use crate::pipeline::clarification::ClarificationContext;
use crate::prompt;
use crate::task::ForgeTask;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeepTriageResult {
  pub complexity: String,
  pub plan: String,
  pub relevant_files: Vec<String>,
  pub implementation_steps: Vec<String>,
  pub context: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum WorkStatus {
  #[default]
  Pending,
  Executing,
  Completed,
  Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
  pub task_id: String,
  pub task_title: String,
  pub task_body: String,
  #[serde(default)]
  pub status: WorkStatus,
  pub complexity: String,
  pub plan: String,
  pub relevant_files: Vec<String>,
  pub implementation_steps: Vec<String>,
  pub context: String,
}

impl Task {
  pub fn from_triage(forge_task: &ForgeTask, deep: &DeepTriageResult) -> Self {
    Self {
      task_id: forge_task.id.clone(),
      task_title: forge_task.title.clone(),
      task_body: forge_task.body.clone(),
      status: WorkStatus::Pending,
      complexity: deep.complexity.clone(),
      plan: deep.plan.clone(),
      relevant_files: deep.relevant_files.clone(),
      implementation_steps: deep.implementation_steps.clone(),
      context: deep.context.clone(),
    }
  }

  pub fn complexity(&self) -> model::Complexity {
    self.complexity.parse().unwrap_or(model::Complexity::Medium)
  }
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
  forge_task: &ForgeTask,
  config: &Config,
  runner: &ClaudeRunner,
  repo_path: &std::path::Path,
  clarification: Option<&ClarificationContext>,
) -> Result<DeepTriageResult> {
  let deep_model = model::resolve(&config.settings.models.triage_deep);

  let labels = forge_task.labels.join(", ");

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

  let prompt = format!(
    r#"Issue {id}: {title}
Labels: {labels}

{body}{clarification_section}"#,
    id = forge_task.id,
    title = forge_task.title,
    labels = labels,
    body = forge_task.body,
    clarification_section = clarification_section,
  );

  let timeout = Some(Duration::from_secs(config.settings.triage_timeout_secs));

  info!("deep triaging: {forge_task}");
  let result: DeepTriageResult =
    runner.run_json(&prompt, prompt::DEEP_TRIAGE, deep_model, repo_path, timeout)?;

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
  forge_task: &ForgeTask,
  deep_result: &DeepTriageResult,
  config: &Config,
  runner: &ClaudeRunner,
  repo_path: &std::path::Path,
) -> Result<ConsultationOutcome> {
  let complex_model = model::resolve(&config.settings.models.complex);

  let prompt = format!(
    r#"Issue {id}: {title}

{body}

## Previous triage attempt (insufficient):
- Plan: {prev_plan}
- Relevant files: {prev_files}
- Steps: {prev_steps}
- Context: {prev_context}"#,
    id = forge_task.id,
    title = forge_task.title,
    body = forge_task.body,
    prev_plan = deep_result.plan,
    prev_files = deep_result.relevant_files.join(", "),
    prev_steps = deep_result.implementation_steps.join("; "),
    prev_context = deep_result.context,
  );

  let timeout = Some(Duration::from_secs(config.settings.triage_timeout_secs));

  info!("consulting on: {forge_task}");
  let raw: serde_json::Value =
    runner.run_json(&prompt, prompt::CONSULT, complex_model, repo_path, timeout)?;

  let status = raw
    .get("status")
    .and_then(|v| v.as_str())
    .unwrap_or("needs_clarification");

  if status == "resolved" {
    let result: DeepTriageResult = serde_json::from_value(raw)
      .map_err(|e| crate::error::ForgeError::Claude(format!("consultation parse: {e}")))?;
    info!(
      "consultation resolved with {} files",
      result.relevant_files.len()
    );
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
