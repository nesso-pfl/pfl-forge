use std::path::Path;

use tracing::info;

use crate::error::Result;
use crate::pipeline::triage::DeepTriageResult;
use crate::task::ForgeTask;

pub struct ClarificationContext {
  pub previous_analysis: DeepTriageResult,
  pub questions: String,
  pub answer: String,
}

fn clarification_dir(repo_path: &Path) -> std::path::PathBuf {
  repo_path.join(".forge").join("clarifications")
}

fn question_path(repo_path: &Path, issue_id: &str) -> std::path::PathBuf {
  clarification_dir(repo_path).join(format!("{issue_id}.md"))
}

fn answer_path(repo_path: &Path, issue_id: &str) -> std::path::PathBuf {
  clarification_dir(repo_path).join(format!("{issue_id}.answer.md"))
}

pub fn write_clarification(
  repo_path: &Path,
  issue: &ForgeTask,
  deep_result: &DeepTriageResult,
  questions: &str,
) -> Result<()> {
  let dir = clarification_dir(repo_path);
  std::fs::create_dir_all(&dir)?;

  let content = format!(
    r#"# Clarification needed: Issue {id}

## Issue
{title}
{body}

## Previous Analysis
Relevant files: {files}
Plan: {plan}
Context: {context}

## Questions
{questions}
"#,
    id = issue.id,
    title = issue.title,
    body = issue.body,
    files = deep_result.relevant_files.join(", "),
    plan = deep_result.plan,
    context = deep_result.context,
    questions = questions,
  );

  let path = question_path(repo_path, &issue.id);
  std::fs::write(&path, &content)?;
  info!("wrote clarification file: {}", path.display());

  Ok(())
}

pub fn check_clarification(
  repo_path: &Path,
  issue_id: &str,
) -> Result<Option<ClarificationContext>> {
  let q_path = question_path(repo_path, issue_id);
  let a_path = answer_path(repo_path, issue_id);

  if !a_path.exists() {
    return Ok(None);
  }

  let answer = std::fs::read_to_string(&a_path)?;
  if answer.trim().is_empty() {
    return Ok(None);
  }

  let q_content = std::fs::read_to_string(&q_path).unwrap_or_default();

  let (previous_analysis, questions) = parse_question_file(&q_content);

  info!(
    "found clarification answer for issue {issue_id} ({} bytes)",
    answer.len()
  );

  Ok(Some(ClarificationContext {
    previous_analysis,
    questions,
    answer,
  }))
}

fn parse_question_file(content: &str) -> (DeepTriageResult, String) {
  let mut plan = String::new();
  let mut files = Vec::new();
  let mut context = String::new();
  let mut questions = String::new();
  let mut current_section = "";

  for line in content.lines() {
    if line.starts_with("## Previous Analysis") {
      current_section = "analysis";
      continue;
    } else if line.starts_with("## Questions") {
      current_section = "questions";
      continue;
    } else if line.starts_with("## ") {
      current_section = "";
      continue;
    }

    match current_section {
      "analysis" => {
        if let Some(rest) = line.strip_prefix("Relevant files: ") {
          files = rest.split(", ").map(|s| s.to_string()).collect();
        } else if let Some(rest) = line.strip_prefix("Plan: ") {
          plan = rest.to_string();
        } else if let Some(rest) = line.strip_prefix("Context: ") {
          context = rest.to_string();
        }
      }
      "questions" => {
        if !questions.is_empty() {
          questions.push('\n');
        }
        questions.push_str(line);
      }
      _ => {}
    }
  }

  let result = DeepTriageResult {
    complexity: "medium".to_string(),
    plan,
    relevant_files: files,
    implementation_steps: vec![],
    context,
  };

  (result, questions)
}

pub struct PendingClarification {
  pub issue_id: String,
  pub content: String,
}

pub fn list_pending_clarifications(repo_path: &Path) -> Result<Vec<PendingClarification>> {
  let mut pending = Vec::new();

  let dir = clarification_dir(repo_path);
  if !dir.exists() {
    return Ok(pending);
  }
  let entries = std::fs::read_dir(&dir)?;
  for entry in entries {
    let entry = entry?;
    let name = entry.file_name().to_string_lossy().to_string();
    if name.ends_with(".answer.md") || !name.ends_with(".md") {
      continue;
    }
    let issue_id = name.trim_end_matches(".md").to_string();
    // Skip if answer already exists
    if answer_path(repo_path, &issue_id).exists() {
      continue;
    }
    let content = std::fs::read_to_string(entry.path())?;
    pending.push(PendingClarification { issue_id, content });
  }

  pending.sort_by(|a, b| a.issue_id.cmp(&b.issue_id));
  Ok(pending)
}

pub fn write_answer(repo_path: &Path, issue_id: &str, text: &str) -> Result<()> {
  let path = answer_path(repo_path, issue_id);
  let dir = clarification_dir(repo_path);
  std::fs::create_dir_all(&dir)?;
  std::fs::write(&path, text)?;
  info!("wrote answer file: {}", path.display());
  Ok(())
}

pub fn cleanup_clarification(repo_path: &Path, issue_id: &str) -> Result<()> {
  let q_path = question_path(repo_path, issue_id);
  let a_path = answer_path(repo_path, issue_id);

  if q_path.exists() {
    std::fs::remove_file(&q_path)?;
    info!("removed clarification file: {}", q_path.display());
  }
  if a_path.exists() {
    std::fs::remove_file(&a_path)?;
    info!("removed answer file: {}", a_path.display());
  }

  Ok(())
}
