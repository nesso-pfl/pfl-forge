use std::time::Duration;

use pfl_forge::agent::analyze::{self, AnalysisOutcome};
use pfl_forge::claude::model::SONNET;
use pfl_forge::config::Config;
use pfl_forge::intent::registry::Intent;

use crate::mock_claude::MockClaude;

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
fn returns_tasks_from_successful_analysis() {
  let mock = MockClaude::with_json(&analysis_json());
  let config = default_config();
  let intent = sample_intent();

  let outcome = analyze::analyze(&intent, &config, &mock, std::path::Path::new(".")).unwrap();

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
fn returns_multiple_tasks_with_depends_on() {
  let json = r#"{"tasks":[{"id":"task-a","title":"Setup DB","complexity":"low","plan":"Create schema","relevant_files":["db.rs"],"implementation_steps":["Add migration"],"context":"","depends_on":[]},{"id":"task-b","title":"Add API","complexity":"medium","plan":"Build endpoint","relevant_files":["api.rs"],"implementation_steps":["Add route"],"context":"","depends_on":["task-a"]}]}"#;
  let mock = MockClaude::with_json(json);
  let config = default_config();
  let intent = sample_intent();

  let outcome = analyze::analyze(&intent, &config, &mock, std::path::Path::new(".")).unwrap();

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
fn returns_child_intents_when_problem_too_large() {
  let json = r#"{"outcome":"child_intents","child_intents":[{"title":"Sub task A","body":"Do A"},{"title":"Sub task B","body":"Do B"}]}"#;
  let mock = MockClaude::with_json(json);
  let config = default_config();
  let intent = sample_intent();

  let outcome = analyze::analyze(&intent, &config, &mock, std::path::Path::new(".")).unwrap();

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
fn returns_needs_clarification_when_info_insufficient() {
  let json =
    r#"{"outcome":"needs_clarification","clarifications":["What is the target API version?"]}"#;
  let mock = MockClaude::with_json(json);
  let config = default_config();
  let intent = sample_intent();

  let outcome = analyze::analyze(&intent, &config, &mock, std::path::Path::new(".")).unwrap();

  match outcome {
    AnalysisOutcome::NeedsClarification { clarifications } => {
      assert_eq!(clarifications.len(), 1);
      assert_eq!(clarifications[0], "What is the target API version?");
    }
    other => panic!("expected NeedsClarification, got {:?}", other),
  }
}

#[test]
fn uses_analyze_model_from_config() {
  let mock = MockClaude::with_json(&analysis_json());
  let config = default_config();
  let intent = sample_intent();

  analyze::analyze(&intent, &config, &mock, std::path::Path::new(".")).unwrap();

  let call = mock.last_call();
  assert_eq!(call.model, SONNET);
}

#[test]
fn uses_analyze_timeout_from_config() {
  let mock = MockClaude::with_json(&analysis_json());
  let config = default_config();
  let intent = sample_intent();

  analyze::analyze(&intent, &config, &mock, std::path::Path::new(".")).unwrap();

  let call = mock.last_call();
  assert_eq!(call.timeout, Some(Duration::from_secs(600)));
}

#[test]
fn prompt_contains_intent_id_title_body() {
  let mock = MockClaude::with_json(&analysis_json());
  let config = default_config();
  let intent = sample_intent();

  analyze::analyze(&intent, &config, &mock, std::path::Path::new(".")).unwrap();

  let call = mock.last_call();
  assert!(call.prompt.contains("add-tests"));
  assert!(call.prompt.contains("Add tests"));
  assert!(call.prompt.contains("Write spec tests for module X"));
}

#[test]
fn propagates_claude_error() {
  let mock = MockClaude::with_error("API rate limit exceeded");
  let config = default_config();
  let intent = sample_intent();

  let result = analyze::analyze(&intent, &config, &mock, std::path::Path::new("."));
  assert!(result.is_err());
  let err = result.unwrap_err().to_string();
  assert!(err.contains("rate limit"), "error was: {err}");
}
