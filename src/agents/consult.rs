use std::time::Duration;

use tracing::info;

use crate::agents::triage::DeepTriageResult;
use crate::claude::model;
use crate::claude::runner::ClaudeRunner;
use crate::config::Config;
use crate::error::Result;
use crate::prompt;
use crate::task::ForgeTask;

pub enum ConsultationOutcome {
  Resolved(DeepTriageResult),
  NeedsClarification(String),
}

pub fn consult(
  forge_task: &ForgeTask,
  deep_result: &DeepTriageResult,
  config: &Config,
  runner: &ClaudeRunner,
  repo_path: &std::path::Path,
) -> Result<ConsultationOutcome> {
  let complex_model = model::resolve(&config.models.complex);

  let prompt = format!(
    r#"Task {id}: {title}

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

  let timeout = Some(Duration::from_secs(config.triage_timeout_secs));

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
