use crate::config::Config;
use crate::error::Result;
use crate::prompt;

pub fn launch(config: &Config, model: Option<&str>) -> Result<()> {
  let mut cmd = std::process::Command::new("claude");
  cmd
    .arg("--append-system-prompt")
    .arg(prompt::OPERATOR)
    .arg("--allowedTools")
    .arg("Bash");

  if let Some(m) = model {
    cmd.arg("--model").arg(m);
  }

  // TODO: build initial message with inbox summary
  cmd.arg("pfl-forge is ready.");

  use std::os::unix::process::CommandExt;
  let err = cmd.exec();
  Err(crate::error::ForgeError::Claude(format!(
    "exec failed: {err}"
  )))
}
