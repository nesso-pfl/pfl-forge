use std::path::Path;

use serde::Deserialize;
use tracing::info;

use crate::error::{ForgeError, Result};

#[derive(Debug, Clone, Deserialize, Default)]
pub struct DraftFrontmatter {
  #[serde(rename = "type")]
  pub intent_type: Option<String>,
  pub risk: Option<String>,
}

#[derive(Debug, Clone)]
pub struct IntentDraft {
  pub title: String,
  pub body: String,
  pub intent_type: Option<String>,
  pub risk: Option<String>,
}

pub fn parse(content: &str) -> Result<IntentDraft> {
  let (frontmatter, body_text) = split_frontmatter(content)?;

  let fm: DraftFrontmatter = if frontmatter.is_empty() {
    DraftFrontmatter::default()
  } else {
    serde_yaml::from_str(&frontmatter)
      .map_err(|e| ForgeError::Parse(format!("invalid frontmatter: {e}")))?
  };

  let body_text = body_text.trim();
  if body_text.is_empty() {
    return Err(ForgeError::Parse("draft has no body".into()));
  }

  // First paragraph = title, rest = body
  let (title, body) = match body_text.find("\n\n") {
    Some(pos) => (
      body_text[..pos].trim().to_string(),
      body_text[pos..].trim().to_string(),
    ),
    None => (body_text.to_string(), String::new()),
  };

  Ok(IntentDraft {
    title,
    body,
    intent_type: fm.intent_type,
    risk: fm.risk,
  })
}

/// Scan `.forge/intent-drafts/*.md`, convert each to `.forge/intents/*.yaml`, and delete the draft.
/// Returns the list of converted intent IDs (file stems).
pub fn convert_drafts(repo_path: &Path) -> Result<Vec<String>> {
  let drafts_dir = repo_path.join(".forge").join("intent-drafts");
  if !drafts_dir.exists() {
    return Ok(Vec::new());
  }

  let intents_dir = repo_path.join(".forge").join("intents");
  std::fs::create_dir_all(&intents_dir)?;

  let mut entries: Vec<_> = std::fs::read_dir(&drafts_dir)?
    .filter_map(|e| e.ok())
    .collect();
  entries.sort_by_key(|e| e.file_name());

  let mut converted = Vec::new();

  for entry in entries {
    let path = entry.path();
    if path.extension().and_then(|e| e.to_str()) != Some("md") {
      continue;
    }

    let stem = path
      .file_stem()
      .and_then(|s| s.to_str())
      .unwrap_or_default()
      .to_string();
    if stem.is_empty() {
      continue;
    }

    let intent_path = intents_dir.join(format!("{stem}.yaml"));
    if intent_path.exists() {
      info!("draft '{stem}': intent already exists, skipping");
      continue;
    }

    let content = std::fs::read_to_string(&path)?;
    let draft = parse(&content)?;
    let yaml = draft_to_yaml(&draft);

    std::fs::write(&intent_path, &yaml)?;
    std::fs::remove_file(&path)?;
    info!("draft '{stem}': converted to intent");
    converted.push(stem);
  }

  Ok(converted)
}

fn draft_to_yaml(draft: &IntentDraft) -> String {
  let mut yaml = String::new();
  yaml.push_str(&format!(
    "title: \"{}\"\n",
    draft.title.replace('"', "\\\"")
  ));
  yaml.push_str("body: |\n");
  if draft.body.is_empty() {
    yaml.push_str("  \n");
  } else {
    for line in draft.body.lines() {
      yaml.push_str(&format!("  {line}\n"));
    }
  }
  yaml.push_str("source: draft\nstatus: proposed\n");
  if let Some(t) = &draft.intent_type {
    yaml.push_str(&format!("type: {t}\n"));
  }
  if let Some(r) = &draft.risk {
    yaml.push_str(&format!("risk: {r}\n"));
  }
  yaml
}

fn split_frontmatter(content: &str) -> Result<(String, String)> {
  let trimmed = content.trim_start();
  if !trimmed.starts_with("---") {
    return Ok((String::new(), content.to_string()));
  }

  let after_first = &trimmed[3..];
  let end = after_first
    .find("\n---")
    .ok_or_else(|| ForgeError::Parse("unclosed frontmatter".into()))?;

  let frontmatter = after_first[..end].trim().to_string();
  let body = after_first[end + 4..].to_string();
  Ok((frontmatter, body))
}
