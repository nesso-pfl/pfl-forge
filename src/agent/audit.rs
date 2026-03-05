use std::path::Path;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tracing::info;

use crate::claude::model;
use crate::claude::runner::{Claude, ClaudeMetadata, SessionMode};
use crate::config::Config;
use crate::error::Result;
use crate::knowledge::observation::{self, Observation};
use crate::prompt;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditResult {
  pub observations: Vec<AuditObservation>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditObservation {
  pub content: String,
  pub evidence: Vec<AuditEvidence>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEvidence {
  #[serde(rename = "type")]
  pub evidence_type: String,
  #[serde(rename = "ref")]
  pub reference: String,
}

pub fn audit(
  config: &Config,
  runner: &impl Claude,
  repo_path: &Path,
  target_path: Option<&str>,
  intent_id: &str,
) -> Result<(AuditResult, ClaudeMetadata)> {
  let audit_model = model::resolve(&config.models.audit);

  let prompt = match target_path {
    Some(path) => format!("Audit the codebase at path: {path}"),
    None => "Audit the entire codebase.".to_string(),
  };

  let timeout = Some(Duration::from_secs(config.analyze_timeout_secs));

  info!("auditing: {}", target_path.unwrap_or("."));
  let (result, metadata): (AuditResult, _) = runner.run_json_with_meta(
    &prompt,
    prompt::AUDIT,
    audit_model,
    repo_path,
    timeout,
    &SessionMode::new_session(),
  )?;

  let obs_path = repo_path.join(".forge").join("observations.yaml");
  for obs in &result.observations {
    let evidence = obs
      .evidence
      .iter()
      .map(|e| observation::Evidence {
        evidence_type: e
          .evidence_type
          .parse()
          .unwrap_or(crate::knowledge::observation::EvidenceType::File),
        reference: e.reference.clone(),
      })
      .collect();

    let observation = Observation {
      content: obs.content.clone(),
      evidence,
      source: "audit".to_string(),
      intent_id: intent_id.to_string(),
      processed: false,
      created_at: Some(chrono::Utc::now().to_rfc3339()),
      source_session_id: metadata.session_id.clone(),
      processed_session_id: None,
    };
    observation::append(&obs_path, &observation)?;
  }

  info!("audit: {} observations recorded", result.observations.len());
  Ok((result, metadata))
}
