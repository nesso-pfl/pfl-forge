use std::path::Path;

use tracing::info;

use crate::agents::analyze::AnalysisResult;
use crate::error::Result;
use crate::task::ForgeTask;

pub struct ClarificationContext {
  pub previous_analysis: AnalysisResult,
  pub questions: String,
  pub answer: String,
}

fn clarification_dir(repo_path: &Path) -> std::path::PathBuf {
  repo_path.join(".forge").join("clarifications")
}

fn question_path(repo_path: &Path, task_id: &str) -> std::path::PathBuf {
  clarification_dir(repo_path).join(format!("{task_id}.md"))
}

fn answer_path(repo_path: &Path, task_id: &str) -> std::path::PathBuf {
  clarification_dir(repo_path).join(format!("{task_id}.answer.md"))
}

pub fn write_clarification(
  repo_path: &Path,
  forge_task: &ForgeTask,
  deep_result: &AnalysisResult,
  questions: &str,
) -> Result<()> {
  let dir = clarification_dir(repo_path);
  std::fs::create_dir_all(&dir)?;

  let content = format!(
    r#"# Clarification needed: Task {id}

## Task
{title}
{body}

## Previous Analysis
Relevant files: {files}
Plan: {plan}
Context: {context}

## Questions
{questions}
"#,
    id = forge_task.id,
    title = forge_task.title,
    body = forge_task.body,
    files = deep_result.relevant_files.join(", "),
    plan = deep_result.plan,
    context = deep_result.context,
    questions = questions,
  );

  let path = question_path(repo_path, &forge_task.id);
  std::fs::write(&path, &content)?;
  info!("wrote clarification file: {}", path.display());

  Ok(())
}

pub fn check_clarification(
  repo_path: &Path,
  task_id: &str,
) -> Result<Option<ClarificationContext>> {
  let q_path = question_path(repo_path, task_id);
  let a_path = answer_path(repo_path, task_id);

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
    "found clarification answer for task {task_id} ({} bytes)",
    answer.len()
  );

  Ok(Some(ClarificationContext {
    previous_analysis,
    questions,
    answer,
  }))
}

fn parse_question_file(content: &str) -> (AnalysisResult, String) {
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

  let result = AnalysisResult {
    complexity: "medium".to_string(),
    plan,
    relevant_files: files,
    implementation_steps: vec![],
    context,
  };

  (result, questions)
}

pub struct PendingClarification {
  pub task_id: String,
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
    let task_id = name.trim_end_matches(".md").to_string();
    // Skip if answer already exists
    if answer_path(repo_path, &task_id).exists() {
      continue;
    }
    let content = std::fs::read_to_string(entry.path())?;
    pending.push(PendingClarification { task_id, content });
  }

  pending.sort_by(|a, b| a.task_id.cmp(&b.task_id));
  Ok(pending)
}

pub fn write_answer(repo_path: &Path, task_id: &str, text: &str) -> Result<()> {
  let path = answer_path(repo_path, task_id);
  let dir = clarification_dir(repo_path);
  std::fs::create_dir_all(&dir)?;
  std::fs::write(&path, text)?;
  info!("wrote answer file: {}", path.display());
  Ok(())
}

pub fn cleanup_clarification(repo_path: &Path, task_id: &str) -> Result<()> {
  let q_path = question_path(repo_path, task_id);
  let a_path = answer_path(repo_path, task_id);

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
