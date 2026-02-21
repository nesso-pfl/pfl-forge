use std::time::Duration;

use serde::{Deserialize, Serialize};
use tracing::info;

use crate::claude::model;
use crate::claude::runner::Claude;
use crate::config::Config;
use crate::error::Result;
use crate::intent::registry::Intent;
use crate::prompt;

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
  intent: &Intent,
  config: &Config,
  runner: &impl Claude,
  repo_path: &std::path::Path,
) -> Result<AnalysisResult> {
  let deep_model = model::resolve(&config.models.analyze);

  let prompt = format!(
    r#"Intent {id}: {title}

{body}"#,
    id = intent.id(),
    title = intent.title,
    body = intent.body,
  );

  let timeout = Some(Duration::from_secs(config.analyze_timeout_secs));

  info!("analyzing: {intent}");
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
