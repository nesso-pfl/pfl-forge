use pfl_forge::agent::implement;
use pfl_forge::agent::review::ReviewResult;
use pfl_forge::intent::registry::Intent;

use crate::mock_claude::MockClaude;

fn sample_intent() -> Intent {
  let dir = tempfile::tempdir().unwrap();
  let yaml = "title: Fix bug\nbody: Fix the login validation bug\nsource: human\n";
  std::fs::write(dir.path().join("fix-bug.yaml"), yaml).unwrap();
  let intents = Intent::fetch_all(dir.path()).unwrap();
  std::mem::forget(dir);
  intents.into_iter().next().unwrap()
}

#[test]
fn intentコンテキストで実装を実行する() {
  let mock = MockClaude::with_json("{}");
  let intent = sample_intent();
  let dir = tempfile::tempdir().unwrap();

  implement::run(&intent, &mock, "sonnet", dir.path(), None, None).unwrap();

  let call = mock.last_call();
  assert!(call.prompt.contains("fix-bug"));
  assert!(call.prompt.contains("Fix bug"));
  assert!(call.prompt.contains("Fix the login validation bug"));
}

#[test]
fn 低complexityではデフォルトモデルを選択する() {
  let mock = MockClaude::with_json("{}");
  let intent = sample_intent();
  let dir = tempfile::tempdir().unwrap();

  implement::run(&intent, &mock, "default-model", dir.path(), None, None).unwrap();

  let call = mock.last_call();
  assert_eq!(call.model, "default-model");
}

#[test]
fn 高complexityではcomplexモデルを選択する() {
  let mock = MockClaude::with_json("{}");
  let intent = sample_intent();
  let dir = tempfile::tempdir().unwrap();

  implement::run(&intent, &mock, "complex-model", dir.path(), None, None).unwrap();

  let call = mock.last_call();
  assert_eq!(call.model, "complex-model");
}

#[test]
fn リトライ時にレビューフィードバックをプロンプトに含める() {
  let mock = MockClaude::with_json("{}");
  let intent = sample_intent();
  let dir = tempfile::tempdir().unwrap();
  let feedback = ReviewResult {
    approved: false,
    issues: vec!["Missing error handling".into()],
    suggestions: vec!["Add try-catch block".into()],
  };

  implement::run(&intent, &mock, "sonnet", dir.path(), None, Some(&feedback)).unwrap();

  let call = mock.last_call();
  assert!(call.prompt.contains("Previous Review Feedback"));
  assert!(call.prompt.contains("Missing error handling"));
  assert!(call.prompt.contains("Add try-catch block"));
}

#[test]
fn 初回実行時はレビューセクションを省略する() {
  let mock = MockClaude::with_json("{}");
  let intent = sample_intent();
  let dir = tempfile::tempdir().unwrap();

  implement::run(&intent, &mock, "sonnet", dir.path(), None, None).unwrap();

  let call = mock.last_call();
  assert!(!call.prompt.contains("Previous Review Feedback"));
}

#[test]
fn claudeエラーを伝播する() {
  let mock = MockClaude::with_error("process crashed");
  let intent = sample_intent();
  let dir = tempfile::tempdir().unwrap();

  let result = implement::run(&intent, &mock, "sonnet", dir.path(), None, None);
  assert!(result.is_err());
}
