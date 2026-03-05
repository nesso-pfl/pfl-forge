use std::time::Duration;

use serde::{Deserialize, Serialize};
use tracing::info;

use crate::claude::model;
use crate::claude::runner::{Claude, ClaudeMetadata, SessionMode};
use crate::config::Config;
use crate::error::Result;
use crate::intent::registry::Intent;
use crate::prompt;

/// Summary of another active intent, passed to Analyze Agent for dependency detection.
#[derive(Debug, Clone)]
pub struct ActiveIntentContext {
  pub id: String,
  pub title: String,
  pub status: String,
  pub relevant_files: Vec<String>,
  pub plan: Option<String>,
}

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
  #[serde(default)]
  depends_on_intents: Vec<String>,
  #[serde(default)]
  observations: Vec<String>,
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
  active_intents: &[ActiveIntentContext],
  session: &SessionMode,
) -> Result<(AnalysisOutcome, ClaudeMetadata, Vec<String>, Vec<String>)> {
  let deep_model = model::resolve(&config.models.analyze);

  // When resuming from clarification, send only the answers
  let prompt = if matches!(session, SessionMode::Resume(_)) {
    build_clarification_resume_prompt(intent)
  } else {
    build_full_prompt(intent, active_intents)
  };

  let timeout = Some(Duration::from_secs(config.analyze_timeout_secs));

  info!("analyzing: {intent}");
  let (raw, metadata): (RawAnalysis, _) = runner.run_json_with_meta(
    &prompt,
    prompt::ANALYZE,
    deep_model,
    repo_path,
    timeout,
    session,
  )?;
  let depends_on_intents = raw.depends_on_intents.clone();
  let observations = raw.observations.clone();
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

  Ok((outcome, metadata, depends_on_intents, observations))
}

fn build_full_prompt(intent: &Intent, active_intents: &[ActiveIntentContext]) -> String {
  let mut prompt = format!(
    "Intent {id}: {title}\n\n{body}",
    id = intent.id(),
    title = intent.title,
    body = intent.body,
  );

  // Include answered clarifications from previous runs
  let answered: Vec<_> = intent
    .clarifications
    .iter()
    .filter(|c| c.answer.is_some())
    .collect();
  if !answered.is_empty() {
    prompt.push_str("\n\n## Human Decisions\n\n");
    for c in &answered {
      prompt.push_str(&format!(
        "Q: {}\nA: {}\n\n",
        c.question,
        c.answer.as_ref().unwrap()
      ));
    }
  }

  if !active_intents.is_empty() {
    prompt.push_str("\n\n## Active Intents\n\n");
    for ai in active_intents {
      prompt.push_str(&format!("- **{}** ({}): {}\n", ai.id, ai.status, ai.title));
      if !ai.relevant_files.is_empty() {
        prompt.push_str(&format!("  files: {}\n", ai.relevant_files.join(", ")));
      }
      if let Some(plan) = &ai.plan {
        let summary: String = plan.chars().take(200).collect();
        prompt.push_str(&format!("  plan: {summary}\n"));
      }
    }
  }

  prompt
}

fn build_clarification_resume_prompt(intent: &Intent) -> String {
  let mut prompt = String::from("Clarification answers:\n\n");
  for c in &intent.clarifications {
    if let Some(ref answer) = c.answer {
      prompt.push_str(&format!("Q: {}\nA: {}\n\n", c.question, answer));
    }
  }
  prompt.push_str("Please continue with the analysis using these answers.");
  prompt
}
