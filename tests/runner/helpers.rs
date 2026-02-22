use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use pfl_forge::claude::runner::Claude;
use pfl_forge::config::Config;
use pfl_forge::error::{ForgeError, Result};
use pfl_forge::intent::registry::Intent;

pub struct MockClaude {
  responses: RefCell<Vec<Result<String>>>,
  pub calls: RefCell<Vec<String>>,
}

impl MockClaude {
  pub fn with_sequence(responses: Vec<Result<String>>) -> Self {
    Self {
      responses: RefCell::new(responses),
      calls: RefCell::new(Vec::new()),
    }
  }

  pub fn call_count(&self) -> usize {
    self.calls.borrow().len()
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
  ) -> Result<String> {
    self.calls.borrow_mut().push(prompt.to_string());
    let mut responses = self.responses.borrow_mut();
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

pub fn approved_review_json() -> &'static str {
  r#"{"approved":true,"issues":[],"suggestions":[]}"#
}

pub fn rejected_review_json() -> &'static str {
  r#"{"approved":false,"issues":["Missing tests"],"suggestions":["Add unit tests"]}"#
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

pub fn load_intent(repo_path: &Path, intent_id: &str) -> Intent {
  let intents_dir = repo_path.join(".forge").join("intents");
  Intent::fetch_all(&intents_dir)
    .unwrap()
    .into_iter()
    .find(|i| i.id() == intent_id)
    .unwrap()
}
