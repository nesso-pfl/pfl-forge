use pfl_forge::intent::registry::Intent;

// --- YAML パース ---

#[test]
fn 全フィールド付きのintent_yamlをパースする() {
  let yaml = r#"
title: "Add user authentication"
body: "Implement OAuth2 login flow"
type: feature
source: human
risk: low
status: approved
parent: parent-intent-1
clarifications:
  - question: "Which OAuth provider?"
    answer: "Google"
created_at: "2026-01-01T00:00:00Z"
"#;
  let intent: Intent = serde_yaml::from_str(yaml).unwrap();
  assert_eq!(intent.title, "Add user authentication");
  assert_eq!(intent.body, "Implement OAuth2 login flow");
  assert_eq!(intent.intent_type.as_deref(), Some("feature"));
  assert_eq!(intent.source, "human");
  assert_eq!(intent.risk.as_deref(), Some("low"));
  assert_eq!(intent.parent.as_deref(), Some("parent-intent-1"));
  assert_eq!(intent.clarifications.len(), 1);
  assert_eq!(intent.clarifications[0].answer.as_deref(), Some("Google"));
}

#[test]
fn idはファイル名のstemになる() {
  let dir = tempfile::tempdir().unwrap();
  let yaml = "title: test\nbody: test body\nsource: human\n";
  std::fs::write(dir.path().join("my-intent-id.yaml"), yaml).unwrap();

  let intents = Intent::fetch_all(dir.path()).unwrap();
  assert_eq!(intents[0].id(), "my-intent-id");
}

#[test]
fn 省略可能フィールドはnoneにデフォルトする() {
  let yaml = "title: minimal\nbody: just the basics\nsource: human\n";
  let intent: Intent = serde_yaml::from_str(yaml).unwrap();
  assert!(intent.intent_type.is_none());
  assert!(intent.risk.is_none());
  assert!(intent.parent.is_none());
  assert!(intent.created_at.is_none());
  assert!(intent.clarifications.is_empty());
}

// --- needs_clarification ---

#[test]
fn answerがnullの場合はclarification必要() {
  let yaml = r#"
title: test
body: body
source: human
clarifications:
  - question: "Which approach?"
    answer: null
"#;
  let intent: Intent = serde_yaml::from_str(yaml).unwrap();
  assert!(intent.needs_clarification());
}

#[test]
fn 全回答済みならclarification不要() {
  let yaml = r#"
title: test
body: body
source: human
clarifications:
  - question: "Which approach?"
    answer: "Option A"
"#;
  let intent: Intent = serde_yaml::from_str(yaml).unwrap();
  assert!(!intent.needs_clarification());
}

#[test]
fn clarificationが空ならclarification不要() {
  let yaml = "title: test\nbody: body\nsource: human\n";
  let intent: Intent = serde_yaml::from_str(yaml).unwrap();
  assert!(!intent.needs_clarification());
}

// --- fetch_all ---

#[test]
fn ディレクトリからyamlファイルを読み込む() {
  let dir = tempfile::tempdir().unwrap();
  let yaml_a = "title: Alpha\nbody: first\nsource: human\n";
  let yaml_b = "title: Beta\nbody: second\nsource: reflection\n";
  std::fs::write(dir.path().join("a-intent.yaml"), yaml_a).unwrap();
  std::fs::write(dir.path().join("b-intent.yaml"), yaml_b).unwrap();

  let intents = Intent::fetch_all(dir.path()).unwrap();
  assert_eq!(intents.len(), 2);
  assert_eq!(intents[0].title, "Alpha");
  assert_eq!(intents[1].title, "Beta");
}

#[test]
fn yaml以外のファイルをスキップする() {
  let dir = tempfile::tempdir().unwrap();
  let yaml = "title: Valid\nbody: intent\nsource: human\n";
  std::fs::write(dir.path().join("valid.yaml"), yaml).unwrap();
  std::fs::write(dir.path().join("readme.md"), "# not an intent").unwrap();
  std::fs::write(dir.path().join("notes.txt"), "ignore me").unwrap();

  let intents = Intent::fetch_all(dir.path()).unwrap();
  assert_eq!(intents.len(), 1);
  assert_eq!(intents[0].title, "Valid");
}

#[test]
fn 存在しないディレクトリでは空を返す() {
  let dir = tempfile::tempdir().unwrap();
  let missing = dir.path().join("nonexistent");

  let intents = Intent::fetch_all(&missing).unwrap();
  assert!(intents.is_empty());
}

// --- intent-drafts ---

#[test]
fn frontmatter付きのintent_draftをパースする() {
  let md = "\
---
type: feature
risk: low
---

Add password reset link to login page.

Users currently have no way to reset their password.
";
  let draft = pfl_forge::intent::draft::parse(md).unwrap();
  assert_eq!(draft.title, "Add password reset link to login page.");
  assert_eq!(
    draft.body,
    "Users currently have no way to reset their password."
  );
  assert_eq!(draft.intent_type.as_deref(), Some("feature"));
  assert_eq!(draft.risk.as_deref(), Some("low"));
}

#[test]
fn intent_draftでtypeとriskを省略できる() {
  let md = "\
---
---

Fix the broken test suite.
";
  let draft = pfl_forge::intent::draft::parse(md).unwrap();
  assert_eq!(draft.title, "Fix the broken test suite.");
  assert!(draft.intent_type.is_none());
  assert!(draft.risk.is_none());
}
