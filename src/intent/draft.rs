use serde::Deserialize;

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
