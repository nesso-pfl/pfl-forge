use std::path::Path;
use std::time::Instant;

use tracing::{info, warn};

use crate::agent::review::ReviewResult;
use crate::agent::{analyze, implement, reflect, review};
use crate::claude::runner::Claude;
use crate::config::Config;
use crate::error::Result;
use crate::git;
use crate::intent::registry::{Intent, IntentStatus};
use crate::knowledge::history::{self, HistoryEntry, Outcome, StepResult};
use crate::task::{self, Task, WorkStatus};

#[derive(Debug, Clone, PartialEq)]
pub enum Step {
  Analyze,
  Implement,
  Rebase,
  Review,
  Audit,
  Report,
}

impl Step {
  fn name(&self) -> &'static str {
    match self {
      Step::Analyze => "analyze",
      Step::Implement => "implement",
      Step::Rebase => "rebase",
      Step::Review => "review",
      Step::Audit => "audit",
      Step::Report => "report",
    }
  }
}

pub fn default_flow(intent_type: Option<&str>) -> Vec<Step> {
  match intent_type {
    Some("audit") => vec![Step::Audit, Step::Report],
    _ => vec![Step::Analyze, Step::Implement, Step::Review],
  }
}

#[derive(Debug)]
pub struct IntentResult {
  pub flow: Vec<String>,
  pub step_results: Vec<StepResult>,
  pub outcome: Outcome,
  pub failure_reason: Option<String>,
}

pub fn process_intent(
  intent: &mut Intent,
  config: &Config,
  claude: &impl Claude,
  repo_path: &Path,
) -> Result<IntentResult> {
  let flow = default_flow(intent.intent_type.as_deref());
  let flow_names: Vec<String> = flow.iter().map(|s| s.name().to_string()).collect();

  info!("processing intent {}: flow={:?}", intent, flow_names);
  intent.status = IntentStatus::Implementing;
  update_intent_file(repo_path, intent)?;

  let mut step_results = Vec::new();

  // Analyze
  let start = Instant::now();
  let analysis = analyze::analyze(intent, config, claude, repo_path)?;
  step_results.push(StepResult {
    step: "analyze".into(),
    duration_secs: start.elapsed().as_secs(),
  });

  // Create task from analysis
  let mut task = Task::from_analysis(intent, &analysis);

  // Worktree setup
  let worktree_path = git::worktree::create(
    repo_path,
    &config.worktree_dir,
    &intent.branch_name(),
    &config.base_branch,
  )?;
  git::worktree::ensure_gitignore_forge(&worktree_path)?;
  task::write_task_yaml(&worktree_path, &task)?;

  // Implement + Review cycle
  let selected_model = task.complexity().select_model(&config.models);
  let timeout = std::time::Duration::from_secs(config.worker_timeout_secs);

  let task_outcome = run_implement_review_cycle(
    intent,
    &mut task,
    config,
    claude,
    repo_path,
    &worktree_path,
    selected_model,
    timeout,
    &mut step_results,
  );

  // Determine intent status from task outcome
  let (outcome, failure_reason) = match task_outcome {
    TaskOutcome::Done => {
      intent.status = IntentStatus::Done;
      (Outcome::Success, None)
    }
    TaskOutcome::Failed(reason) => {
      intent.status = IntentStatus::Error;
      (Outcome::Failed, Some(reason))
    }
    TaskOutcome::Blocked(reason) => {
      intent.status = IntentStatus::Blocked;
      (Outcome::Failed, Some(reason))
    }
    TaskOutcome::Escalated(reason) => {
      intent.status = IntentStatus::Error;
      (Outcome::Escalated, Some(reason))
    }
  };

  update_intent_file(repo_path, intent)?;

  // Record history
  let entry = HistoryEntry {
    intent_id: intent.id().to_string(),
    intent_type: intent.intent_type.clone(),
    intent_risk: intent.risk.clone(),
    title: intent.title.clone(),
    flow: flow_names.clone(),
    step_results: step_results.clone(),
    outcome: outcome.clone(),
    failure_reason: failure_reason.clone(),
    observations: vec![],
    created_at: Some(chrono::Utc::now().to_rfc3339()),
  };
  if let Err(e) = history::write(repo_path, &entry) {
    warn!("failed to write history: {e}");
  }

  // Reflect: run after successful leaf intent completion
  if outcome == Outcome::Success && !has_children(repo_path, intent.id()) {
    let start = Instant::now();
    match reflect::reflect(intent, config, claude, repo_path) {
      Ok(r) => info!("reflect: generated {} intents", r.intents.len()),
      Err(e) => warn!("reflect failed: {e}"),
    }
    step_results.push(StepResult {
      step: "reflect".into(),
      duration_secs: start.elapsed().as_secs(),
    });
  }

  Ok(IntentResult {
    flow: flow_names,
    step_results,
    outcome,
    failure_reason,
  })
}

#[allow(dead_code)] // Blocked used when multi-task support is added
enum TaskOutcome {
  Done,
  Failed(String),
  Blocked(String),
  Escalated(String),
}

fn run_implement_review_cycle(
  intent: &Intent,
  task: &mut Task,
  config: &Config,
  claude: &impl Claude,
  repo_path: &Path,
  worktree_path: &Path,
  selected_model: &str,
  timeout: std::time::Duration,
  step_results: &mut Vec<StepResult>,
) -> TaskOutcome {
  let mut review_feedback: Option<ReviewResult> = None;
  let max_retries = config.max_review_retries;

  for attempt in 0..=max_retries {
    // Implement
    task.status = WorkStatus::Implementing;
    let start = Instant::now();
    let impl_result = implement::run(
      intent,
      claude,
      selected_model,
      worktree_path,
      Some(timeout),
      review_feedback.as_ref(),
    );
    step_results.push(StepResult {
      step: "implement".into(),
      duration_secs: start.elapsed().as_secs(),
    });

    if let Err(e) = impl_result {
      task.status = WorkStatus::Failed;
      return TaskOutcome::Failed(format!("implement failed: {e}"));
    }

    // Rebase
    let start = Instant::now();
    let rebase_ok =
      git::branch::try_rebase(worktree_path, &config.base_branch, intent.id()).unwrap_or(false);
    step_results.push(StepResult {
      step: "rebase".into(),
      duration_secs: start.elapsed().as_secs(),
    });

    if !rebase_ok {
      info!(
        "rebase conflict for {}, attempting reimplementation",
        intent.id()
      );
      // Remove worktree, delete old branch, and recreate from updated main
      if let Err(e) = git::worktree::remove(repo_path, worktree_path) {
        warn!("failed to remove worktree: {e}");
      }
      let _ = git::branch::delete(repo_path, &intent.branch_name());
      let new_wt = match git::worktree::create(
        repo_path,
        &config.worktree_dir,
        &intent.branch_name(),
        &config.base_branch,
      ) {
        Ok(p) => p,
        Err(e) => return TaskOutcome::Escalated(format!("worktree recreation failed: {e}")),
      };
      let _ = git::worktree::ensure_gitignore_forge(&new_wt);
      let _ = task::write_task_yaml(&new_wt, task);

      // Reimplementation attempt
      let start = Instant::now();
      let reimpl = implement::run(intent, claude, selected_model, &new_wt, Some(timeout), None);
      step_results.push(StepResult {
        step: "implement".into(),
        duration_secs: start.elapsed().as_secs(),
      });

      if reimpl.is_err() {
        task.status = WorkStatus::Failed;
        return TaskOutcome::Escalated("reimplementation failed after rebase conflict".into());
      }

      // Rebase again after reimplementation
      let start = Instant::now();
      let rebase_ok2 =
        git::branch::try_rebase(&new_wt, &config.base_branch, intent.id()).unwrap_or(false);
      step_results.push(StepResult {
        step: "rebase".into(),
        duration_secs: start.elapsed().as_secs(),
      });

      if !rebase_ok2 {
        task.status = WorkStatus::Failed;
        return TaskOutcome::Escalated("rebase conflict persists after reimplementation".into());
      }
    }

    // Review
    let start = Instant::now();
    let review_result = review::review(
      intent,
      task,
      config,
      claude,
      worktree_path,
      &config.base_branch,
    );
    step_results.push(StepResult {
      step: "review".into(),
      duration_secs: start.elapsed().as_secs(),
    });

    match review_result {
      Ok(result) if result.approved => {
        task.status = WorkStatus::Completed;
        return TaskOutcome::Done;
      }
      Ok(result) => {
        info!(
          "review rejected (attempt {}/{})",
          attempt + 1,
          max_retries + 1
        );
        if attempt < max_retries {
          review_feedback = Some(result);
          continue;
        }
        task.status = WorkStatus::Failed;
        return TaskOutcome::Failed("review rejected after max retries".into());
      }
      Err(e) => {
        task.status = WorkStatus::Failed;
        return TaskOutcome::Failed(format!("review failed: {e}"));
      }
    }
  }

  unreachable!()
}

fn has_children(repo_path: &Path, intent_id: &str) -> bool {
  let intents_dir = repo_path.join(".forge").join("intents");
  Intent::fetch_all(&intents_dir)
    .unwrap_or_default()
    .iter()
    .any(|i| i.parent.as_deref() == Some(intent_id))
}

pub fn update_intent_file(repo_path: &Path, intent: &Intent) -> Result<()> {
  let intents_dir = repo_path.join(".forge").join("intents");
  let path = intents_dir.join(format!("{}.yaml", intent.id()));
  if !path.exists() {
    return Ok(());
  }
  let content = serde_yaml::to_string(intent)?;
  std::fs::write(&path, content)?;
  Ok(())
}
