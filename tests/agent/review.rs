use std::process::Command;

use pfl_forge::agent::review;
use pfl_forge::config::Config;
use pfl_forge::intent::registry::Intent;
use pfl_forge::task::Task;

use crate::mock_claude::MockClaude;

fn default_config() -> Config {
  serde_yaml::from_str("{}").unwrap()
}

fn sample_intent() -> Intent {
  let dir = tempfile::tempdir().unwrap();
  let yaml = "title: Fix bug\nbody: Fix validation\nsource: human\n";
  std::fs::write(dir.path().join("fix-bug.yaml"), yaml).unwrap();
  let intents = Intent::fetch_all(dir.path()).unwrap();
  std::mem::forget(dir);
  intents.into_iter().next().unwrap()
}

fn sample_task() -> Task {
  let intent: Intent = serde_yaml::from_str("title: t\nbody: b\nsource: human\n").unwrap();
  let analysis = pfl_forge::agent::analyze::AnalysisResult {
    complexity: "low".into(),
    plan: "The implementation plan".into(),
    relevant_files: vec!["src/lib.rs".into()],
    implementation_steps: vec!["step 1".into()],
    context: "context".into(),
  };
  Task::from_analysis(&intent, &analysis)
}

/// Set up a temp git repo with origin/main ref and a diff
fn setup_git_repo() -> tempfile::TempDir {
  let dir = tempfile::tempdir().unwrap();
  let p = dir.path();

  let run = |args: &[&str]| {
    Command::new("git")
      .args(args)
      .current_dir(p)
      .env("GIT_AUTHOR_NAME", "test")
      .env("GIT_AUTHOR_EMAIL", "test@test.com")
      .env("GIT_COMMITTER_NAME", "test")
      .env("GIT_COMMITTER_EMAIL", "test@test.com")
      .output()
      .expect("git failed")
  };

  run(&["init", "-b", "main"]);
  std::fs::write(p.join("file.txt"), "original\n").unwrap();
  run(&["add", "."]);
  run(&["commit", "-m", "initial"]);
  // Create fake origin/main ref
  run(&["update-ref", "refs/remotes/origin/main", "HEAD"]);
  // Make a change
  std::fs::write(p.join("file.txt"), "modified\n").unwrap();
  run(&["add", "."]);
  run(&["commit", "-m", "change"]);

  dir
}

#[test]
fn 承認されたレビュー結果を返す() {
  let json = r#"{"approved":true,"issues":[],"suggestions":[]}"#;
  let mock = MockClaude::with_json(json);
  let config = default_config();
  let intent = sample_intent();
  let task = sample_task();
  let repo = setup_git_repo();

  let (result, _meta) =
    review::review(&intent, &task, &config, &mock, repo.path(), "main").unwrap();
  assert!(result.approved);
  assert!(result.issues.is_empty());
}

#[test]
fn 却下時にissueを返す() {
  let json = r#"{"approved":false,"issues":["Missing tests"],"suggestions":["Add unit tests"]}"#;
  let mock = MockClaude::with_json(json);
  let config = default_config();
  let intent = sample_intent();
  let task = sample_task();
  let repo = setup_git_repo();

  let (result, _meta) =
    review::review(&intent, &task, &config, &mock, repo.path(), "main").unwrap();
  assert!(!result.approved);
  assert_eq!(result.issues, vec!["Missing tests"]);
  assert_eq!(result.suggestions, vec!["Add unit tests"]);
}

#[test]
fn プロンプトにdiffとplanを含める() {
  let json = r#"{"approved":true,"issues":[],"suggestions":[]}"#;
  let mock = MockClaude::with_json(json);
  let config = default_config();
  let intent = sample_intent();
  let task = sample_task();
  let repo = setup_git_repo();

  review::review(&intent, &task, &config, &mock, repo.path(), "main").unwrap();

  let call = mock.last_call();
  assert!(call.prompt.contains("The implementation plan"));
  assert!(call.prompt.contains("modified"));
}

#[test]
fn configのデフォルトモデルを使用する() {
  let json = r#"{"approved":true,"issues":[],"suggestions":[]}"#;
  let mock = MockClaude::with_json(json);
  let config = default_config();
  let intent = sample_intent();
  let task = sample_task();
  let repo = setup_git_repo();

  review::review(&intent, &task, &config, &mock, repo.path(), "main").unwrap();

  let call = mock.last_call();
  assert_eq!(call.model, pfl_forge::claude::model::SONNET);
}

#[test]
fn 大きなdiffを切り詰める() {
  let json = r#"{"approved":true,"issues":[],"suggestions":[]}"#;
  let mock = MockClaude::with_json(json);
  let config = default_config();
  let intent = sample_intent();
  let task = sample_task();

  // Create a repo with a >50KB diff
  let dir = tempfile::tempdir().unwrap();
  let p = dir.path();
  let run = |args: &[&str]| {
    Command::new("git")
      .args(args)
      .current_dir(p)
      .env("GIT_AUTHOR_NAME", "test")
      .env("GIT_AUTHOR_EMAIL", "test@test.com")
      .env("GIT_COMMITTER_NAME", "test")
      .env("GIT_COMMITTER_EMAIL", "test@test.com")
      .output()
      .expect("git failed")
  };
  run(&["init", "-b", "main"]);
  std::fs::write(p.join("big.txt"), "a\n").unwrap();
  run(&["add", "."]);
  run(&["commit", "-m", "initial"]);
  run(&["update-ref", "refs/remotes/origin/main", "HEAD"]);
  // Write >50KB of content
  let large_content = "x".repeat(60_000);
  std::fs::write(p.join("big.txt"), &large_content).unwrap();
  run(&["add", "."]);
  run(&["commit", "-m", "large change"]);

  review::review(&intent, &task, &config, &mock, p, "main").unwrap();

  let call = mock.last_call();
  // The prompt should be truncated: diff portion ≤ 50000 chars
  assert!(
    call.prompt.len() < 60_000,
    "prompt should be truncated, got {} bytes",
    call.prompt.len()
  );
}

#[test]
fn claudeエラーを伝播する() {
  let mock = MockClaude::with_error("API error");
  let config = default_config();
  let intent = sample_intent();
  let task = sample_task();
  let repo = setup_git_repo();

  let result = review::review(&intent, &task, &config, &mock, repo.path(), "main");
  assert!(result.is_err());
}
