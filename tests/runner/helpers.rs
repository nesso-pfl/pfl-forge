use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;
use std::time::Duration;

use pfl_forge::claude::runner::Claude;
use pfl_forge::config::Config;
use pfl_forge::error::{ForgeError, Result};
use pfl_forge::intent::registry::Intent;

pub struct MockClaude {
  responses: Mutex<Vec<Result<String>>>,
  pub calls: Mutex<Vec<String>>,
}

impl MockClaude {
  pub fn with_sequence(responses: Vec<Result<String>>) -> Self {
    Self {
      responses: Mutex::new(responses),
      calls: Mutex::new(Vec::new()),
    }
  }

  pub fn call_count(&self) -> usize {
    self.calls.lock().unwrap().len()
  }
}

impl Claude for MockClaude {
  fn run_prompt(
    &self,
    prompt: &str,
    _system_prompt: &str,
    _model: &str,
    _cwd: &Path,
    _timeout: Option<Duration>,
    _session_id: Option<&str>,
  ) -> Result<String> {
    self.calls.lock().unwrap().push(prompt.to_string());
    let mut responses = self.responses.lock().unwrap();
    if responses.len() > 1 {
      let resp = responses.remove(0);
      match resp {
        Ok(s) => Ok(s),
        Err(e) => Err(ForgeError::Claude(format!("{e}"))),
      }
    } else if let Some(resp) = responses.first() {
      match resp {
        Ok(s) => Ok(s.clone()),
        Err(e) => Err(ForgeError::Claude(format!("{e}"))),
      }
    } else {
      Err(ForgeError::Claude("no responses configured".into()))
    }
  }
}

/// Wrap inner_json in Claude's `{"result": "..."}` envelope
pub fn json_response(inner_json: &str) -> Result<String> {
  let escaped = inner_json.replace('\\', "\\\\").replace('"', "\\\"");
  Ok(format!(r#"{{"result": "{escaped}"}}"#))
}

pub fn raw_response(text: &str) -> Result<String> {
  Ok(format!(r#"{{"result": "{}"}}"#, text.replace('"', "\\\"")))
}

pub fn error_response(msg: &str) -> Result<String> {
  Err(ForgeError::Claude(msg.to_string()))
}

pub fn default_config() -> Config {
  serde_yaml::from_str("{}").unwrap()
}

pub fn analysis_json() -> &'static str {
  r#"{"complexity":"low","plan":"Write tests","relevant_files":["src/lib.rs"],"implementation_steps":["Add test module"],"context":"Testing context"}"#
}

pub fn multi_task_analysis_json() -> &'static str {
  r#"{"tasks":[{"id":"task-a","title":"First task","complexity":"low","plan":"Do A","relevant_files":["a.rs"],"implementation_steps":["Step A"],"context":"","depends_on":[]},{"id":"task-b","title":"Second task","complexity":"low","plan":"Do B","relevant_files":["b.rs"],"implementation_steps":["Step B"],"context":"","depends_on":["task-a"]}]}"#
}

pub fn two_independent_tasks_json() -> &'static str {
  r#"{"tasks":[{"id":"task-a","title":"First task","complexity":"low","plan":"Do A","relevant_files":["a.rs"],"implementation_steps":["Step A"],"context":"","depends_on":[]},{"id":"task-b","title":"Second task","complexity":"low","plan":"Do B","relevant_files":["b.rs"],"implementation_steps":["Step B"],"context":"","depends_on":[]}]}"#
}

pub fn approved_review_json() -> &'static str {
  r#"{"approved":true,"issues":[],"suggestions":[]}"#
}

pub fn rejected_review_json() -> &'static str {
  r#"{"approved":false,"issues":["Missing tests"],"suggestions":["Add unit tests"]}"#
}

pub fn reflect_json() -> &'static str {
  r#"{"intents":[]}"#
}

fn git(cwd: &Path, args: &[&str]) -> std::process::Output {
  Command::new("git")
    .args(args)
    .current_dir(cwd)
    .env("GIT_AUTHOR_NAME", "test")
    .env("GIT_AUTHOR_EMAIL", "test@test.com")
    .env("GIT_COMMITTER_NAME", "test")
    .env("GIT_COMMITTER_EMAIL", "test@test.com")
    .output()
    .expect("git failed")
}

/// Set up a temp git repo with a bare origin, and an intent file.
/// Returns (TempDir, repo_path).
pub fn setup_repo_with_intent(intent_id: &str) -> (tempfile::TempDir, PathBuf) {
  let dir = tempfile::tempdir().unwrap();
  let origin_path = dir.path().join("origin.git");
  let repo_path = dir.path().join("repo");

  // Create bare origin
  std::fs::create_dir_all(&origin_path).unwrap();
  git(&origin_path, &["init", "--bare"]);

  // Create working repo
  std::fs::create_dir_all(&repo_path).unwrap();
  git(&repo_path, &["init", "-b", "main"]);
  git(
    &repo_path,
    &["remote", "add", "origin", origin_path.to_str().unwrap()],
  );

  // Initial commit and push
  std::fs::write(repo_path.join("file.txt"), "original\n").unwrap();
  git(&repo_path, &["add", "."]);
  git(&repo_path, &["commit", "-m", "initial"]);
  git(&repo_path, &["push", "-u", "origin", "main"]);

  // Create intent file
  let intents_dir = repo_path.join(".forge").join("intents");
  std::fs::create_dir_all(&intents_dir).unwrap();
  let yaml = "title: Fix bug\nbody: Fix the validation bug\nsource: human\nstatus: approved\n";
  std::fs::write(intents_dir.join(format!("{intent_id}.yaml")), yaml).unwrap();

  (dir, repo_path)
}

/// Set up a repo where a pre-existing branch conflicts with origin/main.
/// Creates branch `forge/{intent_id}` with a conflicting commit on file.txt,
/// then pushes a different change to file.txt on origin/main.
pub fn setup_repo_with_conflict(intent_id: &str) -> (tempfile::TempDir, PathBuf) {
  let (dir, repo_path) = setup_repo_with_intent(intent_id);

  let branch = format!("forge/{intent_id}");

  // Create the branch with a conflicting commit
  git(&repo_path, &["checkout", "-b", &branch]);
  std::fs::write(repo_path.join("file.txt"), "branch change\n").unwrap();
  git(&repo_path, &["add", "file.txt"]);
  git(&repo_path, &["commit", "-m", "branch commit"]);
  git(&repo_path, &["checkout", "main"]);

  // Push a conflicting change to origin/main
  std::fs::write(repo_path.join("file.txt"), "main change\n").unwrap();
  git(&repo_path, &["add", "file.txt"]);
  git(&repo_path, &["commit", "-m", "conflicting main commit"]);
  git(&repo_path, &["push", "origin", "main"]);

  (dir, repo_path)
}

pub fn setup_repo_with_audit_intent(intent_id: &str) -> (tempfile::TempDir, PathBuf) {
  let dir = tempfile::tempdir().unwrap();
  let repo_path = dir.path().join("repo");

  std::fs::create_dir_all(&repo_path).unwrap();

  // Create .forge directories
  let intents_dir = repo_path.join(".forge").join("intents");
  std::fs::create_dir_all(&intents_dir).unwrap();
  let knowledge_dir = repo_path.join(".forge").join("knowledge").join("history");
  std::fs::create_dir_all(&knowledge_dir).unwrap();

  let yaml = format!(
    "title: Audit codebase\nbody: Run audit\nsource: human\ntype: audit\nstatus: approved\n"
  );
  std::fs::write(intents_dir.join(format!("{intent_id}.yaml")), yaml).unwrap();

  (dir, repo_path)
}

pub fn audit_result_json() -> &'static str {
  r#"{"observations":[{"content":"Found unused import","evidence":[{"type":"file","ref":"src/main.rs:3"}]}]}"#
}

pub fn add_intent(repo_path: &Path, intent_id: &str, status: &str) {
  let intents_dir = repo_path.join(".forge").join("intents");
  let yaml =
    format!("title: {intent_id}\nbody: Body of {intent_id}\nsource: human\nstatus: {status}\n");
  std::fs::write(intents_dir.join(format!("{intent_id}.yaml")), yaml).unwrap();
}

pub fn load_intent(repo_path: &Path, intent_id: &str) -> Intent {
  let intents_dir = repo_path.join(".forge").join("intents");
  Intent::fetch_all(&intents_dir)
    .unwrap()
    .into_iter()
    .find(|i| i.id() == intent_id)
    .unwrap()
}

pub fn add_implementing_intent(
  repo_path: &Path,
  intent_id: &str,
  last_step: Option<&str>,
  session_id: Option<&str>,
) {
  let intents_dir = repo_path.join(".forge").join("intents");
  let mut yaml =
    format!("title: {intent_id}\nbody: Body of {intent_id}\nsource: human\nstatus: implementing\n");
  if let Some(step) = last_step {
    yaml.push_str(&format!("last_step: {step}\n"));
  }
  if let Some(sid) = session_id {
    yaml.push_str(&format!("session_id: {sid}\n"));
  }
  std::fs::write(intents_dir.join(format!("{intent_id}.yaml")), yaml).unwrap();
}

pub fn setup_worktree_with_tasks(repo_path: &Path, config: &Config, intent_id: &str) -> PathBuf {
  let branch = format!("forge/{intent_id}");
  let worktree_path = pfl_forge::git::worktree::create(
    repo_path,
    &config.worktree_dir,
    &branch,
    &config.base_branch,
  )
  .unwrap();
  pfl_forge::git::worktree::ensure_gitignore_forge(&worktree_path).unwrap();

  // Write tasks.yaml
  let tasks_yaml = format!(
    r#"- id: {intent_id}
  title: {intent_id}
  intent_id: {intent_id}
  status: pending
  complexity: low
  plan: Do something
  relevant_files:
    - src/lib.rs
  implementation_steps:
    - Step 1
  context: ""
  depends_on: []
"#
  );
  let forge_dir = worktree_path.join(".forge");
  std::fs::create_dir_all(&forge_dir).unwrap();
  std::fs::write(forge_dir.join("tasks.yaml"), tasks_yaml).unwrap();

  worktree_path
}
