use pfl_forge::knowledge::history::{self, HistoryEntry, Outcome, StepResult};

// --- YAML パース ---

#[test]
fn 全フィールド付きのhistory_yamlをパースする() {
  let yaml = r#"
intent_id: fix-login
intent_type: fix
intent_risk: low
title: "Fix login validation"
flow:
  - analyze
  - implement
  - review
step_results:
  - step: analyze
    duration_secs: 45
  - step: implement
    duration_secs: 120
  - step: review
    duration_secs: 30
outcome: success
failure_reason: null
observations:
  - obs-001
created_at: "2026-01-01T00:00:00Z"
"#;
  let entry: HistoryEntry = serde_yaml::from_str(yaml).unwrap();
  assert_eq!(entry.intent_id, "fix-login");
  assert_eq!(entry.intent_type.as_deref(), Some("fix"));
  assert_eq!(entry.title, "Fix login validation");
  assert_eq!(entry.flow, vec!["analyze", "implement", "review"]);
  assert_eq!(entry.step_results.len(), 3);
  assert_eq!(entry.outcome, Outcome::Success);
  assert!(entry.failure_reason.is_none());
  assert_eq!(entry.observations, vec!["obs-001"]);
}

#[test]
fn outcomeはsuccess_failed_escalatedをサポートする() {
  for (yaml_val, expected) in [
    ("success", Outcome::Success),
    ("failed", Outcome::Failed),
    ("escalated", Outcome::Escalated),
  ] {
    let yaml = format!("intent_id: t\ntitle: t\nflow: []\noutcome: {yaml_val}\n");
    let entry: HistoryEntry = serde_yaml::from_str(&yaml).unwrap();
    assert_eq!(entry.outcome, expected);
  }
}

#[test]
fn failure_reasonは省略可能() {
  let yaml = "intent_id: t\ntitle: t\nflow: []\noutcome: success\n";
  let entry: HistoryEntry = serde_yaml::from_str(yaml).unwrap();
  assert!(entry.failure_reason.is_none());

  let yaml_with_reason =
    "intent_id: t\ntitle: t\nflow: []\noutcome: failed\nfailure_reason: timeout\n";
  let entry: HistoryEntry = serde_yaml::from_str(yaml_with_reason).unwrap();
  assert_eq!(entry.failure_reason.as_deref(), Some("timeout"));
}

// --- 読み書き ---

#[test]
fn historyエントリをyamlに書き込む() {
  let dir = tempfile::tempdir().unwrap();
  let entry = HistoryEntry {
    intent_id: "test-intent".into(),
    intent_type: Some("feature".into()),
    intent_risk: Some("low".into()),
    title: "Test feature".into(),
    flow: vec!["analyze".into(), "implement".into(), "review".into()],
    step_results: vec![],
    outcome: Outcome::Success,
    failure_reason: None,
    observations: vec![],
    created_at: None,
  };

  history::write(dir.path(), &entry).unwrap();
  let loaded = history::load(dir.path(), "test-intent").unwrap();
  assert_eq!(loaded.title, "Test feature");
  assert_eq!(loaded.outcome, Outcome::Success);
}

#[test]
fn step_resultsに所要時間が含まれる() {
  let dir = tempfile::tempdir().unwrap();
  let entry = HistoryEntry {
    intent_id: "perf-test".into(),
    intent_type: None,
    intent_risk: None,
    title: "Performance".into(),
    flow: vec!["analyze".into(), "implement".into()],
    step_results: vec![
      StepResult {
        step: "analyze".into(),
        duration_secs: 45,
        metadata: None,
      },
      StepResult {
        step: "implement".into(),
        duration_secs: 300,
        metadata: None,
      },
    ],
    outcome: Outcome::Success,
    failure_reason: None,
    observations: vec![],
    created_at: None,
  };

  history::write(dir.path(), &entry).unwrap();
  let loaded = history::load(dir.path(), "perf-test").unwrap();
  assert_eq!(loaded.step_results.len(), 2);
  assert_eq!(loaded.step_results[0].step, "analyze");
  assert_eq!(loaded.step_results[0].duration_secs, 45);
  assert_eq!(loaded.step_results[1].duration_secs, 300);
}

#[test]
fn observation参照が含まれる() {
  let dir = tempfile::tempdir().unwrap();
  let entry = HistoryEntry {
    intent_id: "obs-test".into(),
    intent_type: None,
    intent_risk: None,
    title: "Obs test".into(),
    flow: vec!["implement".into()],
    step_results: vec![],
    outcome: Outcome::Success,
    failure_reason: None,
    observations: vec!["obs-001".into(), "obs-002".into()],
    created_at: None,
  };

  history::write(dir.path(), &entry).unwrap();
  let loaded = history::load(dir.path(), "obs-test").unwrap();
  assert_eq!(loaded.observations, vec!["obs-001", "obs-002"]);
}
