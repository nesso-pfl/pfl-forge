use pfl_forge::intent::registry::IntentStatus;
use pfl_forge::knowledge::history::{self, Outcome};
use pfl_forge::runner;

use crate::helpers::*;

// --- run_intents ---

#[test]
fn approvedのintentのみ処理する() {
  let (_dir, repo) = setup_repo_with_intent("target");
  add_intent(&repo, "proposed-one", "proposed");
  add_intent(&repo, "done-one", "done");
  let config = default_config();

  let mock = MockClaude::with_sequence(vec![
    json_response(analysis_json()),
    raw_response("Done"),
    json_response(approved_review_json()),
  ]);

  let results = runner::run_intents(&config, &mock, &repo, false).unwrap();

  assert_eq!(results.len(), 1);
  assert_eq!(results[0].0, "target");
  assert_eq!(results[0].1.outcome, Outcome::Success);
  // 3 calls: analyze + implement + review (only for the approved intent)
  assert_eq!(mock.call_count(), 3);
}

#[test]
fn approved_intentがなければ空を返す() {
  let (_dir, repo) = setup_repo_with_intent("some-intent");
  // Change the existing intent to proposed
  add_intent(&repo, "some-intent", "proposed");
  let config = default_config();

  let mock = MockClaude::with_sequence(vec![]);

  let results = runner::run_intents(&config, &mock, &repo, false).unwrap();

  assert!(results.is_empty());
  assert_eq!(mock.call_count(), 0);
}

#[test]
fn dry_runではanalyzeを実行しない() {
  let (_dir, repo) = setup_repo_with_intent("dry-target");
  let config = default_config();

  let mock = MockClaude::with_sequence(vec![]);

  let results = runner::run_intents(&config, &mock, &repo, true).unwrap();

  assert!(results.is_empty());
  assert_eq!(mock.call_count(), 0);
}

#[test]
fn 複数intentを順次処理する() {
  let (_dir, repo) = setup_repo_with_intent("first");
  add_intent(&repo, "second", "approved");
  let mut config = default_config();
  config.parallel_workers = 1; // Sequential: mock responses depend on order

  let mock = MockClaude::with_sequence(vec![
    // first intent
    json_response(analysis_json()),
    raw_response("Done 1"),
    json_response(approved_review_json()),
    // second intent
    json_response(analysis_json()),
    raw_response("Done 2"),
    json_response(approved_review_json()),
  ]);

  let results = runner::run_intents(&config, &mock, &repo, false).unwrap();

  assert_eq!(results.len(), 2);
  assert_eq!(results[0].1.outcome, Outcome::Success);
  assert_eq!(results[1].1.outcome, Outcome::Success);
  assert_eq!(mock.call_count(), 6);
}

// --- 基本実行 ---

#[test]
fn 全タスク成功でintentがdoneになる() {
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
fn 全タスク失敗でintentがerrorになる() {
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
fn 単一タスクのレビュー失敗でintentがerrorになる() {
  // With a single task, review failure after max retries → Error (not Blocked).
  // Blocked requires multiple tasks where some succeed and some fail.
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
fn 複数タスクで一部失敗するとintentがblockedになる() {
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
fn 依存先の失敗で依存タスクをスキップする() {
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
fn レビュー却下時にimplement_reviewサイクルをリトライする() {
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
fn リトライ上限でタスクが失敗する() {
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
fn rebaseがimplementとreviewの間に実行される() {
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
  assert!(impl_pos < rebase_pos);
  assert!(rebase_pos < review_pos);
}

// --- Reflect 自動挿入 ---

#[test]
fn リーフintent完了後にreflectが実行される() {
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
    source_session_id: None,
    processed_session_id: None,
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
fn 子intentを持つ親intentではreflectをスキップする() {
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
  assert!(!steps.contains(&"reflect"), "steps: {:?}", steps);
  assert_eq!(mock.call_count(), 3);
}

// --- コンフリクト解決 ---

#[test]
fn rebase失敗時に再実装する() {
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
fn 再実装失敗時にエスカレートする() {
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
fn intent完了後にhistoryを記録する() {
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
fn historyにstep_resultsが含まれる() {
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

// --- Resume ---

#[test]
fn implementing_intentを再開する() {
  // last_step=analyze + worktree with tasks.yaml → skip analyze
  let (_dir, repo) = setup_repo_with_intent("resume-target");
  let config = default_config();

  // Overwrite the intent as implementing with last_step=analyze
  add_implementing_intent(&repo, "resume-target", Some("analyze"), None);

  // Create worktree with tasks.yaml
  setup_worktree_with_tasks(&repo, &config, "resume-target");

  // Only implement + review needed (no analyze)
  let mock = MockClaude::with_sequence(vec![
    raw_response("Implementation done"),
    json_response(approved_review_json()),
  ]);

  let mut intent = load_intent(&repo, "resume-target");
  let result = runner::process_intent(&mut intent, &config, &mock, &repo).unwrap();

  assert_eq!(result.outcome, Outcome::Success);
  assert_eq!(intent.status, IntentStatus::Done);
  // Only 2 calls: implement + review (analyze skipped)
  assert_eq!(mock.call_count(), 2);
  let steps: Vec<&str> = result
    .step_results
    .iter()
    .map(|s| s.step.as_str())
    .collect();
  assert!(!steps.contains(&"analyze"));
}

#[test]
fn worktreeがなければ最初からやり直す() {
  // last_step=analyze but no worktree → run from start
  let (_dir, repo) = setup_repo_with_intent("resume-no-wt");
  add_implementing_intent(&repo, "resume-no-wt", Some("analyze"), None);
  let config = default_config();

  // analyze + implement + review
  let mock = MockClaude::with_sequence(vec![
    json_response(analysis_json()),
    raw_response("Done"),
    json_response(approved_review_json()),
  ]);

  let mut intent = load_intent(&repo, "resume-no-wt");
  let result = runner::process_intent(&mut intent, &config, &mock, &repo).unwrap();

  assert_eq!(result.outcome, Outcome::Success);
  // 3 calls: analyze + implement + review
  assert_eq!(mock.call_count(), 3);
}

#[test]
fn approvedとimplementingの両方を処理する() {
  let (_dir, repo) = setup_repo_with_intent("approved-one");
  add_implementing_intent(&repo, "impl-one", Some("analyze"), None);
  let mut config = default_config();
  config.parallel_workers = 1; // Sequential: mock responses depend on order

  // Create worktree for the implementing intent
  setup_worktree_with_tasks(&repo, &config, "impl-one");

  let mock = MockClaude::with_sequence(vec![
    // approved-one: analyze + implement + review
    json_response(analysis_json()),
    raw_response("Done 1"),
    json_response(approved_review_json()),
    // impl-one: implement + review (resume, analyze skipped)
    raw_response("Done 2"),
    json_response(approved_review_json()),
  ]);

  let results = runner::run_intents(&config, &mock, &repo, false).unwrap();

  assert_eq!(results.len(), 2);
  assert!(results.iter().all(|(_, r)| r.outcome == Outcome::Success));
  // 5 total calls
  assert_eq!(mock.call_count(), 5);
}

// --- SessionMode ---

#[test]
fn 新規実行では全エージェントにnewセッションを渡す() {
  let (_dir, repo) = setup_repo_with_intent("session-new");
  let config = default_config();

  let mock = MockClaude::with_sequence(vec![
    json_response(analysis_json()),
    raw_response("Done"),
    json_response(approved_review_json()),
  ]);

  let mut intent = load_intent(&repo, "session-new");
  runner::process_intent(&mut intent, &config, &mock, &repo).unwrap();

  let calls = mock.captured_calls();
  // analyze, implement, review = 3 calls minimum
  assert!(calls.len() >= 3);
  for call in &calls {
    match &call.session {
      CapturedSession::New(id) => assert!(!id.is_empty()),
      other => panic!("expected New session, got {:?}", other),
    }
  }
}

#[test]
fn resume時にimplementにresumeセッションを渡す() {
  let (_dir, repo) = setup_repo_with_intent("session-resume");
  let config = default_config();

  let prev_session = "prev-implement-session-id";
  add_implementing_intent(
    &repo,
    "session-resume",
    Some("analyze"),
    Some(ImplementingIntentOptions {
      analyze_session: None,
      implement_session: Some(prev_session.to_string()),
    }),
  );
  setup_worktree_with_tasks(&repo, &config, "session-resume");

  let mock = MockClaude::with_sequence(vec![
    raw_response("Done"),
    json_response(approved_review_json()),
  ]);

  let mut intent = load_intent(&repo, "session-resume");
  runner::process_intent(&mut intent, &config, &mock, &repo).unwrap();

  let calls = mock.captured_calls();
  // First call (implement) should use Resume with the saved session
  assert_eq!(
    calls[0].session,
    CapturedSession::Resume(prev_session.into())
  );
  // Second call (review) should use a new session
  match &calls[1].session {
    CapturedSession::New(id) => assert!(!id.is_empty()),
    other => panic!("expected New session for review, got {:?}", other),
  }
}

#[test]
fn session_idがintent_yamlにspawn前に書き込まれる() {
  let (_dir, repo) = setup_repo_with_intent("session-write");
  let config = default_config();

  let mock = MockClaude::with_sequence(vec![
    json_response(analysis_json()),
    raw_response("Done"),
    json_response(approved_review_json()),
  ]);

  let mut intent = load_intent(&repo, "session-write");
  runner::process_intent(&mut intent, &config, &mock, &repo).unwrap();

  // After processing, all session fields should be populated
  assert!(
    intent.sessions.analyze.is_some(),
    "analyze session should be set"
  );
  assert!(
    intent.sessions.implement.is_some(),
    "implement session should be set"
  );
  assert!(
    intent.sessions.review.is_some(),
    "review session should be set"
  );

  // Verify the sessions match what was passed to the mock
  let calls = mock.captured_calls();
  let analyze_sid = match &calls[0].session {
    CapturedSession::New(id) => id.clone(),
    other => panic!("expected New, got {:?}", other),
  };
  assert_eq!(
    intent.sessions.analyze.as_deref(),
    Some(analyze_sid.as_str())
  );
}
