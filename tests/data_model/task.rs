use pfl_forge::agent::analyze::TaskSpec;
use pfl_forge::claude::model::{Complexity, OPUS, SONNET};
use pfl_forge::config::ModelSettings;
use pfl_forge::intent::registry::Intent;
use pfl_forge::task::{Task, WorkStatus};

fn sample_intent() -> Intent {
  serde_yaml::from_str("title: Test task\nbody: Implement feature X\nsource: human\n").unwrap()
}

fn sample_spec() -> TaskSpec {
  TaskSpec {
    id: String::new(),
    title: String::new(),
    complexity: "high".into(),
    plan: "Step-by-step plan".into(),
    relevant_files: vec!["src/lib.rs".into()],
    implementation_steps: vec!["Add module".into(), "Write tests".into()],
    context: "Background context".into(),
    depends_on: vec![],
  }
}

// --- Task 生成 ---

#[test]
fn from_specで全フィールドが設定される() {
  let dir = tempfile::tempdir().unwrap();
  let yaml = "title: Test task\nbody: Implement feature X\nsource: human\n";
  std::fs::write(dir.path().join("task-42.yaml"), yaml).unwrap();
  let intents = Intent::fetch_all(dir.path()).unwrap();
  let intent = &intents[0];

  let spec = sample_spec();
  let task = Task::from_spec(intent, &spec);

  assert_eq!(task.intent_id, "task-42");
  assert_eq!(task.title, "Test task");
  assert_eq!(task.complexity, "high");
  assert_eq!(task.plan, "Step-by-step plan");
  assert_eq!(task.relevant_files, vec!["src/lib.rs"]);
  assert_eq!(task.implementation_steps.len(), 2);
  assert_eq!(task.context, "Background context");
}

#[test]
fn from_specでステータスがpendingになる() {
  let intent = sample_intent();
  let spec = sample_spec();
  let task = Task::from_spec(&intent, &spec);
  assert_eq!(task.status, WorkStatus::Pending);
}

// --- complexity ---

#[test]
fn 低complexityはデフォルトモデルを選択する() {
  let settings = ModelSettings::default();
  let model = Complexity::Low.select_model(&settings);
  assert_eq!(model, SONNET);
}

#[test]
fn 高complexityはcomplexモデルを選択する() {
  let settings = ModelSettings::default();
  let model = Complexity::High.select_model(&settings);
  assert_eq!(model, OPUS);
}

#[test]
fn 不明なcomplexityはmediumにデフォルトする() {
  let intent = sample_intent();
  let spec = TaskSpec {
    complexity: "unknown_value".into(),
    ..sample_spec()
  };
  let task = Task::from_spec(&intent, &spec);
  assert_eq!(task.complexity(), Complexity::Medium);
}

// --- YAML I/O ---

#[test]
fn tasks_yamlの書き込みと読み込みが往復する() {
  let intent = sample_intent();
  let spec = sample_spec();
  let task = Task::from_spec(&intent, &spec);

  let dir = tempfile::tempdir().unwrap();
  let repo_path = dir.path();
  pfl_forge::task::write_all_tasks(repo_path, "test-intent", &[task.clone()]).unwrap();

  let loaded = pfl_forge::task::read_all_tasks(repo_path, "test-intent").unwrap();
  assert_eq!(loaded.len(), 1);
  assert_eq!(loaded[0].title, task.title);
  assert_eq!(loaded[0].complexity, task.complexity);
  assert_eq!(loaded[0].relevant_files, task.relevant_files);
}

#[test]
fn tasks_existが正しく判定する() {
  let dir = tempfile::tempdir().unwrap();
  let repo_path = dir.path();

  assert!(!pfl_forge::task::tasks_exist(repo_path, "nonexistent"));

  let intent = sample_intent();
  let spec = sample_spec();
  let task = Task::from_spec(&intent, &spec);
  pfl_forge::task::write_all_tasks(repo_path, "exists", &[task]).unwrap();

  assert!(pfl_forge::task::tasks_exist(repo_path, "exists"));
}
