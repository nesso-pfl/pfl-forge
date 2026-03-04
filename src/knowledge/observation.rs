use std::fs::{File, OpenOptions};
use std::path::Path;

use fs2::FileExt;
use serde::{Deserialize, Serialize};

use crate::error::Result;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceType {
  File,
  Skill,
  History,
  Decision,
}

impl std::str::FromStr for EvidenceType {
  type Err = String;

  fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
    match s {
      "file" => Ok(Self::File),
      "skill" => Ok(Self::Skill),
      "history" => Ok(Self::History),
      "decision" => Ok(Self::Decision),
      _ => Err(format!("unknown evidence type: {s}")),
    }
  }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Evidence {
  #[serde(rename = "type")]
  pub evidence_type: EvidenceType,
  #[serde(rename = "ref")]
  pub reference: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Observation {
  pub content: String,
  #[serde(default)]
  pub evidence: Vec<Evidence>,
  pub source: String,
  pub intent_id: String,
  #[serde(default)]
  pub processed: bool,
  pub created_at: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub source_session_id: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub processed_session_id: Option<String>,
}

pub fn load(path: &Path) -> Result<Vec<Observation>> {
  if !path.exists() {
    return Ok(Vec::new());
  }
  let content = std::fs::read_to_string(path)?;
  let observations: Vec<Observation> = serde_yaml::from_str(&content)?;
  Ok(observations)
}

fn lock_file(path: &Path) -> Result<File> {
  let lock_path = path.with_extension("yaml.lock");
  let file = OpenOptions::new()
    .create(true)
    .write(true)
    .truncate(false)
    .open(lock_path)?;
  file.lock_exclusive()?;
  Ok(file)
}

pub fn append(path: &Path, observation: &Observation) -> Result<()> {
  let _lock = lock_file(path)?;
  let mut observations = load(path)?;
  observations.push(observation.clone());
  let content = serde_yaml::to_string(&observations)?;
  std::fs::write(path, content)?;
  Ok(())
}

pub fn unprocessed(observations: &[Observation]) -> Vec<&Observation> {
  observations.iter().filter(|o| !o.processed).collect()
}

pub fn mark_processed(path: &Path, intent_id: &str, session_id: Option<&str>) -> Result<()> {
  let _lock = lock_file(path)?;
  let mut observations = load(path)?;
  for obs in &mut observations {
    if obs.intent_id == intent_id {
      obs.processed = true;
      obs.processed_session_id = session_id.map(String::from);
    }
  }
  let content = serde_yaml::to_string(&observations)?;
  std::fs::write(path, content)?;
  Ok(())
}
