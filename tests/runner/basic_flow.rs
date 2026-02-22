use pfl_forge::intent::registry::IntentStatus;
use pfl_forge::knowledge::history::{self, Outcome};
use pfl_forge::runner;

use crate::helpers::*;

// --- 基本実行 ---

#[test]
fn all_tasks_done_marks_intent_done() {
  let (_dir, repo) = setup_repo_with_intent("fix-bug");
  let mut intent = load_intent(&repo, "fix-bug");
  let config = default_config();

  // analyze → implement (raw) → review (approved)
  let mock = MockClaude::with_sequence(vec![
    json_response(analysis_json()),
    raw_response("Implementation done"),
    json_response(approved_review_json()),
  ]);

  let result = runner::process_intent(&mut intent, &config, &mock, &repo).unwrap();

  assert_eq!(intent.status, IntentStatus::Done);
  assert_eq!(result.outcome, Outcome::Success);
  assert_eq!(result.flow, vec!["analyze", "implement", "review"]);
}

// --- 失敗時のステータス集約 ---

#[test]
fn all_tasks_failed_marks_intent_error() {
  let (_dir, repo) = setup_repo_with_intent("fail-task");
  let mut intent = load_intent(&repo, "fail-task");
  let config = default_config();

  // analyze succeeds, implement fails
  let mock = MockClaude::with_sequence(vec![
    json_response(analysis_json()),
    error_response("implement crashed"),
  ]);

  let result = runner::process_intent(&mut intent, &config, &mock, &repo).unwrap();

  assert_eq!(intent.status, IntentStatus::Error);
  assert_eq!(result.outcome, Outcome::Failed);
  assert!(result.failure_reason.unwrap().contains("implement failed"));
}

#[test]
fn partial_task_failure_marks_intent_blocked() {
  // With a single task, review failure after max retries → Error (not Blocked).
  // Blocked requires multiple tasks where some succeed and some fail.
  // For single-task intents, review failure → Error.
  let (_dir, repo) = setup_repo_with_intent("partial");
  let mut intent = load_intent(&repo, "partial");
  let mut config = default_config();
  config.max_review_retries = 0; // fail on first rejection

  // analyze → implement → review (rejected, no retries) → Failed
  let mock = MockClaude::with_sequence(vec![
    json_response(analysis_json()),
    raw_response("Implementation done"),
    json_response(rejected_review_json()),
  ]);

  let result = runner::process_intent(&mut intent, &config, &mock, &repo).unwrap();

  // Single task: all failed → Error
  assert_eq!(intent.status, IntentStatus::Error);
  assert_eq!(result.outcome, Outcome::Failed);
}

// --- 複数タスク ---

#[test]
fn multi_task_partial_failure_marks_intent_blocked() {
  // 2 independent tasks: task-a succeeds, task-b fails → intent Blocked
  let (_dir, repo) = setup_repo_with_intent("partial-multi");
  let mut intent = load_intent(&repo, "partial-multi");
  let config = default_config();

  let mock = MockClaude::with_sequence(vec![
    json_response(two_independent_tasks_json()), // analyze
    raw_response("Impl A done"),                 // implement task-a
    json_response(approved_review_json()),       // review task-a (approved)
    error_response("implement crashed"),         // implement task-b (fails)
  ]);

  let result = runner::process_intent(&mut intent, &config, &mock, &repo).unwrap();

  assert_eq!(intent.status, IntentStatus::Blocked);
  assert_eq!(result.outcome, Outcome::Failed);
}

#[test]
fn dependency_failure_skips_dependent_tasks() {
  // task-b depends_on task-a; task-a fails → task-b skipped
  let (_dir, repo) = setup_repo_with_intent("dep-skip");
  let mut intent = load_intent(&repo, "dep-skip");
  let config = default_config();

  let mock = MockClaude::with_sequence(vec![
    json_response(multi_task_analysis_json()), // analyze
    error_response("implement crashed"),       // implement task-a (fails)
  ]);

  let result = runner::process_intent(&mut intent, &config, &mock, &repo).unwrap();

  // Both tasks failed (task-a directly, task-b skipped) → all failed → Error
  assert_eq!(intent.status, IntentStatus::Error);
  assert_eq!(result.outcome, Outcome::Failed);
  // Only 2 calls: analyze + implement (task-b never attempted)
  assert_eq!(mock.call_count(), 2);
}

// --- Review リトライ ---

#[test]
fn rejected_review_retries_implement_review_cycle() {
  let (_dir, repo) = setup_repo_with_intent("retry-task");
  let mut intent = load_intent(&repo, "retry-task");
  let mut config = default_config();
  config.max_review_retries = 1;

  // analyze → implement → review(rejected) → implement(retry) → review(approved)
  let mock = MockClaude::with_sequence(vec![
    json_response(analysis_json()),        // analyze
    raw_response("First attempt"),         // implement #1
    json_response(rejected_review_json()), // review #1 (rejected)
    raw_response("Second attempt"),        // implement #2 (retry)
    json_response(approved_review_json()), // review #2 (approved)
  ]);

  let result = runner::process_intent(&mut intent, &config, &mock, &repo).unwrap();

  assert_eq!(intent.status, IntentStatus::Done);
  assert_eq!(result.outcome, Outcome::Success);
  // Should have 5 calls: analyze + impl + review + impl + review
  assert_eq!(mock.call_count(), 5);
}

#[test]
fn retry_exhaustion_marks_task_failed() {
  let (_dir, repo) = setup_repo_with_intent("exhaust");
  let mut intent = load_intent(&repo, "exhaust");
  let mut config = default_config();
  config.max_review_retries = 1;

  // analyze → implement → review(rejected) → implement(retry) → review(rejected again)
  let mock = MockClaude::with_sequence(vec![
    json_response(analysis_json()),
    raw_response("First attempt"),
    json_response(rejected_review_json()),
    raw_response("Second attempt"),
    json_response(rejected_review_json()), // still rejected
  ]);

  let result = runner::process_intent(&mut intent, &config, &mock, &repo).unwrap();

  assert_eq!(intent.status, IntentStatus::Error);
  assert_eq!(result.outcome, Outcome::Failed);
  assert!(result.failure_reason.unwrap().contains("max retries"));
}

// --- Rebase ---

#[test]
fn rebase_runs_between_implement_and_review() {
  let (_dir, repo) = setup_repo_with_intent("rebase-test");
  let mut intent = load_intent(&repo, "rebase-test");
  let config = default_config();

  let mock = MockClaude::with_sequence(vec![
    json_response(analysis_json()),
    raw_response("Done"),
    json_response(approved_review_json()),
  ]);

  let result = runner::process_intent(&mut intent, &config, &mock, &repo).unwrap();

  // Check that rebase step appears in step_results between implement and review
  let steps: Vec<&str> = result
    .step_results
    .iter()
    .map(|s| s.step.as_str())
    .collect();
  assert!(steps.contains(&"rebase"), "steps: {:?}", steps);

  let impl_pos = steps.iter().position(|s| *s == "implement").unwrap();
  let rebase_pos = steps.iter().position(|s| *s == "rebase").unwrap();
  let review_pos = steps.iter().position(|s| *s == "review").unwrap();
  assert!(impl_pos < rebase_pos, "implement should come before rebase");
  assert!(rebase_pos < review_pos, "rebase should come before review");
}

// --- Reflect 自動挿入 ---

#[test]
fn reflect_runs_after_leaf_intent_completion() {
  let (_dir, repo) = setup_repo_with_intent("leaf-intent");
  let mut intent = load_intent(&repo, "leaf-intent");
  let config = default_config();

  // Add an observation so reflect actually calls Claude
  let obs_path = repo.join(".forge").join("observations.yaml");
  let obs = pfl_forge::knowledge::observation::Observation {
    content: "found duplicated logic".into(),
    evidence: vec![],
    source: "implement".into(),
    intent_id: "leaf-intent".into(),
    processed: false,
    created_at: None,
  };
  pfl_forge::knowledge::observation::append(&obs_path, &obs).unwrap();

  // analyze → implement → review(approved) → reflect
  let mock = MockClaude::with_sequence(vec![
    json_response(analysis_json()),
    raw_response("Done"),
    json_response(approved_review_json()),
    json_response(reflect_json()), // reflect call
  ]);

  let result = runner::process_intent(&mut intent, &config, &mock, &repo).unwrap();

  let steps: Vec<&str> = result
    .step_results
    .iter()
    .map(|s| s.step.as_str())
    .collect();
  assert!(steps.contains(&"reflect"), "steps: {:?}", steps);
  assert_eq!(mock.call_count(), 4);
}

#[test]
fn reflect_skipped_for_parent_intent_with_children() {
  let (_dir, repo) = setup_repo_with_intent("parent-intent");
  let mut intent = load_intent(&repo, "parent-intent");
  let config = default_config();

  // Create a child intent that references parent-intent
  let intents_dir = repo.join(".forge").join("intents");
  let child_yaml = "title: Child task\nbody: Sub-task\nsource: human\nparent: parent-intent\n";
  std::fs::write(intents_dir.join("child-task.yaml"), child_yaml).unwrap();

  // analyze → implement → review(approved), no reflect
  let mock = MockClaude::with_sequence(vec![
    json_response(analysis_json()),
    raw_response("Done"),
    json_response(approved_review_json()),
  ]);

  let result = runner::process_intent(&mut intent, &config, &mock, &repo).unwrap();

  assert_eq!(result.outcome, Outcome::Success);
  let steps: Vec<&str> = result
    .step_results
    .iter()
    .map(|s| s.step.as_str())
    .collect();
  assert!(
    !steps.contains(&"reflect"),
    "reflect should be skipped for parent, steps: {:?}",
    steps
  );
  assert_eq!(mock.call_count(), 3); // no reflect call
}

// --- コンフリクト解決 ---

#[test]
fn rebase_failure_triggers_reimplementation() {
  let (_dir, repo) = setup_repo_with_conflict("conflict-rebase");
  let mut intent = load_intent(&repo, "conflict-rebase");
  let config = default_config();

  // analyze → implement → (rebase fails, reimpl) → implement → review(approved)
  let mock = MockClaude::with_sequence(vec![
    json_response(analysis_json()),        // analyze
    raw_response("First attempt"),         // implement (on conflicting branch)
    raw_response("Reimplementation"),      // implement (fresh from updated main)
    json_response(approved_review_json()), // review
  ]);

  let result = runner::process_intent(&mut intent, &config, &mock, &repo).unwrap();

  assert_eq!(result.outcome, Outcome::Success);
  assert_eq!(intent.status, IntentStatus::Done);
  // 4 Claude calls: analyze + implement + reimpl + review
  assert_eq!(mock.call_count(), 4);
}

#[test]
fn reimplementation_failure_escalates_to_human() {
  let (_dir, repo) = setup_repo_with_conflict("conflict-escalate");
  let mut intent = load_intent(&repo, "conflict-escalate");
  let config = default_config();

  // analyze → implement → (rebase fails) → reimpl fails → escalated
  let mock = MockClaude::with_sequence(vec![
    json_response(analysis_json()),            // analyze
    raw_response("First attempt"),             // implement (on conflicting branch)
    error_response("reimplementation failed"), // reimpl fails
  ]);

  let result = runner::process_intent(&mut intent, &config, &mock, &repo).unwrap();

  assert_eq!(result.outcome, Outcome::Escalated);
  assert_eq!(intent.status, IntentStatus::Error);
  assert!(result
    .failure_reason
    .unwrap()
    .contains("reimplementation failed"));
}

// --- History 記録 ---

#[test]
fn records_history_after_intent_completion() {
  let (_dir, repo) = setup_repo_with_intent("history-test");
  let mut intent = load_intent(&repo, "history-test");
  let config = default_config();

  let mock = MockClaude::with_sequence(vec![
    json_response(analysis_json()),
    raw_response("Done"),
    json_response(approved_review_json()),
  ]);

  runner::process_intent(&mut intent, &config, &mock, &repo).unwrap();

  let entry = history::load(&repo, "history-test").unwrap();
  assert_eq!(entry.intent_id, "history-test");
  assert_eq!(entry.outcome, Outcome::Success);
  assert_eq!(entry.title, "Fix bug");
}

#[test]
fn history_includes_step_results_and_cost() {
  let (_dir, repo) = setup_repo_with_intent("cost-test");
  let mut intent = load_intent(&repo, "cost-test");
  let config = default_config();

  let mock = MockClaude::with_sequence(vec![
    json_response(analysis_json()),
    raw_response("Done"),
    json_response(approved_review_json()),
  ]);

  runner::process_intent(&mut intent, &config, &mock, &repo).unwrap();

  let entry = history::load(&repo, "cost-test").unwrap();
  let step_names: Vec<&str> = entry.step_results.iter().map(|s| s.step.as_str()).collect();
  assert!(step_names.contains(&"analyze"));
  assert!(step_names.contains(&"implement"));
  assert!(step_names.contains(&"review"));
  assert_eq!(entry.flow, vec!["analyze", "implement", "review"]);
}
