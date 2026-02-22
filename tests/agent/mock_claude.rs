use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::time::Duration;

use pfl_forge::claude::runner::Claude;
use pfl_forge::error::{ForgeError, Result};

#[derive(Debug, Clone)]
pub struct CapturedCall {
  pub prompt: String,
  pub system_prompt: String,
  pub model: String,
  pub cwd: PathBuf,
  pub timeout: Option<Duration>,
}

pub struct MockClaude {
  response: Result<String>,
  pub calls: RefCell<Vec<CapturedCall>>,
}

impl MockClaude {
  /// Create a mock that wraps `inner_json` in Claude's `{"result": "..."}` envelope.
  pub fn with_json(inner_json: &str) -> Self {
    let escaped = inner_json.replace('\\', "\\\\").replace('"', "\\\"");
    let response = format!(r#"{{"result": "{escaped}"}}"#);
    Self {
      response: Ok(response),
      calls: RefCell::new(Vec::new()),
    }
  }

  pub fn with_error(msg: &str) -> Self {
    Self {
      response: Err(ForgeError::Claude(msg.to_string())),
      calls: RefCell::new(Vec::new()),
    }
  }

  pub fn last_call(&self) -> CapturedCall {
    self
      .calls
      .borrow()
      .last()
      .expect("no calls recorded")
      .clone()
  }
}

impl Claude for MockClaude {
  fn run_prompt(
    &self,
    prompt: &str,
    system_prompt: &str,
    model: &str,
    cwd: &Path,
    timeout: Option<Duration>,
  ) -> Result<String> {
    self.calls.borrow_mut().push(CapturedCall {
      prompt: prompt.to_string(),
      system_prompt: system_prompt.to_string(),
      model: model.to_string(),
      cwd: cwd.to_path_buf(),
      timeout,
    });
    match &self.response {
      Ok(s) => Ok(s.clone()),
      Err(e) => Err(ForgeError::Claude(format!("{e}"))),
    }
  }
}
