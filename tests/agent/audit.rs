use pfl_forge::agent::audit;
use pfl_forge::config::Config;
use pfl_forge::knowledge::observation;

use crate::mock_claude::MockClaude;

fn default_config() -> Config {
  serde_yaml::from_str("{}").unwrap()
}

fn audit_json() -> &'static str {
  r#"{"observations":[{"content":"Missing error handling in handler.rs","evidence":[{"type":"file","ref":"src/handler.rs:42"}]},{"content":"Duplicated validation logic","evidence":[{"type":"file","ref":"src/api.rs:10"},{"type":"file","ref":"src/web.rs:15"}]}]}"#
}

#[test]
fn 監査結果からobservationを記録する() {
  let mock = MockClaude::with_json(audit_json());
  let config = default_config();
  let dir = tempfile::tempdir().unwrap();
  std::fs::create_dir_all(dir.path().join(".forge")).unwrap();

  let result = audit::audit(&config, &mock, dir.path(), None, "test-audit").unwrap();

  assert_eq!(result.observations.len(), 2);
  assert!(result.observations[0]
    .content
    .contains("Missing error handling"));

  // Verify observations were written to file
  let obs_path = dir.path().join(".forge").join("observations.yaml");
  let loaded = observation::load(&obs_path).unwrap();
  assert_eq!(loaded.len(), 2);
  assert_eq!(loaded[0].source, "audit");
  assert!(!loaded[0].processed);
}

#[test]
fn intentは生成しない() {
  let mock = MockClaude::with_json(audit_json());
  let config = default_config();
  let dir = tempfile::tempdir().unwrap();
  std::fs::create_dir_all(dir.path().join(".forge")).unwrap();

  let result = audit::audit(&config, &mock, dir.path(), None, "test-audit").unwrap();

  // AuditResult only contains observations, no intents
  assert_eq!(result.observations.len(), 2);
  // No intents directory should be created
  assert!(!dir.path().join(".forge").join("intents").exists());
}

#[test]
fn パス引数で監査対象を絞れる() {
  let mock = MockClaude::with_json(audit_json());
  let config = default_config();
  let dir = tempfile::tempdir().unwrap();
  std::fs::create_dir_all(dir.path().join(".forge")).unwrap();

  audit::audit(
    &config,
    &mock,
    dir.path(),
    Some("src/handler/"),
    "test-audit",
  )
  .unwrap();

  let call = mock.last_call();
  assert!(call.prompt.contains("src/handler/"));
}

#[test]
fn configのauditモデルを使用する() {
  let mock = MockClaude::with_json(audit_json());
  let config = default_config();
  let dir = tempfile::tempdir().unwrap();
  std::fs::create_dir_all(dir.path().join(".forge")).unwrap();

  audit::audit(&config, &mock, dir.path(), None, "test-audit").unwrap();

  let call = mock.last_call();
  // Uses models.audit (defaults to opus)
  assert_eq!(call.model, pfl_forge::claude::model::OPUS);
}

#[test]
fn claudeエラーを伝播する() {
  let mock = MockClaude::with_error("API error");
  let config = default_config();
  let dir = tempfile::tempdir().unwrap();
  std::fs::create_dir_all(dir.path().join(".forge")).unwrap();

  let result = audit::audit(&config, &mock, dir.path(), None, "test-audit");
  assert!(result.is_err());
}
