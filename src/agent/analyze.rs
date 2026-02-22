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
pub struct TaskSpec {
  #[serde(default)]
  pub id: String,
  #[serde(default)]
  pub title: String,
  pub complexity: String,
  pub plan: String,
  pub relevant_files: Vec<String>,
  pub implementation_steps: Vec<String>,
  #[serde(default)]
  pub context: String,
  #[serde(default)]
  pub depends_on: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChildIntentProposal {
  pub title: String,
  pub body: String,
}

#[derive(Debug, Clone)]
pub enum AnalysisOutcome {
  Tasks(Vec<TaskSpec>),
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
  #[serde(default)]
  tasks: Vec<TaskSpec>,
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
      _ => {
        if raw.tasks.is_empty() {
          // Backward compat: single task from top-level fields
          AnalysisOutcome::Tasks(vec![TaskSpec {
            id: String::new(),
            title: String::new(),
            complexity: raw.complexity,
            plan: raw.plan,
            relevant_files: raw.relevant_files,
            implementation_steps: raw.implementation_steps,
            context: raw.context,
            depends_on: vec![],
          }])
        } else {
          AnalysisOutcome::Tasks(raw.tasks)
        }
      }
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
    AnalysisOutcome::Tasks(specs) => {
      info!("analysis: {} task(s)", specs.len());
      for spec in specs {
        info!(
          "  task: complexity={}, {} relevant files, {} steps",
          spec.complexity,
          spec.relevant_files.len(),
          spec.implementation_steps.len(),
        );
      }
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
