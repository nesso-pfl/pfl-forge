use crate::config::ModelSettings;

pub const HAIKU: &str = "claude-haiku-4-5-20251001";
pub const SONNET: &str = "claude-sonnet-4-5-20250929";
pub const OPUS: &str = "claude-opus-4-6";

pub fn resolve(name: &str) -> &'static str {
  match name.to_lowercase().as_str() {
    "haiku" => HAIKU,
    "sonnet" => SONNET,
    "opus" => OPUS,
    _ => SONNET,
  }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Complexity {
  Low,
  Medium,
  High,
}

impl Complexity {
  pub fn select_model(self, settings: &ModelSettings) -> &'static str {
    match self {
      Complexity::Low => resolve(&settings.default),
      Complexity::Medium => resolve(&settings.default),
      Complexity::High => resolve(&settings.complex),
    }
  }
}

impl std::str::FromStr for Complexity {
  type Err = String;

  fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
    match s.to_lowercase().as_str() {
      "low" => Ok(Complexity::Low),
      "medium" => Ok(Complexity::Medium),
      "high" => Ok(Complexity::High),
      _ => Err(format!("unknown complexity: {s}")),
    }
  }
}
