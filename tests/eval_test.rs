use pfl_forge::eval;

#[test]
fn フィクスチャyamlをロードする() {
  let dir = tempfile::tempdir().unwrap();
  let fixtures_dir = dir.path().join("fixtures");
  std::fs::create_dir_all(&fixtures_dir).unwrap();

  std::fs::write(
    fixtures_dir.join("test-fixture.yaml"),
    r#"
intent:
  title: "Add a feature"
  body: "Add a new feature to the system"
expectations:
  relevant_files_contain:
    - "src/"
  plan_mentions:
    - "feature"
  has_implementation_steps: true
  complexity_is_one_of:
    - low
    - medium
"#,
  )
  .unwrap();

  let fixtures = eval::load_fixtures(&fixtures_dir).unwrap();

  assert_eq!(fixtures.len(), 1);
  assert_eq!(fixtures[0].0, "test-fixture");
  assert_eq!(fixtures[0].1.intent.title, "Add a feature");
  assert_eq!(
    fixtures[0].1.expectations.relevant_files_contain,
    vec!["src/"]
  );
  assert_eq!(fixtures[0].1.expectations.plan_mentions, vec!["feature"]);
  assert_eq!(
    fixtures[0].1.expectations.has_implementation_steps,
    Some(true)
  );
  assert_eq!(
    fixtures[0].1.expectations.complexity_is_one_of,
    vec!["low", "medium"]
  );
  assert!(fixtures[0].1.expectations.steps_mention.is_empty());
}

#[test]
fn 存在しないディレクトリでは空を返す() {
  let dir = tempfile::tempdir().unwrap();
  let fixtures = eval::load_fixtures(&dir.path().join("nonexistent")).unwrap();
  assert!(fixtures.is_empty());
}

#[test]
fn yaml以外のファイルをスキップする() {
  let dir = tempfile::tempdir().unwrap();
  let fixtures_dir = dir.path().join("fixtures");
  std::fs::create_dir_all(&fixtures_dir).unwrap();

  std::fs::write(fixtures_dir.join("readme.md"), "# readme").unwrap();
  std::fs::write(
    fixtures_dir.join("valid.yaml"),
    "intent:\n  title: Test\n  body: Body\nexpectations: {}\n",
  )
  .unwrap();

  let fixtures = eval::load_fixtures(&fixtures_dir).unwrap();
  assert_eq!(fixtures.len(), 1);
  assert_eq!(fixtures[0].0, "valid");
}

#[test]
fn expectationsの全フィールドが省略可能() {
  let dir = tempfile::tempdir().unwrap();
  let fixtures_dir = dir.path().join("fixtures");
  std::fs::create_dir_all(&fixtures_dir).unwrap();

  std::fs::write(
    fixtures_dir.join("minimal.yaml"),
    "intent:\n  title: Minimal\n  body: Body\nexpectations: {}\n",
  )
  .unwrap();

  let fixtures = eval::load_fixtures(&fixtures_dir).unwrap();
  assert_eq!(fixtures.len(), 1);
  let exp = &fixtures[0].1.expectations;
  assert!(exp.relevant_files_contain.is_empty());
  assert!(exp.plan_mentions.is_empty());
  assert!(exp.has_implementation_steps.is_none());
  assert!(exp.complexity_is_one_of.is_empty());
  assert!(exp.min_relevant_files.is_none());
  assert!(exp.should_approve.is_none());
}

#[test]
fn eval_analyzeでチェックが実行される() {
  use std::path::Path;
  use std::time::Duration;

  use pfl_forge::claude::runner::Claude;
  use pfl_forge::config::Config;
  use pfl_forge::error::Result;

  struct MockClaude;
  impl Claude for MockClaude {
    fn run_prompt(
      &self,
      _prompt: &str,
      _system_prompt: &str,
      _model: &str,
      _cwd: &Path,
      _timeout: Option<Duration>,
      _session_id: Option<&str>,
    ) -> Result<String> {
      let inner = r#"{"complexity":"low","plan":"Add health endpoint","relevant_files":["src/handler.rs"],"implementation_steps":["Create handler","Add route to router"],"context":""}"#;
      let escaped = inner.replace('\\', "\\\\").replace('"', "\\\"");
      Ok(format!(r#"{{"result": "{escaped}"}}"#))
    }
  }

  let fixture = eval::Fixture {
    intent: eval::FixtureIntent {
      title: "Add health check".into(),
      body: "Add GET /health".into(),
    },
    repo_ref: None,
    diff: None,
    plan: None,
    expectations: eval::Expectations {
      relevant_files_contain: vec!["src/".into()],
      plan_mentions: vec!["health".into()],
      steps_mention: vec!["route".into()],
      has_implementation_steps: Some(true),
      complexity_is_one_of: vec!["low".into(), "medium".into()],
      min_relevant_files: None,
      should_approve: None,
    },
  };

  let config: Config = serde_yaml::from_str("{}").unwrap();
  let dir = tempfile::tempdir().unwrap();

  let result = eval::eval_analyze("test", &fixture, &config, &MockClaude, dir.path()).unwrap();

  assert!(result.all_passed(), "checks: {:?}", result.checks);
  assert!(!result.checks.is_empty());
}
