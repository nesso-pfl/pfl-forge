use std::time::Duration;

use serde::{Deserialize, Serialize};
use tracing::info;

use crate::claude::model;
use crate::claude::runner::ClaudeRunner;
use crate::config::Config;
use crate::error::Result;
use crate::prompt;
use crate::task::clarification::ClarificationContext;
use crate::task::Issue;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnalysisResult {
  pub complexity: String,
  pub plan: String,
  pub relevant_files: Vec<String>,
  pub implementation_steps: Vec<String>,
  pub context: String,
}

impl AnalysisResult {
  pub fn is_sufficient(&self) -> bool {
    !self.relevant_files.is_empty()
      && !self.implementation_steps.is_empty()
      && !self.plan.is_empty()
  }
}

pub fn analyze(
  issue: &Issue,
  config: &Config,
  runner: &ClaudeRunner,
  repo_path: &std::path::Path,
  clarification: Option<&ClarificationContext>,
) -> Result<AnalysisResult> {
  let deep_model = model::resolve(&config.models.triage_deep);

  let labels = issue.labels.join(", ");

  let clarification_section = if let Some(ctx) = clarification {
    format!(
      r#"

## Previous Analysis (from prior analysis attempt)
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
    r#"Task {id}: {title}
Labels: {labels}

{body}{clarification_section}"#,
    id = issue.id,
    title = issue.title,
    labels = labels,
    body = issue.body,
    clarification_section = clarification_section,
  );

  let timeout = Some(Duration::from_secs(config.triage_timeout_secs));

  info!("analyzing: {issue}");
  let result: AnalysisResult =
    runner.run_json(&prompt, prompt::ANALYZE, deep_model, repo_path, timeout)?;

  info!(
    "analysis: complexity={}, {} relevant files, {} steps, sufficient={}",
    result.complexity,
    result.relevant_files.len(),
    result.implementation_steps.len(),
    result.is_sufficient(),
  );

  Ok(result)
}
