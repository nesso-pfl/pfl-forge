use std::time::Duration;

use pfl_forge::agent::analyze::{self, ActiveIntentContext, AnalysisOutcome};
use pfl_forge::claude::model::OPUS;
use pfl_forge::claude::runner::SessionMode;
use pfl_forge::config::Config;
use pfl_forge::intent::registry::Clarification;
use pfl_forge::intent::registry::Intent;

use crate::mock_claude::{CapturedSession, MockClaude};

fn default_config() -> Config {
  serde_yaml::from_str("{}").unwrap()
}

fn sample_intent() -> Intent {
  let dir = tempfile::tempdir().unwrap();
  let yaml = "title: Add tests\nbody: Write spec tests for module X\nsource: human\n";
  std::fs::write(dir.path().join("add-tests.yaml"), yaml).unwrap();
  let intents = Intent::fetch_all(dir.path()).unwrap();
  // Leak tempdir so the intent stays valid
  let intents = intents;
  std::mem::forget(dir);
  intents.into_iter().next().unwrap()
}

fn analysis_json() -> String {
  r#"{"complexity":"low","plan":"Write tests","relevant_files":["src/lib.rs"],"implementation_steps":["Add test module"],"context":"Testing context"}"#.to_string()
}

#[test]
fn 成功した分析からタスクスペックを返す() {
  let mock = MockClaude::with_json(&analysis_json());
  let config = default_config();
  let intent = sample_intent();

  let (outcome, _meta, _depends, _obs) = analyze::analyze(
    &intent,
    &config,
    &mock,
    std::path::Path::new("."),
    &[],
    &SessionMode::new_session(),
  )
  .unwrap();

  let specs = match outcome {
    AnalysisOutcome::Tasks(s) => s,
    other => panic!("expected Tasks, got {:?}", other),
  };
  assert_eq!(specs.len(), 1);
  let spec = &specs[0];
  assert_eq!(spec.complexity, "low");
  assert_eq!(spec.plan, "Write tests");
  assert_eq!(spec.relevant_files, vec!["src/lib.rs"]);
  assert_eq!(spec.implementation_steps, vec!["Add test module"]);
}

#[test]
fn 複数タスクとdepends_onを返す() {
  let json = r#"{"tasks":[{"id":"task-a","title":"Setup DB","complexity":"low","plan":"Create schema","relevant_files":["db.rs"],"implementation_steps":["Add migration"],"context":"","depends_on":[]},{"id":"task-b","title":"Add API","complexity":"medium","plan":"Build endpoint","relevant_files":["api.rs"],"implementation_steps":["Add route"],"context":"","depends_on":["task-a"]}]}"#;
  let mock = MockClaude::with_json(json);
  let config = default_config();
  let intent = sample_intent();

  let (outcome, _meta, _depends, _obs) = analyze::analyze(
    &intent,
    &config,
    &mock,
    std::path::Path::new("."),
    &[],
    &SessionMode::new_session(),
  )
  .unwrap();

  let specs = match outcome {
    AnalysisOutcome::Tasks(s) => s,
    other => panic!("expected Tasks, got {:?}", other),
  };
  assert_eq!(specs.len(), 2);
  assert_eq!(specs[0].id, "task-a");
  assert_eq!(specs[1].id, "task-b");
  assert_eq!(specs[1].depends_on, vec!["task-a"]);
}

#[test]
fn 問題が大きい場合は子intentを返す() {
  let json = r#"{"outcome":"child_intents","child_intents":[{"title":"Sub task A","body":"Do A"},{"title":"Sub task B","body":"Do B"}]}"#;
  let mock = MockClaude::with_json(json);
  let config = default_config();
  let intent = sample_intent();

  let (outcome, _meta, _depends, _obs) = analyze::analyze(
    &intent,
    &config,
    &mock,
    std::path::Path::new("."),
    &[],
    &SessionMode::new_session(),
  )
  .unwrap();

  match outcome {
    AnalysisOutcome::ChildIntents(children) => {
      assert_eq!(children.len(), 2);
      assert_eq!(children[0].title, "Sub task A");
      assert_eq!(children[1].title, "Sub task B");
    }
    other => panic!("expected ChildIntents, got {:?}", other),
  }
}

#[test]
fn 情報不足の場合はclarificationを返す() {
  let json =
    r#"{"outcome":"needs_clarification","clarifications":["What is the target API version?"]}"#;
  let mock = MockClaude::with_json(json);
  let config = default_config();
  let intent = sample_intent();

  let (outcome, _meta, _depends, _obs) = analyze::analyze(
    &intent,
    &config,
    &mock,
    std::path::Path::new("."),
    &[],
    &SessionMode::new_session(),
  )
  .unwrap();

  match outcome {
    AnalysisOutcome::NeedsClarification { clarifications } => {
      assert_eq!(clarifications.len(), 1);
      assert_eq!(clarifications[0], "What is the target API version?");
    }
    other => panic!("expected NeedsClarification, got {:?}", other),
  }
}

#[test]
fn configのanalyzeモデルを使用する() {
  let mock = MockClaude::with_json(&analysis_json());
  let config = default_config();
  let intent = sample_intent();

  analyze::analyze(
    &intent,
    &config,
    &mock,
    std::path::Path::new("."),
    &[],
    &SessionMode::new_session(),
  )
  .unwrap();

  let call = mock.last_call();
  assert_eq!(call.model, OPUS);
}

#[test]
fn configのanalyzeタイムアウトを使用する() {
  let mock = MockClaude::with_json(&analysis_json());
  let config = default_config();
  let intent = sample_intent();

  analyze::analyze(
    &intent,
    &config,
    &mock,
    std::path::Path::new("."),
    &[],
    &SessionMode::new_session(),
  )
  .unwrap();

  let call = mock.last_call();
  assert_eq!(call.timeout, Some(Duration::from_secs(600)));
}

#[test]
fn プロンプトにintentのid_title_bodyが含まれる() {
  let mock = MockClaude::with_json(&analysis_json());
  let config = default_config();
  let intent = sample_intent();

  analyze::analyze(
    &intent,
    &config,
    &mock,
    std::path::Path::new("."),
    &[],
    &SessionMode::new_session(),
  )
  .unwrap();

  let call = mock.last_call();
  assert!(call.prompt.contains("add-tests"));
  assert!(call.prompt.contains("Add tests"));
  assert!(call.prompt.contains("Write spec tests for module X"));
}

#[test]
fn active_intentのコンテキストをプロンプトに含める() {
  let mock = MockClaude::with_json(&analysis_json());
  let config = default_config();
  let intent = sample_intent();

  let active = vec![ActiveIntentContext {
    id: "other-intent".into(),
    title: "Refactor auth".into(),
    status: "implementing".into(),
    relevant_files: vec!["src/auth.rs".into(), "src/session.rs".into()],
    plan: Some("Extract auth logic into separate module".into()),
  }];

  analyze::analyze(
    &intent,
    &config,
    &mock,
    std::path::Path::new("."),
    &active,
    &SessionMode::new_session(),
  )
  .unwrap();

  let call = mock.last_call();
  assert!(call.prompt.contains("Active Intents"));
  assert!(call.prompt.contains("other-intent"));
  assert!(call.prompt.contains("Refactor auth"));
  assert!(call.prompt.contains("src/auth.rs"));
  assert!(call.prompt.contains("Extract auth logic"));
}

#[test]
fn active_intentが空ならセクションを省略する() {
  let mock = MockClaude::with_json(&analysis_json());
  let config = default_config();
  let intent = sample_intent();

  analyze::analyze(
    &intent,
    &config,
    &mock,
    std::path::Path::new("."),
    &[],
    &SessionMode::new_session(),
  )
  .unwrap();

  let call = mock.last_call();
  assert!(!call.prompt.contains("Active Intents"));
}

#[test]
fn claudeエラーを伝播する() {
  let mock = MockClaude::with_error("API rate limit exceeded");
  let config = default_config();
  let intent = sample_intent();

  let result = analyze::analyze(
    &intent,
    &config,
    &mock,
    std::path::Path::new("."),
    &[],
    &SessionMode::new_session(),
  );
  assert!(result.is_err());
  let err = result.unwrap_err().to_string();
  assert!(err.contains("rate limit"), "error was: {err}");
}

#[test]
fn 回答済みclarificationをプロンプトに含める() {
  let mock = MockClaude::with_json(&analysis_json());
  let config = default_config();
  let mut intent = sample_intent();
  intent.clarifications = vec![
    Clarification {
      question: "Which API version?".into(),
      answer: Some("v2".into()),
    },
    Clarification {
      question: "Unanswered question".into(),
      answer: None,
    },
  ];

  analyze::analyze(
    &intent,
    &config,
    &mock,
    std::path::Path::new("."),
    &[],
    &SessionMode::new_session(),
  )
  .unwrap();

  let call = mock.last_call();
  assert!(call.prompt.contains("Human Decisions"));
  assert!(call.prompt.contains("Which API version?"));
  assert!(call.prompt.contains("v2"));
  assert!(!call.prompt.contains("Unanswered question"));
}

#[test]
fn resume時にclarificationが空ならフルプロンプトを使う() {
  let mock = MockClaude::with_json(&analysis_json());
  let config = default_config();
  let intent = sample_intent();

  // Resume with no clarifications — should fall back to full prompt
  analyze::analyze(
    &intent,
    &config,
    &mock,
    std::path::Path::new("."),
    &[],
    &SessionMode::Resume("prev-session".into()),
  )
  .unwrap();

  let call = mock.last_call();
  // Full prompt contains intent title, not "Clarification answers"
  assert!(
    call.prompt.contains("Add tests"),
    "should use full prompt, got: {}",
    &call.prompt[..100.min(call.prompt.len())]
  );
  assert!(
    !call.prompt.contains("Clarification answers"),
    "should not send clarification resume prompt"
  );
}

#[test]
fn resume時にclarification回答済みならresumeプロンプトを使う() {
  let mock = MockClaude::with_json(&analysis_json());
  let config = default_config();
  let mut intent = sample_intent();
  intent.clarifications = vec![Clarification {
    question: "Which API version?".into(),
    answer: Some("v2".into()),
  }];

  analyze::analyze(
    &intent,
    &config,
    &mock,
    std::path::Path::new("."),
    &[],
    &SessionMode::Resume("prev-session".into()),
  )
  .unwrap();

  let call = mock.last_call();
  assert!(call.prompt.contains("Clarification answers"));
  assert!(call.prompt.contains("Which API version?"));
  assert!(call.prompt.contains("v2"));
  // Session should be Resume
  assert_eq!(call.session, CapturedSession::Resume("prev-session".into()));
}

#[test]
fn clarificationが空ならセクションを省略する() {
  let mock = MockClaude::with_json(&analysis_json());
  let config = default_config();
  let intent = sample_intent();

  analyze::analyze(
    &intent,
    &config,
    &mock,
    std::path::Path::new("."),
    &[],
    &SessionMode::new_session(),
  )
  .unwrap();

  let call = mock.last_call();
  assert!(!call.prompt.contains("Human Decisions"));
}
