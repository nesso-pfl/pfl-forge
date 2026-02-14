use std::time::Duration;

use tracing::info;

use crate::agents::analyze::AnalysisResult;
use crate::claude::model;
use crate::claude::runner::ClaudeRunner;
use crate::config::Config;
use crate::error::Result;
use crate::prompt;
use crate::task::ForgeTask;

pub enum ArchitectOutcome {
  Resolved(AnalysisResult),
  NeedsClarification(String),
}

pub fn resolve(
  forge_task: &ForgeTask,
  analysis: &AnalysisResult,
  config: &Config,
  runner: &ClaudeRunner,
  repo_path: &std::path::Path,
) -> Result<ArchitectOutcome> {
  let complex_model = model::resolve(&config.models.complex);

  let prompt = format!(
    r#"Task {id}: {title}

{body}

## Previous analysis attempt (insufficient):
- Plan: {prev_plan}
- Relevant files: {prev_files}
- Steps: {prev_steps}
- Context: {prev_context}"#,
    id = forge_task.id,
    title = forge_task.title,
    body = forge_task.body,
    prev_plan = analysis.plan,
    prev_files = analysis.relevant_files.join(", "),
    prev_steps = analysis.implementation_steps.join("; "),
    prev_context = analysis.context,
  );

  let timeout = Some(Duration::from_secs(config.triage_timeout_secs));

  info!("architect resolving: {forge_task}");
  let raw: serde_json::Value = runner.run_json(
    &prompt,
    prompt::ARCHITECT,
    complex_model,
    repo_path,
    timeout,
  )?;

  let status = raw
    .get("status")
    .and_then(|v| v.as_str())
    .unwrap_or("needs_clarification");

  if status == "resolved" {
    let result: AnalysisResult = serde_json::from_value(raw)
      .map_err(|e| crate::error::ForgeError::Claude(format!("architect parse: {e}")))?;
    info!(
      "architect resolved with {} files",
      result.relevant_files.len()
    );
    Ok(ArchitectOutcome::Resolved(result))
  } else {
    let message = raw
      .get("message")
      .and_then(|v| v.as_str())
      .unwrap_or("Unable to determine implementation plan")
      .to_string();
    info!("architect needs clarification: {message}");
    Ok(ArchitectOutcome::NeedsClarification(message))
  }
}
