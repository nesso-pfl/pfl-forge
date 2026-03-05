use pfl_forge::agent::implement;
use pfl_forge::agent::review::ReviewResult;
use pfl_forge::claude::runner::SessionMode;
use pfl_forge::intent::registry::Intent;
use pfl_forge::task::Task;

use crate::mock_claude::MockClaude;

fn sample_intent() -> Intent {
  let dir = tempfile::tempdir().unwrap();
  let yaml = "title: Fix bug\nbody: Fix the login validation bug\nsource: human\n";
  std::fs::write(dir.path().join("fix-bug.yaml"), yaml).unwrap();
  let intents = Intent::fetch_all(dir.path()).unwrap();
  std::mem::forget(dir);
  intents.into_iter().next().unwrap()
}

fn sample_task(intent: &Intent) -> Task {
  Task {
    id: intent.id().to_string(),
    title: intent.title.clone(),
    intent_id: intent.id().to_string(),
    status: pfl_forge::task::WorkStatus::Pending,
    complexity: "low".into(),
    plan: "Write validation logic".into(),
    relevant_files: vec!["src/login.rs".into()],
    implementation_steps: vec!["Add email check".into(), "Add tests".into()],
    context: "Login module context".into(),
    depends_on: vec![],
  }
}

#[test]
fn intentコンテキストで実装を実行する() {
  let mock = MockClaude::with_json("{}");
  let intent = sample_intent();
  let task = sample_task(&intent);
  let dir = tempfile::tempdir().unwrap();

  implement::run(
    &intent,
    &task,
    &mock,
    "sonnet",
    dir.path(),
    None,
    None,
    &SessionMode::new_session(),
  )
  .unwrap();

  let call = mock.last_call();
  assert!(call.prompt.contains("Fix bug"));
  assert!(call.prompt.contains("Fix the login validation bug"));
  assert!(call.prompt.contains("Write validation logic"));
  assert!(call.prompt.contains("src/login.rs"));
  assert!(call.prompt.contains("Add email check"));
}

#[test]
fn 低complexityではデフォルトモデルを選択する() {
  let mock = MockClaude::with_json("{}");
  let intent = sample_intent();
  let task = sample_task(&intent);
  let dir = tempfile::tempdir().unwrap();

  implement::run(
    &intent,
    &task,
    &mock,
    "default-model",
    dir.path(),
    None,
    None,
    &SessionMode::new_session(),
  )
  .unwrap();

  let call = mock.last_call();
  assert_eq!(call.model, "default-model");
}

#[test]
fn 高complexityではcomplexモデルを選択する() {
  let mock = MockClaude::with_json("{}");
  let intent = sample_intent();
  let task = sample_task(&intent);
  let dir = tempfile::tempdir().unwrap();

  implement::run(
    &intent,
    &task,
    &mock,
    "complex-model",
    dir.path(),
    None,
    None,
    &SessionMode::new_session(),
  )
  .unwrap();

  let call = mock.last_call();
  assert_eq!(call.model, "complex-model");
}

#[test]
fn リトライ時にレビューフィードバックをプロンプトに含める() {
  let mock = MockClaude::with_json("{}");
  let intent = sample_intent();
  let task = sample_task(&intent);
  let dir = tempfile::tempdir().unwrap();
  let feedback = ReviewResult {
    task_id: "task-1".into(),
    approved: false,
    issues: vec!["Missing error handling".into()],
    suggestions: vec!["Add try-catch block".into()],
    observations: vec![],
    session_id: None,
  };

  implement::run(
    &intent,
    &task,
    &mock,
    "sonnet",
    dir.path(),
    None,
    Some(&feedback),
    &SessionMode::new_session(),
  )
  .unwrap();

  let call = mock.last_call();
  assert!(call.prompt.contains("Previous Review Feedback"));
  assert!(call.prompt.contains("Missing error handling"));
  assert!(call.prompt.contains("Add try-catch block"));
}

#[test]
fn 初回実行時はレビューセクションを省略する() {
  let mock = MockClaude::with_json("{}");
  let intent = sample_intent();
  let task = sample_task(&intent);
  let dir = tempfile::tempdir().unwrap();

  implement::run(
    &intent,
    &task,
    &mock,
    "sonnet",
    dir.path(),
    None,
    None,
    &SessionMode::new_session(),
  )
  .unwrap();

  let call = mock.last_call();
  assert!(!call.prompt.contains("Previous Review Feedback"));
}

#[test]
fn claudeエラーを伝播する() {
  let mock = MockClaude::with_error("process crashed");
  let intent = sample_intent();
  let task = sample_task(&intent);
  let dir = tempfile::tempdir().unwrap();

  let result = implement::run(
    &intent,
    &task,
    &mock,
    "sonnet",
    dir.path(),
    None,
    None,
    &SessionMode::new_session(),
  );
  assert!(result.is_err());
}

#[test]
fn プロンプトにtaskのplan_steps_filesが含まれる() {
  let mock = MockClaude::with_json("{}");
  let intent = sample_intent();
  let task = sample_task(&intent);
  let dir = tempfile::tempdir().unwrap();

  implement::run(
    &intent,
    &task,
    &mock,
    "sonnet",
    dir.path(),
    None,
    None,
    &SessionMode::new_session(),
  )
  .unwrap();

  let call = mock.last_call();
  assert!(call.prompt.contains("**Plan:**"));
  assert!(call.prompt.contains("Write validation logic"));
  assert!(call.prompt.contains("**Relevant files:**"));
  assert!(call.prompt.contains("- src/login.rs"));
  assert!(call.prompt.contains("**Steps:**"));
  assert!(call.prompt.contains("1. Add email check"));
  assert!(call.prompt.contains("2. Add tests"));
  assert!(call.prompt.contains("**Context:**"));
  assert!(call.prompt.contains("Login module context"));
  assert!(call.prompt.contains("**Complexity:** low"));
}
