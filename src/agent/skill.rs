use std::path::Path;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tracing::info;

use crate::claude::model;
use crate::claude::runner::{Claude, ClaudeMetadata, SessionMode};
use crate::config::Config;
use crate::error::Result;
use crate::knowledge::history;
use crate::prompt;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObservedPattern {
  pub name: String,
  pub description: String,
  #[serde(default)]
  pub frequency: u32,
  #[serde(default)]
  pub examples: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObserveResult {
  pub patterns: Vec<ObservedPattern>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillDraft {
  pub name: String,
  pub description: String,
  pub instructions: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AbstractResult {
  pub skills: Vec<SkillDraft>,
}

/// Observe: analyze history to find repeated patterns.
pub fn observe(
  config: &Config,
  runner: &impl Claude,
  repo_path: &Path,
) -> Result<(ObserveResult, ClaudeMetadata)> {
  let history_dir = repo_path.join(".forge").join("knowledge").join("history");
  let entries = load_recent_history(&history_dir)?;

  if entries.is_empty() {
    info!("skill observe: no history entries");
    return Ok((
      ObserveResult { patterns: vec![] },
      ClaudeMetadata::default(),
    ));
  }

  let observe_model = model::resolve(&config.models.skill);
  let mut prompt = String::from("## Execution History\n\n");
  for entry in &entries {
    prompt.push_str(&format!(
      "- **{}** ({}): {}\n",
      entry.intent_id,
      entry.outcome_str(),
      entry.title,
    ));
    for sr in &entry.step_results {
      prompt.push_str(&format!("  - {}: {}s\n", sr.step, sr.duration_secs));
    }
  }

  let timeout = Some(Duration::from_secs(config.analyze_timeout_secs));

  info!("skill observe: analyzing {} history entries", entries.len());
  runner.run_json_with_meta(
    &prompt,
    prompt::SKILL_OBSERVE,
    observe_model,
    repo_path,
    timeout,
    &SessionMode::new_session(),
  )
}

/// Abstract: generalize observed patterns into reusable skill templates.
pub fn abstract_patterns(
  config: &Config,
  runner: &impl Claude,
  repo_path: &Path,
  patterns: &[ObservedPattern],
) -> Result<(AbstractResult, ClaudeMetadata)> {
  if patterns.is_empty() {
    info!("skill abstract: no patterns to abstract");
    return Ok((AbstractResult { skills: vec![] }, ClaudeMetadata::default()));
  }

  let abstract_model = model::resolve(&config.models.skill);
  let mut prompt = String::from("## Observed Patterns\n\n");
  for p in patterns {
    prompt.push_str(&format!(
      "- **{}** (frequency: {}): {}\n",
      p.name, p.frequency, p.description,
    ));
    if !p.examples.is_empty() {
      prompt.push_str(&format!("  examples: {}\n", p.examples.join(", ")));
    }
  }

  let timeout = Some(Duration::from_secs(config.analyze_timeout_secs));

  info!("skill abstract: processing {} patterns", patterns.len());
  runner.run_json_with_meta(
    &prompt,
    prompt::SKILL_ABSTRACT,
    abstract_model,
    repo_path,
    timeout,
    &SessionMode::new_session(),
  )
}

/// Record: write skill drafts as SKILL.md files.
pub fn record(repo_path: &Path, skills: &[SkillDraft]) -> Result<Vec<String>> {
  let skills_dir = repo_path.join(".claude").join("skills");
  let mut written = Vec::new();

  for skill in skills {
    let skill_dir = skills_dir.join(&skill.name);
    std::fs::create_dir_all(&skill_dir)?;

    let content = format!(
      "---\ndescription: {description}\n---\n\n{instructions}\n",
      description = skill.description,
      instructions = skill.instructions,
    );

    let path = skill_dir.join("SKILL.md");
    std::fs::write(&path, &content)?;
    info!("skill record: wrote {}", path.display());
    written.push(skill.name.clone());
  }

  Ok(written)
}

struct HistorySummary {
  intent_id: String,
  title: String,
  step_results: Vec<history::StepResult>,
  outcome: history::Outcome,
}

impl HistorySummary {
  fn outcome_str(&self) -> &str {
    match &self.outcome {
      history::Outcome::Success => "success",
      history::Outcome::Failed => "failed",
      history::Outcome::Escalated => "escalated",
    }
  }
}

fn load_recent_history(history_dir: &Path) -> Result<Vec<HistorySummary>> {
  if !history_dir.exists() {
    return Ok(Vec::new());
  }

  let mut entries = Vec::new();
  for entry in std::fs::read_dir(history_dir)? {
    let entry = entry?;
    let path = entry.path();
    if path.extension().and_then(|e| e.to_str()) != Some("yaml") {
      continue;
    }
    let content = std::fs::read_to_string(&path)?;
    let h: history::HistoryEntry = serde_yaml::from_str(&content)?;
    entries.push(HistorySummary {
      intent_id: h.intent_id,
      title: h.title,
      step_results: h.step_results,
      outcome: h.outcome,
    });
  }
  Ok(entries)
}
