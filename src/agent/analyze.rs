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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChildIntentProposal {
  pub title: String,
  pub body: String,
}

#[derive(Debug, Clone)]
pub enum AnalysisOutcome {
  Tasks(AnalysisResult),
  ChildIntents(Vec<ChildIntentProposal>),
  NeedsClarification { clarifications: Vec<String> },
}

#[derive(Deserialize)]
struct RawAnalysis {
  #[serde(default = "default_outcome")]
  outcome: String,
  #[serde(default)]
  complexity: String,
  #[serde(default)]
  plan: String,
  #[serde(default)]
  relevant_files: Vec<String>,
  #[serde(default)]
  implementation_steps: Vec<String>,
  #[serde(default)]
  context: String,
  #[serde(default)]
  child_intents: Vec<ChildIntentProposal>,
  #[serde(default)]
  clarifications: Vec<String>,
}

fn default_outcome() -> String {
  "task".into()
}

impl From<RawAnalysis> for AnalysisOutcome {
  fn from(raw: RawAnalysis) -> Self {
    match raw.outcome.as_str() {
      "child_intents" => AnalysisOutcome::ChildIntents(raw.child_intents),
      "needs_clarification" => AnalysisOutcome::NeedsClarification {
        clarifications: raw.clarifications,
      },
      _ => AnalysisOutcome::Tasks(AnalysisResult {
        complexity: raw.complexity,
        plan: raw.plan,
        relevant_files: raw.relevant_files,
        implementation_steps: raw.implementation_steps,
        context: raw.context,
      }),
    }
  }
}

pub fn analyze(
  intent: &Intent,
  config: &Config,
  runner: &impl Claude,
  repo_path: &std::path::Path,
) -> Result<AnalysisOutcome> {
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
  let raw: RawAnalysis =
    runner.run_json(&prompt, prompt::ANALYZE, deep_model, repo_path, timeout)?;
  let outcome = AnalysisOutcome::from(raw);

  match &outcome {
    AnalysisOutcome::Tasks(result) => {
      info!(
        "analysis: complexity={}, {} relevant files, {} steps, sufficient={}",
        result.complexity,
        result.relevant_files.len(),
        result.implementation_steps.len(),
        result.is_sufficient(),
      );
    }
    AnalysisOutcome::ChildIntents(intents) => {
      info!("analysis: decomposed into {} child intents", intents.len());
    }
    AnalysisOutcome::NeedsClarification { clarifications } => {
      info!(
        "analysis: needs clarification ({} questions)",
        clarifications.len()
      );
    }
  }

  Ok(outcome)
}
