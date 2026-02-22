mod helpers;

// --- デフォルト Flow ---

use pfl_forge::runner::{default_flow, Step};

#[test]
fn default_flow_is_analyze_implement_review() {
  let flow = default_flow(None);
  assert_eq!(flow, vec![Step::Analyze, Step::Implement, Step::Review]);
}

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

// --- 基本実行フロー + 自動挿入ステップ ---

mod basic_flow;

// --- Worktree Setup ---

#[test]
#[ignore]
fn runs_worktree_setup_commands_before_implement() {}
