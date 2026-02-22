use pfl_forge::agent::analyze::AnalysisResult;
use pfl_forge::claude::model::{Complexity, OPUS, SONNET};
use pfl_forge::config::ModelSettings;
use pfl_forge::intent::registry::Intent;
use pfl_forge::task::{set_task_status, Task, WorkStatus};

fn sample_intent() -> Intent {
  serde_yaml::from_str("title: Test task\nbody: Implement feature X\nsource: human\n").unwrap()
}

fn sample_analysis() -> AnalysisResult {
  AnalysisResult {
    complexity: "high".into(),
    plan: "Step-by-step plan".into(),
    relevant_files: vec!["src/lib.rs".into()],
    implementation_steps: vec!["Add module".into(), "Write tests".into()],
    context: "Background context".into(),
  }
}

// --- Task 生成 ---

#[test]
fn from_analysis_populates_all_fields() {
  let dir = tempfile::tempdir().unwrap();
  let yaml = "title: Test task\nbody: Implement feature X\nsource: human\n";
  std::fs::write(dir.path().join("task-42.yaml"), yaml).unwrap();
  let intents = Intent::fetch_all(dir.path()).unwrap();
  let intent = &intents[0];

  let analysis = sample_analysis();
  let task = Task::from_analysis(intent, &analysis);

  assert_eq!(task.intent_id, "task-42");
  assert_eq!(task.title, "Test task");
  assert_eq!(task.complexity, "high");
  assert_eq!(task.plan, "Step-by-step plan");
  assert_eq!(task.relevant_files, vec!["src/lib.rs"]);
  assert_eq!(task.implementation_steps.len(), 2);
  assert_eq!(task.context, "Background context");
}

#[test]
fn from_analysis_sets_status_pending() {
  let intent = sample_intent();
  let analysis = sample_analysis();
  let task = Task::from_analysis(&intent, &analysis);
  assert_eq!(task.status, WorkStatus::Pending);
}

// --- complexity ---

#[test]
fn complexity_low_selects_default_model() {
  let settings = ModelSettings::default();
  let model = Complexity::Low.select_model(&settings);
  assert_eq!(model, SONNET);
}

#[test]
fn complexity_high_selects_complex_model() {
  let settings = ModelSettings::default();
  let model = Complexity::High.select_model(&settings);
  assert_eq!(model, OPUS);
}

#[test]
fn unknown_complexity_defaults_to_medium() {
  let intent = sample_intent();
  let mut analysis = sample_analysis();
  analysis.complexity = "unknown_value".into();
  let task = Task::from_analysis(&intent, &analysis);
  assert_eq!(task.complexity(), Complexity::Medium);
}

// --- YAML I/O ---

#[test]
fn write_and_read_task_yaml_roundtrip() {
  let intent = sample_intent();
  let analysis = sample_analysis();
  let task = Task::from_analysis(&intent, &analysis);

  let dir = tempfile::tempdir().unwrap();
  pfl_forge::task::write_task_yaml(dir.path(), &task).unwrap();

  let content = std::fs::read_to_string(dir.path().join(".forge/task.yaml")).unwrap();
  let loaded: Task = serde_yaml::from_str(&content).unwrap();
  assert_eq!(loaded.title, task.title);
  assert_eq!(loaded.complexity, task.complexity);
  assert_eq!(loaded.relevant_files, task.relevant_files);
}

#[test]
fn set_task_status_updates_yaml_file() {
  let dir = tempfile::tempdir().unwrap();
  let intent = sample_intent();
  let analysis = sample_analysis();
  let task = Task::from_analysis(&intent, &analysis);

  let yaml = serde_yaml::to_string(&task).unwrap();
  let path = dir.path().join("task.yaml");
  std::fs::write(&path, yaml).unwrap();

  set_task_status(&path, WorkStatus::Completed).unwrap();

  let content = std::fs::read_to_string(&path).unwrap();
  let loaded: Task = serde_yaml::from_str(&content).unwrap();
  assert_eq!(loaded.status, WorkStatus::Completed);
}
