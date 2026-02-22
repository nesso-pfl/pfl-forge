mod helpers;

// --- デフォルト Flow ---

use pfl_forge::runner::{default_flow, Step};

#[test]
fn default_flow_is_analyze_implement_review() {
  let flow = default_flow(None);
  assert_eq!(flow, vec![Step::Analyze, Step::Implement, Step::Review]);
}

#[test]
fn audit_type_uses_audit_report_flow() {
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
#[ignore]
fn needs_clarification_pauses_intent() {}

#[test]
#[ignore]
fn depends_on_delays_implement_until_dependency_done() {}

// --- 基本実行フロー + 自動挿入ステップ ---

mod basic_flow;

// --- Worktree Setup ---

#[test]
fn runs_worktree_setup_commands_before_implement() {
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
