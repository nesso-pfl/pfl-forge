mod helpers;

// --- デフォルト Flow ---

use pfl_forge::runner::{default_flow, Step};

#[test]
fn デフォルトflowはanalyze_implement_review() {
  let flow = default_flow(None);
  assert_eq!(flow, vec![Step::Analyze, Step::Implement, Step::Review]);
}

#[test]
fn skill_extraction種別はobserve_abstract_record() {
  let flow = default_flow(Some("skill_extraction"));
  assert_eq!(flow, vec![Step::Observe, Step::Abstract, Step::Record]);
}

#[test]
fn audit種別はaudit_reportフローを使う() {
  use helpers::*;
  use pfl_forge::intent::registry::IntentStatus;
  use pfl_forge::knowledge::history::Outcome;
  use pfl_forge::runner;

  let (_dir, repo) = setup_repo_with_audit_intent("audit-test");
  let mut intent = load_intent(&repo, "audit-test");
  let config = default_config();

  let mock = MockClaude::with_sequence(vec![json_response(audit_result_json())]);

  let result = runner::process_intent(&mut intent, &config, &mock, &repo).unwrap();

  assert_eq!(result.flow, vec!["audit", "report"]);
  assert_eq!(result.outcome, Outcome::Success);
  assert_eq!(intent.status, IntentStatus::Done);

  let steps: Vec<&str> = result
    .step_results
    .iter()
    .map(|s| s.step.as_str())
    .collect();
  assert_eq!(steps, vec!["audit", "report"]);
  assert_eq!(mock.call_count(), 1);
}

// --- Flow 調整ルール ---

#[test]
fn clarificationが必要な場合はintentを一時停止する() {
  use helpers::*;
  use pfl_forge::intent::registry::IntentStatus;
  use pfl_forge::knowledge::history::Outcome;
  use pfl_forge::runner;

  let (_dir, repo) = setup_repo_with_intent("clarify-test");
  let mut intent = load_intent(&repo, "clarify-test");
  let config = default_config();

  let clarification_json =
    r#"{"outcome":"needs_clarification","clarifications":["What API version?"]}"#;
  let mock = MockClaude::with_sequence(vec![json_response(clarification_json)]);

  let result = runner::process_intent(&mut intent, &config, &mock, &repo).unwrap();

  assert_eq!(intent.status, IntentStatus::Blocked);
  assert_eq!(result.outcome, Outcome::Failed);
  assert!(result.failure_reason.unwrap().contains("clarification"));
  assert!(intent.needs_clarification());
  assert_eq!(intent.clarifications.len(), 1);
  assert_eq!(intent.clarifications[0].question, "What API version?");
  assert!(intent.clarifications[0].answer.is_none());
  // session_id and last_step saved for resume
  assert_eq!(intent.last_step.as_deref(), Some("analyze"));
  assert!(intent.sessions.analyze.is_some());
}

#[test]
fn depends_onで依存タスク完了までimplementを遅延する() {
  use helpers::*;
  use pfl_forge::intent::registry::IntentStatus;
  use pfl_forge::knowledge::history::Outcome;
  use pfl_forge::runner;

  let (_dir, repo) = setup_repo_with_intent("dep-test");
  let mut intent = load_intent(&repo, "dep-test");
  let config = default_config();

  // analyze returns 2 tasks: task-b depends_on task-a
  // For each task: implement + rebase + review
  let mock = MockClaude::with_sequence(vec![
    json_response(multi_task_analysis_json()), // analyze
    raw_response("Impl A done"),               // implement task-a
    json_response(approved_review_json()),     // review task-a
    raw_response("Impl B done"),               // implement task-b
    json_response(approved_review_json()),     // review task-b
  ]);

  let result = runner::process_intent(&mut intent, &config, &mock, &repo).unwrap();

  assert_eq!(intent.status, IntentStatus::Done);
  assert_eq!(result.outcome, Outcome::Success);
  // 5 calls: analyze + (impl+review)*2
  assert_eq!(mock.call_count(), 5);
}

#[test]
fn skill_extraction種別はobserve_abstract_recordフローを使う() {
  use helpers::*;
  use pfl_forge::intent::registry::IntentStatus;
  use pfl_forge::knowledge::history::Outcome;
  use pfl_forge::runner;

  let (_dir, repo) = setup_repo_with_skill_intent("skill-test");
  let mut intent = load_intent(&repo, "skill-test");
  let config = default_config();

  let mock = MockClaude::with_sequence(vec![
    json_response(observe_result_json()),
    json_response(abstract_result_json()),
  ]);

  let result = runner::process_intent(&mut intent, &config, &mock, &repo).unwrap();

  assert_eq!(result.flow, vec!["observe", "abstract", "record"]);
  assert_eq!(result.outcome, Outcome::Success);
  assert_eq!(intent.status, IntentStatus::Done);

  let steps: Vec<&str> = result
    .step_results
    .iter()
    .map(|s| s.step.as_str())
    .collect();
  assert_eq!(steps, vec!["observe", "abstract", "record"]);
  assert_eq!(mock.call_count(), 2); // observe + abstract (record is non-agent)

  // Verify skill file was written
  let skill_path = repo
    .join(".claude")
    .join("skills")
    .join("test-driven")
    .join("SKILL.md");
  assert!(skill_path.exists());
}

#[test]
fn skill_extractionでパターンなしなら早期終了する() {
  use helpers::*;
  use pfl_forge::intent::registry::IntentStatus;
  use pfl_forge::knowledge::history::Outcome;
  use pfl_forge::runner;

  let (_dir, repo) = setup_repo_with_skill_intent("skill-empty");
  let mut intent = load_intent(&repo, "skill-empty");
  let config = default_config();

  let mock = MockClaude::with_sequence(vec![json_response(r#"{"patterns":[]}"#)]);

  let result = runner::process_intent(&mut intent, &config, &mock, &repo).unwrap();

  assert_eq!(result.outcome, Outcome::Success);
  assert_eq!(intent.status, IntentStatus::Done);
  // Only observe step (no abstract/record since no patterns)
  let steps: Vec<&str> = result
    .step_results
    .iter()
    .map(|s| s.step.as_str())
    .collect();
  assert_eq!(steps, vec!["observe"]);
}

// --- Cross-intent depends_on ---

#[test]
fn cross_intent依存が未完了ならintentをスキップする() {
  use helpers::*;
  use pfl_forge::runner;

  let (_dir, repo) = setup_repo_with_intent("base-intent");
  // Add a second intent that depends on a non-done intent
  add_intent_with_depends_on(&repo, "dependent", "approved", &["base-intent"]);
  let config = default_config();

  // Mock: only base-intent should be processed (analyze + implement + review)
  let mock = MockClaude::with_sequence(vec![
    json_response(analysis_json()),
    raw_response("Done"),
    json_response(approved_review_json()),
  ]);

  let results = runner::run_intents(&config, &mock, &repo, false).unwrap();

  // Only base-intent was processed; dependent was skipped because base-intent wasn't done yet
  // at the time of filtering
  let processed_ids: Vec<&str> = results.iter().map(|(id, _)| id.as_str()).collect();
  assert!(
    !processed_ids.contains(&"dependent"),
    "dependent intent should be skipped when dependency is not done"
  );
}

#[test]
fn cross_intent依存が完了済みならintentを処理する() {
  use helpers::*;
  use pfl_forge::runner;

  let (_dir, repo) = setup_repo_with_intent("dep-done");
  // Mark the dependency as done
  add_intent(&repo, "prereq", "done");
  // Add intent that depends on the done prereq
  add_intent_with_depends_on(&repo, "dep-done", "approved", &["prereq"]);

  let config = default_config();

  let mock = MockClaude::with_sequence(vec![
    json_response(analysis_json()),
    raw_response("Done"),
    json_response(approved_review_json()),
  ]);

  let results = runner::run_intents(&config, &mock, &repo, false).unwrap();

  let processed_ids: Vec<&str> = results.iter().map(|(id, _)| id.as_str()).collect();
  assert!(
    processed_ids.contains(&"dep-done"),
    "intent with satisfied dependencies should be processed"
  );
}

// --- 基本実行フロー + 自動挿入ステップ ---

mod basic_flow;

// --- Worktree Setup ---

#[test]
fn implement前にworktreeセットアップコマンドを実行する() {
  use helpers::*;
  use pfl_forge::runner;

  let (_dir, repo) = setup_repo_with_intent("setup-test");
  let mut intent = load_intent(&repo, "setup-test");
  let mut config = default_config();
  config.worktree_setup = vec!["touch setup_marker.txt".to_string()];

  let mock = MockClaude::with_sequence(vec![
    json_response(analysis_json()),
    raw_response("Done"),
    json_response(approved_review_json()),
  ]);

  let result = runner::process_intent(&mut intent, &config, &mock, &repo).unwrap();
  assert_eq!(
    result.outcome,
    pfl_forge::knowledge::history::Outcome::Success
  );

  // Verify setup command ran in the worktree
  let worktree_path = repo
    .join(&config.worktree_dir)
    .join("forge")
    .join("setup-test");
  assert!(
    worktree_path.join("setup_marker.txt").exists(),
    "worktree setup command should have created marker file"
  );
}
