use std::path::Path;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tracing::info;

use crate::claude::model;
use crate::claude::runner::{Claude, ClaudeMetadata, SessionMode};
use crate::config::Config;
use crate::error::Result;
use crate::intent::registry::Intent;
use crate::knowledge::observation::{self, Observation};
use crate::knowledge::summary;
use crate::prompt;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReflectIntent {
  pub title: String,
  pub body: String,
  #[serde(rename = "type")]
  pub intent_type: Option<String>,
  pub risk: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReflectResult {
  pub intents: Vec<ReflectIntent>,
}

/// Run Reflect Agent on unprocessed observations for the given intent.
/// Returns generated intents and marks observations as processed.
pub fn reflect(
  intent: &Intent,
  config: &Config,
  runner: &impl Claude,
  repo_path: &Path,
  session: &SessionMode,
) -> Result<(ReflectResult, ClaudeMetadata)> {
  let obs_path = repo_path.join(".forge").join("observations.yaml");
  let all_obs = observation::load(&obs_path)?;
  let unprocessed: Vec<&Observation> = observation::unprocessed(&all_obs);

  if unprocessed.is_empty() {
    info!("reflect: no unprocessed observations");
    return Ok((ReflectResult { intents: vec![] }, ClaudeMetadata::default()));
  }

  let reflect_model = model::resolve(&config.models.reflect);

  let mut prompt = format!("## Intent: {title}\n\n", title = intent.title);

  // Include execution summary if available
  if let Ok(exec_summary) = summary::load(repo_path, &intent.id()) {
    prompt.push_str("## Execution Summary\n\n");
    if let Some(ref analyze) = exec_summary.analyze {
      prompt.push_str(&format!(
        "**Analysis**: complexity={}, {} task(s)\n",
        analyze.complexity, analyze.task_count
      ));
      prompt.push_str(&format!("**Plan**: {}\n\n", analyze.plan));
    }
    for ts in &exec_summary.tasks {
      prompt.push_str(&format!("### Task: {}\n", ts.task_id));
      if !ts.commits.is_empty() {
        prompt.push_str("Commits:\n");
        for c in &ts.commits {
          prompt.push_str(&format!("- {c}\n"));
        }
      }
      if let Some(ref rev) = ts.review {
        let verdict = if rev.approved { "approved" } else { "rejected" };
        prompt.push_str(&format!("Review: {verdict}\n"));
        for issue in &rev.issues {
          prompt.push_str(&format!("  issue: {issue}\n"));
        }
        for sug in &rev.suggestions {
          prompt.push_str(&format!("  suggestion: {sug}\n"));
        }
      }
      prompt.push('\n');
    }
  }

  prompt.push_str("## Unprocessed Observations\n\n");
  for obs in &unprocessed {
    prompt.push_str(&format!("- {}\n", obs.content));
  }

  let timeout = Some(Duration::from_secs(config.analyze_timeout_secs));

  info!("reflecting on {} observations", unprocessed.len());
  let (result, metadata): (ReflectResult, _) = runner.run_json_with_meta(
    &prompt,
    prompt::REFLECT,
    reflect_model,
    repo_path,
    timeout,
    session,
  )?;

  // Write generated intents to .forge/intents/
  let intents_dir = repo_path.join(".forge").join("intents");
  std::fs::create_dir_all(&intents_dir)?;
  for ri in &result.intents {
    let id = slug(&ri.title);
    let intent_yaml = GeneratedIntent {
      title: ri.title.clone(),
      body: ri.body.clone(),
      intent_type: ri.intent_type.clone(),
      source: "reflection".to_string(),
      risk: ri.risk.clone(),
      created_at: Some(chrono::Utc::now().to_rfc3339()),
    };
    let content = serde_yaml::to_string(&intent_yaml)?;
    std::fs::write(intents_dir.join(format!("{id}.yaml")), content)?;
  }

  // Mark observations as processed
  observation::mark_processed(&obs_path, &intent.id(), metadata.session_id.as_deref())?;

  info!("reflect: generated {} intents", result.intents.len());
  Ok((result, metadata))
}

#[derive(Serialize)]
struct GeneratedIntent {
  title: String,
  body: String,
  #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
  intent_type: Option<String>,
  source: String,
  #[serde(skip_serializing_if = "Option::is_none")]
  risk: Option<String>,
  created_at: Option<String>,
}

fn slug(title: &str) -> String {
  title
    .to_lowercase()
    .chars()
    .map(|c| if c.is_alphanumeric() { c } else { '-' })
    .collect::<String>()
    .split('-')
    .filter(|s| !s.is_empty())
    .collect::<Vec<_>>()
    .join("-")
}
