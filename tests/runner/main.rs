// --- デフォルト Flow ---

#[test]
#[ignore]
fn default_flow_is_analyze_implement_review() {}

#[test]
#[ignore]
fn audit_type_uses_audit_report_flow() {}

// --- Flow 調整ルール ---

#[test]
#[ignore]
fn needs_clarification_pauses_intent() {}

#[test]
#[ignore]
fn depends_on_delays_implement_until_dependency_done() {}

#[test]
#[ignore]
fn rejected_review_retries_implement_review_cycle() {}

#[test]
#[ignore]
fn retry_exhaustion_marks_task_failed() {}

#[test]
#[ignore]
fn all_tasks_done_marks_intent_done() {}

#[test]
#[ignore]
fn partial_task_failure_marks_intent_blocked() {}

#[test]
#[ignore]
fn all_tasks_failed_marks_intent_error() {}

// --- 自動挿入ステップ ---

#[test]
#[ignore]
fn rebase_runs_between_implement_and_review() {}

#[test]
#[ignore]
fn reflect_runs_after_leaf_intent_completion() {}

#[test]
#[ignore]
fn reflect_skipped_for_parent_intent_with_children() {}

// --- コンフリクト解決 ---

#[test]
#[ignore]
fn rebase_failure_triggers_reimplementation() {}

#[test]
#[ignore]
fn reimplementation_failure_escalates_to_human() {}

// --- History 記録 ---

#[test]
#[ignore]
fn records_history_after_intent_completion() {}

#[test]
#[ignore]
fn history_includes_step_results_and_cost() {}

// --- Worktree Setup ---

#[test]
#[ignore]
fn runs_worktree_setup_commands_before_implement() {}
