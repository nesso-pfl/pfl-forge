use std::time::Duration;

use pfl_forge::agent::analyze;
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

  let result = analyze::analyze(&intent, &config, &mock, std::path::Path::new(".")).unwrap();

  assert_eq!(result.complexity, "low");
  assert_eq!(result.plan, "Write tests");
  assert_eq!(result.relevant_files, vec!["src/lib.rs"]);
  assert_eq!(result.implementation_steps, vec!["Add test module"]);
  assert!(result.is_sufficient());
}

#[test]
#[ignore]
fn returns_child_intents_when_problem_too_large() {}

#[test]
#[ignore]
fn returns_needs_clarification_when_info_insufficient() {}

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
