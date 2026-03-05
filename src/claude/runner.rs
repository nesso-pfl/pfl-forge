use std::path::Path;
use std::process::Command;
use std::time::Duration;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use tracing::{debug, info, warn};

use crate::error::{ForgeError, Result};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ClaudeMetadata {
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub session_id: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub cost_usd: Option<f64>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub duration_ms: Option<u64>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub duration_api_ms: Option<u64>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub input_tokens: Option<u64>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub output_tokens: Option<u64>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub cache_read_input_tokens: Option<u64>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub cache_creation_input_tokens: Option<u64>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub num_turns: Option<u64>,
}

/// Session handling for Claude CLI invocations.
#[derive(Debug, Clone, Default)]
pub enum SessionMode {
  /// Generate a new session with a pre-determined UUID.
  New(String),
  /// Resume an existing session.
  Resume(String),
  /// No session management.
  #[default]
  None,
}

impl SessionMode {
  pub fn session_id(&self) -> Option<&str> {
    match self {
      SessionMode::New(id) | SessionMode::Resume(id) => Some(id),
      SessionMode::None => Option::None,
    }
  }

  /// Generate a new session with a random UUID.
  pub fn new_session() -> Self {
    SessionMode::New(uuid::Uuid::new_v4().to_string())
  }
}

pub trait Claude {
  fn run_prompt(
    &self,
    prompt: &str,
    system_prompt: &str,
    model: &str,
    cwd: &Path,
    timeout: Option<Duration>,
    session: &SessionMode,
  ) -> Result<String>;

  fn run_json<T: DeserializeOwned>(
    &self,
    prompt: &str,
    system_prompt: &str,
    model: &str,
    cwd: &Path,
    timeout: Option<Duration>,
  ) -> Result<T> {
    let raw = self.run_prompt(
      prompt,
      system_prompt,
      model,
      cwd,
      timeout,
      &SessionMode::None,
    )?;
    parse_claude_json_output(&raw)
  }

  fn run_json_with_meta<T: DeserializeOwned>(
    &self,
    prompt: &str,
    system_prompt: &str,
    model: &str,
    cwd: &Path,
    timeout: Option<Duration>,
    session: &SessionMode,
  ) -> Result<(T, ClaudeMetadata)> {
    let raw = self.run_prompt(prompt, system_prompt, model, cwd, timeout, session)?;
    let metadata = parse_metadata(&raw);
    let result = parse_claude_json_output(&raw)?;
    Ok((result, metadata))
  }
}

#[derive(Clone)]
pub struct ClaudeRunner {
  allowed_tools: Vec<String>,
  mcp_config: Option<String>,
}

impl ClaudeRunner {
  pub fn new(
    mut allowed_tools: Vec<String>,
    mcp_config: Option<String>,
    memory_server: Option<&str>,
  ) -> Self {
    // --allowedTools blocks everything not listed, including MCP tools.
    // Allow all tools on the specified MCP server by server-name prefix.
    if let Some(server) = memory_server {
      if !allowed_tools.iter().any(|t| t.starts_with("mcp__")) {
        allowed_tools.push(format!("mcp__{server}"));
      }
    }
    Self {
      allowed_tools,
      mcp_config,
    }
  }
}

impl Claude for ClaudeRunner {
  fn run_prompt(
    &self,
    prompt: &str,
    system_prompt: &str,
    model: &str,
    cwd: &Path,
    timeout: Option<Duration>,
    session: &SessionMode,
  ) -> Result<String> {
    let tools_csv = self.allowed_tools.join(",");

    info!("running claude -p with model={model} in {}", cwd.display());
    debug!("prompt: {prompt}");

    let mut cmd = Command::new("claude");
    cmd
      .args(["-p", "--model", model, "--output-format", "json"])
      .args(["--allowedTools", &tools_csv])
      .current_dir(cwd)
      .env_remove("CLAUDE_CODE_ENTRYPOINT");

    match session {
      SessionMode::New(id) => {
        cmd.args(["--session-id", id]);
      }
      SessionMode::Resume(id) => {
        cmd.args(["--resume", id]);
      }
      SessionMode::None => {}
    }

    if let Some(ref mcp_path) = self.mcp_config {
      cmd.args(["--mcp-config", mcp_path]);
    }

    if !system_prompt.is_empty() {
      cmd.args(["--append-system-prompt", system_prompt]);
    }

    // Remove CLAUDECODE env var to allow nested Claude Code invocation
    cmd.env_remove("CLAUDECODE");

    let mut child = cmd
      .stdin(std::process::Stdio::piped())
      .stdout(std::process::Stdio::piped())
      .stderr(std::process::Stdio::piped())
      .spawn()?;

    // Write prompt to stdin
    use std::io::Write;
    if let Some(mut stdin) = child.stdin.take() {
      stdin.write_all(prompt.as_bytes())?;
    }

    let output = if let Some(dur) = timeout {
      wait_with_timeout(child, dur)?
    } else {
      child.wait_with_output()?
    };

    if !output.status.success() {
      let stderr = String::from_utf8_lossy(&output.stderr);
      return Err(ForgeError::Claude(format!(
        "claude exited with {}: {stderr}",
        output.status
      )));
    }

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    debug!("claude output length: {} bytes", stdout.len());
    Ok(stdout)
  }
}

fn wait_with_timeout(
  mut child: std::process::Child,
  timeout: Duration,
) -> Result<std::process::Output> {
  use std::io::Read;

  let stdout_pipe = child.stdout.take();
  let stderr_pipe = child.stderr.take();

  let stdout_handle = std::thread::spawn(move || {
    let mut buf = Vec::new();
    if let Some(mut pipe) = stdout_pipe {
      let _ = pipe.read_to_end(&mut buf);
    }
    buf
  });

  let stderr_handle = std::thread::spawn(move || {
    let mut buf = Vec::new();
    if let Some(mut pipe) = stderr_pipe {
      let _ = pipe.read_to_end(&mut buf);
    }
    buf
  });

  let start = std::time::Instant::now();
  let poll_interval = Duration::from_secs(1);

  loop {
    match child.try_wait()? {
      Some(status) => {
        let stdout = stdout_handle.join().unwrap_or_default();
        let stderr = stderr_handle.join().unwrap_or_default();
        return Ok(std::process::Output {
          status,
          stdout,
          stderr,
        });
      }
      None if start.elapsed() >= timeout => {
        warn!(
          "claude process timed out after {}s, killing",
          timeout.as_secs()
        );
        let _ = child.kill();
        let _ = child.wait();
        return Err(ForgeError::Timeout(format!(
          "timed out after {}s",
          timeout.as_secs()
        )));
      }
      None => std::thread::sleep(poll_interval),
    }
  }
}

/// Parse Claude's --output-format json response.
/// The response is a JSON object with a "result" field containing the actual text output.
/// We then try to parse that text as JSON of the expected type.
fn parse_claude_json_output<T: DeserializeOwned>(raw: &str) -> Result<T> {
  // First, parse the wrapper object
  let wrapper: serde_json::Value = serde_json::from_str(raw)
    .map_err(|e| ForgeError::Claude(format!("failed to parse claude output as JSON: {e}")))?;

  let result_text = wrapper
    .get("result")
    .and_then(|v| v.as_str())
    .ok_or_else(|| ForgeError::Claude("claude output missing 'result' field".into()))?;

  // Try to extract JSON from the result text (may be wrapped in markdown code block)
  let json_str = extract_json(result_text);

  serde_json::from_str(json_str)
    .map_err(|e| ForgeError::Claude(format!("failed to parse result as expected type: {e}")))
}

/// Extract metadata from the Claude JSON wrapper (session_id, tokens, cost, etc.)
pub fn parse_metadata(raw: &str) -> ClaudeMetadata {
  let wrapper: serde_json::Value = match serde_json::from_str(raw) {
    Ok(v) => v,
    Err(_) => return ClaudeMetadata::default(),
  };
  let usage = wrapper.get("usage");
  ClaudeMetadata {
    session_id: wrapper
      .get("session_id")
      .and_then(|v| v.as_str())
      .map(String::from),
    cost_usd: wrapper.get("total_cost_usd").and_then(|v| v.as_f64()),
    duration_ms: wrapper.get("duration_ms").and_then(|v| v.as_u64()),
    duration_api_ms: wrapper.get("duration_api_ms").and_then(|v| v.as_u64()),
    num_turns: wrapper.get("num_turns").and_then(|v| v.as_u64()),
    input_tokens: usage
      .and_then(|u| u.get("input_tokens"))
      .and_then(|v| v.as_u64()),
    output_tokens: usage
      .and_then(|u| u.get("output_tokens"))
      .and_then(|v| v.as_u64()),
    cache_read_input_tokens: usage
      .and_then(|u| u.get("cache_read_input_tokens"))
      .and_then(|v| v.as_u64()),
    cache_creation_input_tokens: usage
      .and_then(|u| u.get("cache_creation_input_tokens"))
      .and_then(|v| v.as_u64()),
  }
}

/// `claude -p --output-format json` の応答は常にきれいな JSON とは限らない。
/// markdown コードブロックで囲まれていたり、前後に説明テキストが付くことがある。
/// この関数はそうした出力から JSON 部分だけを切り出す。
fn extract_json(text: &str) -> &str {
  // Try to find JSON in a code block
  if let Some(start) = text.find("```json") {
    let json_start = start + 7;
    if let Some(end) = text[json_start..].find("```") {
      return text[json_start..json_start + end].trim();
    }
  }
  if let Some(start) = text.find("```") {
    let json_start = start + 3;
    // Skip language identifier if on the same line
    let json_start = text[json_start..]
      .find('\n')
      .map(|n| json_start + n + 1)
      .unwrap_or(json_start);
    if let Some(end) = text[json_start..].find("```") {
      return text[json_start..json_start + end].trim();
    }
  }
  // Try raw JSON (find first { or [)
  if let Some(start) = text.find('{') {
    if let Some(end) = text.rfind('}') {
      return &text[start..=end];
    }
  }
  text.trim()
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn 生のjsonをそのまま返す() {
    let input = r#"{"actionable": true, "complexity": "low"}"#;
    assert_eq!(extract_json(input), input);
  }

  #[test]
  fn コードブロックマーカーを除去する() {
    let input = "Here's the result:\n```json\n{\"actionable\": true}\n```\n";
    assert_eq!(extract_json(input), "{\"actionable\": true}");
  }

  #[test]
  fn 前後のテキストからjsonを抽出する() {
    let input = "The analysis shows: {\"key\": \"value\"} end";
    assert_eq!(extract_json(input), "{\"key\": \"value\"}");
  }

  #[test]
  fn claude出力からresultフィールドを抽出する() {
    #[derive(serde::Deserialize)]
    struct TestOutput {
      actionable: bool,
    }

    let raw = r#"{"result": "{\"actionable\": true}", "cost_usd": 0.01}"#;
    let parsed: TestOutput = parse_claude_json_output(raw).unwrap();
    assert!(parsed.actionable);
  }

  #[test]
  fn ラッパーからメタデータを抽出する() {
    let raw = r#"{
      "type": "result",
      "result": "hello",
      "session_id": "abc-123",
      "total_cost_usd": 0.044,
      "duration_ms": 2514,
      "duration_api_ms": 2482,
      "num_turns": 3,
      "usage": {
        "input_tokens": 100,
        "output_tokens": 50,
        "cache_read_input_tokens": 200,
        "cache_creation_input_tokens": 300
      }
    }"#;
    let meta = parse_metadata(raw);
    assert_eq!(meta.session_id.as_deref(), Some("abc-123"));
    assert_eq!(meta.cost_usd, Some(0.044));
    assert_eq!(meta.duration_ms, Some(2514));
    assert_eq!(meta.duration_api_ms, Some(2482));
    assert_eq!(meta.num_turns, Some(3));
    assert_eq!(meta.input_tokens, Some(100));
    assert_eq!(meta.output_tokens, Some(50));
    assert_eq!(meta.cache_read_input_tokens, Some(200));
    assert_eq!(meta.cache_creation_input_tokens, Some(300));
  }

  #[test]
  fn メタデータ欠損時はデフォルト値を返す() {
    let raw = r#"{"result": "hello"}"#;
    let meta = parse_metadata(raw);
    assert!(meta.session_id.is_none());
    assert!(meta.cost_usd.is_none());
    assert!(meta.input_tokens.is_none());
  }

  #[test]
  fn 不正なjsonではデフォルトメタデータを返す() {
    let meta = parse_metadata("not json");
    assert!(meta.session_id.is_none());
  }

  #[test]
  fn new_sessionはuuidを持つnewバリアントを返す() {
    let session = SessionMode::new_session();
    match &session {
      SessionMode::New(id) => {
        assert!(!id.is_empty());
        // UUID v4 format: 8-4-4-4-12
        assert_eq!(id.len(), 36);
      }
      other => panic!("expected New, got {:?}", other),
    }
  }

  #[test]
  fn session_idはnewとresumeでidを返しnoneではnoneを返す() {
    let new = SessionMode::New("abc-123".into());
    assert_eq!(new.session_id(), Some("abc-123"));

    let resume = SessionMode::Resume("def-456".into());
    assert_eq!(resume.session_id(), Some("def-456"));

    let none = SessionMode::None;
    assert_eq!(none.session_id(), Option::None);
  }

  #[test]
  fn new_sessionは毎回異なるuuidを生成する() {
    let s1 = SessionMode::new_session();
    let s2 = SessionMode::new_session();
    assert_ne!(s1.session_id(), s2.session_id());
  }

  #[test]
  fn memory_serverありならmcpサーバープレフィックスを追加する() {
    let runner = ClaudeRunner::new(
      vec!["Read".into(), "Write".into()],
      None,
      Some("memory-pfl"),
    );
    assert!(runner
      .allowed_tools
      .contains(&"mcp__memory-pfl".to_string()));
  }

  #[test]
  fn memory_serverなしならmcpツールを追加しない() {
    let runner = ClaudeRunner::new(vec!["Read".into()], None, None);
    assert!(!runner.allowed_tools.iter().any(|t| t.starts_with("mcp__")));
  }

  #[test]
  fn 既にmcpツールがあれば重複追加しない() {
    let runner = ClaudeRunner::new(vec!["mcp__custom-server".into()], None, Some("memory-pfl"));
    assert_eq!(
      runner
        .allowed_tools
        .iter()
        .filter(|t| t.starts_with("mcp__"))
        .count(),
      1
    );
  }
}
