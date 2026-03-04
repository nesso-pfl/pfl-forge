use std::path::Path;

use serde::Deserialize;
use tracing::info;

use crate::agent::analyze::{self, AnalysisOutcome};
use crate::agent::review;
use crate::claude::runner::Claude;
use crate::config::Config;
use crate::error::Result;
use crate::intent::registry::Intent;
use crate::task::Task;

#[derive(Debug, Deserialize)]
pub struct Fixture {
  pub intent: FixtureIntent,
  #[serde(default)]
  pub repo_ref: Option<String>,
  #[serde(default)]
  pub diff: Option<String>,
  #[serde(default)]
  pub plan: Option<String>,
  pub expectations: Expectations,
}

#[derive(Debug, Deserialize)]
pub struct FixtureIntent {
  pub title: String,
  pub body: String,
}

#[derive(Debug, Default, Deserialize)]
pub struct Expectations {
  #[serde(default)]
  pub relevant_files_contain: Vec<String>,
  #[serde(default)]
  pub plan_mentions: Vec<String>,
  #[serde(default)]
  pub steps_mention: Vec<String>,
  #[serde(default)]
  pub has_implementation_steps: Option<bool>,
  #[serde(default)]
  pub complexity_is_one_of: Vec<String>,
  #[serde(default)]
  pub min_relevant_files: Option<usize>,
  #[serde(default)]
  pub should_approve: Option<bool>,
}

#[derive(Debug)]
pub struct EvalResult {
  pub fixture_name: String,
  pub checks: Vec<Check>,
}

#[derive(Debug)]
pub struct Check {
  pub name: String,
  pub passed: bool,
  pub detail: String,
}

impl EvalResult {
  pub fn all_passed(&self) -> bool {
    self.checks.iter().all(|c| c.passed)
  }
}

pub fn load_fixtures(fixtures_dir: &Path) -> Result<Vec<(String, Fixture)>> {
  if !fixtures_dir.exists() {
    return Ok(Vec::new());
  }

  let mut fixtures = Vec::new();
  for entry in std::fs::read_dir(fixtures_dir)? {
    let entry = entry?;
    let path = entry.path();
    if path.extension().and_then(|e| e.to_str()) != Some("yaml") {
      continue;
    }
    let name = path
      .file_stem()
      .and_then(|s| s.to_str())
      .unwrap_or_default()
      .to_string();
    let content = std::fs::read_to_string(&path)?;
    let fixture: Fixture = serde_yaml::from_str(&content)?;
    fixtures.push((name, fixture));
  }
  fixtures.sort_by(|a, b| a.0.cmp(&b.0));
  Ok(fixtures)
}

pub fn eval_analyze(
  fixture_name: &str,
  fixture: &Fixture,
  config: &Config,
  claude: &impl Claude,
  repo_path: &Path,
) -> Result<EvalResult> {
  let intent = Intent::synthetic(&fixture.intent.title, &fixture.intent.body);

  info!("eval analyze: running fixture '{fixture_name}'");
  let (outcome, _meta, _depends, _observations) =
    analyze::analyze(&intent, config, claude, repo_path, &[], None)?;

  let mut checks = Vec::new();

  match &outcome {
    AnalysisOutcome::Tasks(tasks) => {
      // Single task (legacy format) has fields directly
      if tasks.len() == 1 {
        let task = &tasks[0];
        check_relevant_files(&fixture.expectations, &task.relevant_files, &mut checks);
        check_plan_mentions(&fixture.expectations, &task.plan, &mut checks);
        check_steps_mention(
          &fixture.expectations,
          &task.implementation_steps,
          &mut checks,
        );
        check_implementation_steps(
          &fixture.expectations,
          &task.implementation_steps,
          &mut checks,
        );
        check_complexity(&fixture.expectations, &task.complexity, &mut checks);
      } else {
        // Multi-task: aggregate
        let all_files: Vec<String> = tasks
          .iter()
          .flat_map(|t| t.relevant_files.iter().cloned())
          .collect();
        let all_plans: String = tasks
          .iter()
          .map(|t| t.plan.as_str())
          .collect::<Vec<_>>()
          .join("\n");
        let all_steps: Vec<String> = tasks
          .iter()
          .flat_map(|t| t.implementation_steps.iter().cloned())
          .collect();
        check_relevant_files(&fixture.expectations, &all_files, &mut checks);
        check_plan_mentions(&fixture.expectations, &all_plans, &mut checks);
        check_steps_mention(&fixture.expectations, &all_steps, &mut checks);
        check_implementation_steps(&fixture.expectations, &all_steps, &mut checks);
      }
    }
    AnalysisOutcome::NeedsClarification { .. } => {
      checks.push(Check {
        name: "outcome_type".into(),
        passed: false,
        detail: "expected Tasks, got NeedsClarification".into(),
      });
    }
    AnalysisOutcome::ChildIntents(_) => {
      checks.push(Check {
        name: "outcome_type".into(),
        passed: false,
        detail: "expected Tasks, got ChildIntents".into(),
      });
    }
  }

  Ok(EvalResult {
    fixture_name: fixture_name.to_string(),
    checks,
  })
}

fn check_relevant_files(exp: &Expectations, files: &[String], checks: &mut Vec<Check>) {
  for pattern in &exp.relevant_files_contain {
    let found = files.iter().any(|f| f.contains(pattern.as_str()));
    checks.push(Check {
      name: format!("relevant_files_contain '{pattern}'"),
      passed: found,
      detail: if found {
        "found".into()
      } else {
        format!("not found in {:?}", files)
      },
    });
  }

  if let Some(min) = exp.min_relevant_files {
    checks.push(Check {
      name: format!("min_relevant_files >= {min}"),
      passed: files.len() >= min,
      detail: format!("got {}", files.len()),
    });
  }
}

fn check_plan_mentions(exp: &Expectations, plan: &str, checks: &mut Vec<Check>) {
  let plan_lower = plan.to_lowercase();
  for keyword in &exp.plan_mentions {
    let found = plan_lower.contains(&keyword.to_lowercase());
    checks.push(Check {
      name: format!("plan_mentions '{keyword}'"),
      passed: found,
      detail: if found {
        "found".into()
      } else {
        "not found in plan".into()
      },
    });
  }
}

fn check_steps_mention(exp: &Expectations, steps: &[String], checks: &mut Vec<Check>) {
  let joined = steps.join(" ").to_lowercase();
  for keyword in &exp.steps_mention {
    let found = joined.contains(&keyword.to_lowercase());
    checks.push(Check {
      name: format!("steps_mention '{keyword}'"),
      passed: found,
      detail: if found {
        "found".into()
      } else {
        "not found in implementation_steps".into()
      },
    });
  }
}

fn check_implementation_steps(exp: &Expectations, steps: &[String], checks: &mut Vec<Check>) {
  if let Some(true) = exp.has_implementation_steps {
    checks.push(Check {
      name: "has_implementation_steps".into(),
      passed: !steps.is_empty(),
      detail: format!("{} steps", steps.len()),
    });
  }
}

pub fn eval_review(
  fixture_name: &str,
  fixture: &Fixture,
  config: &Config,
  claude: &impl Claude,
  repo_path: &Path,
) -> Result<EvalResult> {
  let intent = Intent::synthetic(&fixture.intent.title, &fixture.intent.body);
  let plan = fixture
    .plan
    .as_deref()
    .unwrap_or("Implementation plan not specified");
  let diff = fixture.diff.as_deref().unwrap_or("(no diff provided)");

  let task = Task {
    id: "eval".into(),
    title: fixture.intent.title.clone(),
    intent_id: "eval".into(),
    status: crate::task::WorkStatus::Pending,
    complexity: "medium".into(),
    plan: plan.to_string(),
    relevant_files: vec![],
    implementation_steps: vec![],
    context: String::new(),
    depends_on: vec![],
  };

  info!("eval review: running fixture '{fixture_name}'");
  let (result, _meta) = review::review_with_diff(&intent, &task, config, claude, repo_path, diff)?;

  let mut checks = Vec::new();

  if let Some(expected) = fixture.expectations.should_approve {
    checks.push(Check {
      name: "should_approve".into(),
      passed: result.approved == expected,
      detail: format!("expected {expected}, got {}", result.approved),
    });
  }

  Ok(EvalResult {
    fixture_name: fixture_name.to_string(),
    checks,
  })
}

fn check_complexity(exp: &Expectations, complexity: &str, checks: &mut Vec<Check>) {
  if !exp.complexity_is_one_of.is_empty() {
    let found = exp.complexity_is_one_of.iter().any(|c| c == complexity);
    checks.push(Check {
      name: "complexity".into(),
      passed: found,
      detail: format!(
        "got '{complexity}', expected one of {:?}",
        exp.complexity_is_one_of
      ),
    });
  }
}
