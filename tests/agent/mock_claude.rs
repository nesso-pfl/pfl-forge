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
  responses: RefCell<Vec<Result<String>>>,
  pub calls: RefCell<Vec<CapturedCall>>,
}

impl MockClaude {
  /// Create a mock that wraps `inner_json` in Claude's `{"result": "..."}` envelope.
  pub fn with_json(inner_json: &str) -> Self {
    let escaped = inner_json.replace('\\', "\\\\").replace('"', "\\\"");
    let response = format!(r#"{{"result": "{escaped}"}}"#);
    Self {
      responses: RefCell::new(vec![Ok(response)]),
      calls: RefCell::new(Vec::new()),
    }
  }

  pub fn with_error(msg: &str) -> Self {
    Self {
      responses: RefCell::new(vec![Err(ForgeError::Claude(msg.to_string()))]),
      calls: RefCell::new(Vec::new()),
    }
  }

  /// Create a mock that returns responses in sequence.
  /// Each call to `run_prompt` pops the next response.
  /// If responses are exhausted, returns the last one repeatedly.
  pub fn with_sequence(responses: Vec<Result<String>>) -> Self {
    Self {
      responses: RefCell::new(responses),
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

  pub fn call_count(&self) -> usize {
    self.calls.borrow().len()
  }
}

/// Wrap inner_json in Claude's result envelope.
pub fn json_response(inner_json: &str) -> Result<String> {
  let escaped = inner_json.replace('\\', "\\\\").replace('"', "\\\"");
  Ok(format!(r#"{{"result": "{escaped}"}}"#))
}

/// Wrap raw string in Claude's result envelope (for implement which returns raw text).
pub fn raw_response(text: &str) -> Result<String> {
  Ok(format!(r#"{{"result": "{}"}}"#, text.replace('"', "\\\"")))
}

pub fn error_response(msg: &str) -> Result<String> {
  Err(ForgeError::Claude(msg.to_string()))
}

impl Claude for MockClaude {
  fn run_prompt(
    &self,
    prompt: &str,
    system_prompt: &str,
    model: &str,
    cwd: &Path,
    timeout: Option<Duration>,
    _session_id: Option<&str>,
  ) -> Result<String> {
    self.calls.borrow_mut().push(CapturedCall {
      prompt: prompt.to_string(),
      system_prompt: system_prompt.to_string(),
      model: model.to_string(),
      cwd: cwd.to_path_buf(),
      timeout,
    });
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
