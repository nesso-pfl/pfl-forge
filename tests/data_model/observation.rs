use pfl_forge::knowledge::observation::{self, Evidence, EvidenceType, Observation};

// --- YAML パース ---

#[test]
fn parses_observation_yaml_with_all_fields() {
  let yaml = r#"
- content: "Duplicate validation logic found"
  evidence:
    - type: file
      ref: src/handler/login.rs
    - type: file
      ref: src/handler/signup.rs
  source: implement
  intent_id: fix-login
  processed: false
  created_at: "2026-01-01T00:00:00Z"
"#;
  let observations: Vec<Observation> = serde_yaml::from_str(yaml).unwrap();
  assert_eq!(observations.len(), 1);
  let obs = &observations[0];
  assert_eq!(obs.content, "Duplicate validation logic found");
  assert_eq!(obs.source, "implement");
  assert_eq!(obs.intent_id, "fix-login");
  assert!(!obs.processed);
  assert_eq!(obs.evidence.len(), 2);
}

#[test]
fn evidence_type_supports_file_skill_history_decision() {
  let yaml = r#"
- content: "test"
  evidence:
    - type: file
      ref: src/main.rs
    - type: skill
      ref: .claude/skills/api/SKILL.md
    - type: history
      ref: fix-login
    - type: decision
      ref: error-handling-policy
  source: reflect
  intent_id: test-id
"#;
  let observations: Vec<Observation> = serde_yaml::from_str(yaml).unwrap();
  let evidence = &observations[0].evidence;
  assert_eq!(evidence[0].evidence_type, EvidenceType::File);
  assert_eq!(evidence[1].evidence_type, EvidenceType::Skill);
  assert_eq!(evidence[2].evidence_type, EvidenceType::History);
  assert_eq!(evidence[3].evidence_type, EvidenceType::Decision);
}

#[test]
fn processed_defaults_to_false() {
  let yaml = r#"
- content: "something"
  source: audit
  intent_id: audit-1
"#;
  let observations: Vec<Observation> = serde_yaml::from_str(yaml).unwrap();
  assert!(!observations[0].processed);
}

// --- 読み書き ---

#[test]
fn appends_observation_to_yaml_file() {
  let dir = tempfile::tempdir().unwrap();
  let path = dir.path().join("observations.yaml");

  let obs1 = Observation {
    content: "First observation".into(),
    evidence: vec![],
    source: "implement".into(),
    intent_id: "intent-1".into(),
    processed: false,
    created_at: None,
  };
  let obs2 = Observation {
    content: "Second observation".into(),
    evidence: vec![Evidence {
      evidence_type: EvidenceType::File,
      reference: "src/lib.rs".into(),
    }],
    source: "audit".into(),
    intent_id: "intent-2".into(),
    processed: false,
    created_at: None,
  };

  observation::append(&path, &obs1).unwrap();
  observation::append(&path, &obs2).unwrap();

  let loaded = observation::load(&path).unwrap();
  assert_eq!(loaded.len(), 2);
  assert_eq!(loaded[0].content, "First observation");
  assert_eq!(loaded[1].content, "Second observation");
  assert_eq!(loaded[1].evidence.len(), 1);
}

#[test]
fn collects_unprocessed_observations() {
  let observations = vec![
    Observation {
      content: "processed".into(),
      evidence: vec![],
      source: "implement".into(),
      intent_id: "a".into(),
      processed: true,
      created_at: None,
    },
    Observation {
      content: "unprocessed".into(),
      evidence: vec![],
      source: "implement".into(),
      intent_id: "b".into(),
      processed: false,
      created_at: None,
    },
  ];

  let unproc = observation::unprocessed(&observations);
  assert_eq!(unproc.len(), 1);
  assert_eq!(unproc[0].content, "unprocessed");
}

#[test]
fn marks_observations_as_processed() {
  let dir = tempfile::tempdir().unwrap();
  let path = dir.path().join("observations.yaml");

  let obs = Observation {
    content: "needs processing".into(),
    evidence: vec![],
    source: "implement".into(),
    intent_id: "target-intent".into(),
    processed: false,
    created_at: None,
  };
  observation::append(&path, &obs).unwrap();

  observation::mark_processed(&path, "target-intent").unwrap();

  let loaded = observation::load(&path).unwrap();
  assert!(loaded[0].processed);
}
