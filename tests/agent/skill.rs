use pfl_forge::agent::skill;
use pfl_forge::config::Config;

use crate::mock_claude::MockClaude;

fn default_config() -> Config {
  serde_yaml::from_str("{}").unwrap()
}

#[test]
fn 履歴からパターンを検出する() {
  let mock = MockClaude::with_json(
    r#"{"patterns":[{"name":"test-first","description":"Write tests before implementation","frequency":3,"examples":["intent-1","intent-2","intent-3"]}]}"#,
  );
  let config = default_config();
  let dir = tempfile::tempdir().unwrap();

  // Create history entries
  let history_dir = dir.path().join(".forge").join("knowledge").join("history");
  std::fs::create_dir_all(&history_dir).unwrap();
  std::fs::write(
    history_dir.join("intent-1.yaml"),
    "intent_id: intent-1\ntitle: Add feature A\nflow: [analyze, implement, review]\nstep_results:\n  - step: analyze\n    duration_secs: 10\noutcome: success\n",
  ).unwrap();
  std::fs::write(
    history_dir.join("intent-2.yaml"),
    "intent_id: intent-2\ntitle: Add feature B\nflow: [analyze, implement, review]\nstep_results:\n  - step: analyze\n    duration_secs: 8\noutcome: success\n",
  ).unwrap();

  let (result, _meta) = skill::observe(&config, &mock, dir.path()).unwrap();

  assert_eq!(result.patterns.len(), 1);
  assert_eq!(result.patterns[0].name, "test-first");
  assert_eq!(result.patterns[0].frequency, 3);
}

#[test]
fn 履歴がなければ空のパターンを返す() {
  let mock = MockClaude::with_json(r#"{"patterns":[]}"#);
  let config = default_config();
  let dir = tempfile::tempdir().unwrap();

  let (result, _meta) = skill::observe(&config, &mock, dir.path()).unwrap();

  assert!(result.patterns.is_empty());
  assert_eq!(mock.call_count(), 0); // Claude should not be called
}

#[test]
fn パターンからスキルテンプレートを生成する() {
  let mock = MockClaude::with_json(
    r#"{"skills":[{"name":"test-driven","description":"Write tests before code","instructions":"1. Read the task\n2. Write tests first\n3. Implement to pass tests"}]}"#,
  );
  let config = default_config();
  let dir = tempfile::tempdir().unwrap();

  let patterns = vec![skill::ObservedPattern {
    name: "test-first".into(),
    description: "Write tests before implementation".into(),
    frequency: 3,
    examples: vec!["intent-1".into()],
  }];

  let (result, _meta) = skill::abstract_patterns(&config, &mock, dir.path(), &patterns).unwrap();

  assert_eq!(result.skills.len(), 1);
  assert_eq!(result.skills[0].name, "test-driven");
}

#[test]
fn パターンが空ならスキル生成をスキップする() {
  let mock = MockClaude::with_json(r#"{"skills":[]}"#);
  let config = default_config();
  let dir = tempfile::tempdir().unwrap();

  let (result, _meta) = skill::abstract_patterns(&config, &mock, dir.path(), &[]).unwrap();

  assert!(result.skills.is_empty());
  assert_eq!(mock.call_count(), 0);
}

#[test]
fn スキルをファイルに書き出す() {
  let dir = tempfile::tempdir().unwrap();
  let skills = vec![skill::SkillDraft {
    name: "test-driven".into(),
    description: "Write tests before code".into(),
    instructions: "1. Read the task\n2. Write tests first".into(),
  }];

  let written = skill::record(dir.path(), &skills).unwrap();

  assert_eq!(written, vec!["test-driven"]);

  let path = dir
    .path()
    .join(".claude")
    .join("skills")
    .join("test-driven")
    .join("SKILL.md");
  assert!(path.exists());

  let content = std::fs::read_to_string(&path).unwrap();
  assert!(content.contains("description: Write tests before code"));
  assert!(content.contains("1. Read the task"));
}
