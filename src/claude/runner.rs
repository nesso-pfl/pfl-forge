use std::path::Path;
use std::process::Command;

use serde::de::DeserializeOwned;
use tracing::{debug, info};

use crate::error::{ForgeError, Result};

pub struct ClaudeRunner {
    allowed_tools: Vec<String>,
}

impl ClaudeRunner {
    pub fn new(allowed_tools: Vec<String>) -> Self {
        Self { allowed_tools }
    }

    /// Run claude -p with a prompt, returning raw stdout.
    pub fn run_prompt(
        &self,
        prompt: &str,
        model: &str,
        cwd: &Path,
    ) -> Result<String> {
        let tools_csv = self.allowed_tools.join(",");

        info!("running claude -p with model={model} in {}", cwd.display());
        debug!("prompt: {prompt}");

        let mut cmd = Command::new("claude");
        cmd.args(["-p", "--model", model, "--output-format", "json"])
            .args(["--allowedTools", &tools_csv])
            .current_dir(cwd)
            .env_remove("CLAUDE_CODE_ENTRYPOINT");

        // Remove CLAUDECODE env var to allow nested Claude Code invocation
        cmd.env_remove("CLAUDECODE");

        let child = cmd
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()?;

        // Write prompt to stdin
        use std::io::Write;
        let mut child = child;
        if let Some(mut stdin) = child.stdin.take() {
            stdin.write_all(prompt.as_bytes())?;
        }

        let output = child.wait_with_output()?;

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

    /// Run claude -p and parse the JSON result field from the output.
    pub fn run_json<T: DeserializeOwned>(
        &self,
        prompt: &str,
        model: &str,
        cwd: &Path,
    ) -> Result<T> {
        let raw = self.run_prompt(prompt, model, cwd)?;
        parse_claude_json_output(&raw)
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
    fn test_extract_json_raw() {
        let input = r#"{"actionable": true, "complexity": "low"}"#;
        assert_eq!(extract_json(input), input);
    }

    #[test]
    fn test_extract_json_code_block() {
        let input = "Here's the result:\n```json\n{\"actionable\": true}\n```\n";
        assert_eq!(extract_json(input), "{\"actionable\": true}");
    }

    #[test]
    fn test_extract_json_with_surrounding_text() {
        let input = "The analysis shows: {\"key\": \"value\"} end";
        assert_eq!(extract_json(input), "{\"key\": \"value\"}");
    }

    #[test]
    fn test_parse_claude_json_output() {
        #[derive(serde::Deserialize)]
        struct TestOutput {
            actionable: bool,
        }

        let raw = r#"{"result": "{\"actionable\": true}", "cost_usd": 0.01}"#;
        let parsed: TestOutput = parse_claude_json_output(raw).unwrap();
        assert!(parsed.actionable);
    }
}
