use pfl_forge::agent::reflect;
use pfl_forge::config::Config;
use pfl_forge::intent::registry::Intent;
use pfl_forge::knowledge::observation::{self, Observation};

use crate::mock_claude::MockClaude;

fn default_config() -> Config {
  serde_yaml::from_str("{}").unwrap()
}

fn sample_intent(dir: &std::path::Path) -> Intent {
  let intents_dir = dir.join(".forge").join("intents");
  std::fs::create_dir_all(&intents_dir).unwrap();
  let yaml = "title: Fix bug\nbody: Fix validation\nsource: human\n";
  std::fs::write(intents_dir.join("fix-bug.yaml"), yaml).unwrap();
  Intent::fetch_all(&intents_dir)
    .unwrap()
    .into_iter()
    .next()
    .unwrap()
}

fn write_observations(dir: &std::path::Path, intent_id: &str, count: usize) {
  let obs_path = dir.join(".forge").join("observations.yaml");
  for i in 0..count {
    let obs = Observation {
      content: format!("observation {i}"),
      evidence: vec![],
      source: "implement".to_string(),
      intent_id: intent_id.to_string(),
      processed: false,
      created_at: None,
    };
    observation::append(&obs_path, &obs).unwrap();
  }
}

fn reflect_json() -> &'static str {
  r#"{"intents":[{"title":"Extract shared validation","body":"Deduplicate validation logic","type":"refactor","risk":"low"}]}"#
}

#[test]
fn generates_intents_from_observations() {
  let dir = tempfile::tempdir().unwrap();
  std::fs::create_dir_all(dir.path().join(".forge")).unwrap();
  let intent = sample_intent(dir.path());
  write_observations(dir.path(), "fix-bug", 2);

  let mock = MockClaude::with_json(reflect_json());
  let config = default_config();

  let result = reflect::reflect(&intent, &config, &mock, dir.path()).unwrap();

  assert_eq!(result.intents.len(), 1);
  assert_eq!(result.intents[0].title, "Extract shared validation");

  // Verify intent file was written
  let intents_dir = dir.path().join(".forge").join("intents");
  let intents = Intent::fetch_all(&intents_dir).unwrap();
  let generated = intents
    .iter()
    .find(|i| i.title == "Extract shared validation");
  assert!(generated.is_some());
}

#[test]
fn generated_intents_have_reflection_source() {
  let dir = tempfile::tempdir().unwrap();
  std::fs::create_dir_all(dir.path().join(".forge")).unwrap();
  let intent = sample_intent(dir.path());
  write_observations(dir.path(), "fix-bug", 1);

  let mock = MockClaude::with_json(reflect_json());
  let config = default_config();

  reflect::reflect(&intent, &config, &mock, dir.path()).unwrap();

  let intents_dir = dir.path().join(".forge").join("intents");
  let intents = Intent::fetch_all(&intents_dir).unwrap();
  let generated = intents
    .iter()
    .find(|i| i.title == "Extract shared validation")
    .unwrap();
  assert_eq!(generated.source, "reflection");
}

#[test]
fn processes_only_unprocessed_observations() {
  let dir = tempfile::tempdir().unwrap();
  std::fs::create_dir_all(dir.path().join(".forge")).unwrap();
  let intent = sample_intent(dir.path());

  // Write one processed and one unprocessed observation
  let obs_path = dir.path().join(".forge").join("observations.yaml");
  let processed_obs = Observation {
    content: "already processed".into(),
    evidence: vec![],
    source: "implement".into(),
    intent_id: "fix-bug".into(),
    processed: true,
    created_at: None,
  };
  let unprocessed_obs = Observation {
    content: "new observation".into(),
    evidence: vec![],
    source: "implement".into(),
    intent_id: "fix-bug".into(),
    processed: false,
    created_at: None,
  };
  observation::append(&obs_path, &processed_obs).unwrap();
  observation::append(&obs_path, &unprocessed_obs).unwrap();

  let mock = MockClaude::with_json(reflect_json());
  let config = default_config();

  reflect::reflect(&intent, &config, &mock, dir.path()).unwrap();

  // The prompt should only contain the unprocessed observation
  let call = mock.last_call();
  assert!(call.prompt.contains("new observation"));
  assert!(!call.prompt.contains("already processed"));
}

#[test]
fn marks_observations_as_processed() {
  let dir = tempfile::tempdir().unwrap();
  std::fs::create_dir_all(dir.path().join(".forge")).unwrap();
  let intent = sample_intent(dir.path());
  write_observations(dir.path(), "fix-bug", 2);

  let mock = MockClaude::with_json(reflect_json());
  let config = default_config();

  reflect::reflect(&intent, &config, &mock, dir.path()).unwrap();

  // All observations for this intent should now be processed
  let obs_path = dir.path().join(".forge").join("observations.yaml");
  let obs = observation::load(&obs_path).unwrap();
  assert!(obs.iter().all(|o| o.processed));
}

#[test]
fn propagates_claude_error() {
  let dir = tempfile::tempdir().unwrap();
  std::fs::create_dir_all(dir.path().join(".forge")).unwrap();
  let intent = sample_intent(dir.path());
  write_observations(dir.path(), "fix-bug", 1);

  let mock = MockClaude::with_error("API error");
  let config = default_config();

  let result = reflect::reflect(&intent, &config, &mock, dir.path());
  assert!(result.is_err());
}
