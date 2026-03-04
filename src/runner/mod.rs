use std::path::Path;
use std::time::Instant;

use tracing::{info, warn};

use crate::agent::analyze::{ActiveIntentContext, AnalysisOutcome};
use crate::agent::review::ReviewResult;
use crate::agent::{analyze, audit, implement, reflect, review, skill};
use crate::claude::runner::{parse_metadata, Claude};
use crate::config::Config;
use crate::error::Result;
use crate::git;
use crate::intent::registry::{Intent, IntentStatus};
use crate::knowledge::history::{self, HistoryEntry, Outcome, StepResult};
use crate::knowledge::summary::{
  self, AnalyzeSummary, ExecutionSummary, ReviewSummary, TaskSummary,
};
use crate::task::{self, Task, WorkStatus};

#[derive(Debug, Clone, PartialEq)]
pub enum Step {
  Analyze,
  Implement,
  Rebase,
  Review,
  Audit,
  Report,
  Observe,
  Abstract,
  Record,
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
      Step::Observe => "observe",
      Step::Abstract => "abstract",
      Step::Record => "record",
    }
  }
}

pub fn default_flow(intent_type: Option<&str>) -> Vec<Step> {
  match intent_type {
    Some("audit") => vec![Step::Audit, Step::Report],
    Some("skill_extraction") => vec![Step::Observe, Step::Abstract, Step::Record],
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

pub fn run_intents(
  config: &Config,
  claude: &(impl Claude + Sync),
  repo_path: &Path,
  dry_run: bool,
) -> Result<Vec<(String, IntentResult)>> {
  // Convert any pending drafts before loading intents
  let converted = crate::intent::draft::convert_drafts(repo_path)?;
  if !converted.is_empty() {
    info!("converted {} draft(s): {:?}", converted.len(), converted);
  }

  let intents_dir = repo_path.join(".forge").join("intents");
  let all_intents = Intent::fetch_all(&intents_dir)?;
  let mut targets: Vec<Intent> = all_intents
    .iter()
    .filter(|i| i.status == IntentStatus::Approved || i.status == IntentStatus::Implementing)
    .filter(|i| {
      i.depends_on.is_empty()
        || i.depends_on.iter().all(|dep| {
          all_intents
            .iter()
            .any(|other| other.id() == dep && other.status == IntentStatus::Done)
        })
    })
    .cloned()
    .collect();

  if targets.is_empty() {
    info!("no approved intents found");
    return Ok(Vec::new());
  }

  if dry_run {
    for intent in &targets {
      info!("[dry-run] would process: {}", intent);
    }
    return Ok(Vec::new());
  }

  let batch_size = config.parallel_workers.max(1);
  let mut results = Vec::new();

  for batch in targets.chunks_mut(batch_size) {
    let batch_results: Vec<_> = std::thread::scope(|s| {
      let handles: Vec<_> = batch
        .iter_mut()
        .map(|intent| {
          s.spawn(|| {
            let id = intent.id().to_string();
            let result = process_intent(intent, config, claude, repo_path);
            (id, result)
          })
        })
        .collect();

      handles.into_iter().map(|h| h.join().unwrap()).collect()
    });

    for (id, result) in batch_results {
      match result {
        Ok(r) => {
          info!("{}: {:?}", id, r.outcome);
          results.push((id, r));
        }
        Err(e) => {
          warn!("{}: error: {e}", id);
        }
      }
    }
  }
  Ok(results)
}

pub fn process_intent(
  intent: &mut Intent,
  config: &Config,
  claude: &impl Claude,
  repo_path: &Path,
) -> Result<IntentResult> {
  let flow = default_flow(intent.intent_type.as_deref());
  let flow_names: Vec<String> = flow.iter().map(|s| s.name().to_string()).collect();
  let resuming = intent.status == IntentStatus::Implementing
    || (intent.status == IntentStatus::Approved && intent.last_step.is_some());

  info!(
    "processing intent {}: flow={:?} resuming={resuming}",
    intent, flow_names
  );
  intent.status = IntentStatus::Implementing;
  update_intent_file(repo_path, intent)?;

  if flow.contains(&Step::Audit) {
    return run_audit_report_flow(intent, config, claude, repo_path, flow_names);
  }

  if flow.contains(&Step::Observe) {
    return run_skill_extraction_flow(intent, config, claude, repo_path, flow_names);
  }

  let mut step_results = Vec::new();
  let mut exec_summary = ExecutionSummary {
    intent_id: intent.id().to_string(),
    ..Default::default()
  };

  // Check if we can resume from a previous run
  let worktree_path_for_resume =
    git::worktree::path_for(repo_path, &config.worktree_dir, &intent.branch_name());
  let has_tasks_yaml = worktree_path_for_resume
    .join(".forge")
    .join("tasks.yaml")
    .exists();
  let can_resume_tasks =
    resuming && intent.last_step.as_deref() == Some("analyze") && has_tasks_yaml;
  // Resume analyze with clarification answers (session_id present, no tasks yet)
  let resume_clarification = resuming
    && intent.last_step.as_deref() == Some("analyze")
    && !has_tasks_yaml
    && intent.sessions.analyze.is_some()
    && !intent.needs_clarification();

  let (mut tasks, worktree_path) = if can_resume_tasks {
    // Resume: read tasks from worktree, skip analyze
    info!("resuming from analyze: reading tasks from worktree");
    let tasks = task::read_all_tasks(&worktree_path_for_resume)?;
    (tasks, worktree_path_for_resume)
  } else {
    // Normal or clarification resume: run analyze
    if resuming && !resume_clarification {
      info!("resume data not found, running from start");
    }
    if resume_clarification {
      info!("resuming analyze with clarification answers");
    }

    // Gather active intent contexts for dependency detection
    let active_intents = gather_active_intents(repo_path, config, intent.id());

    // Analyze (with optional resume from clarification)
    let analyze_session_id = if resuming {
      intent.sessions.analyze.as_deref()
    } else {
      None
    };
    let start = Instant::now();
    let (analysis_outcome, analyze_meta, depends_on_intents, analyze_observations) =
      analyze::analyze(
        intent,
        config,
        claude,
        repo_path,
        &active_intents,
        analyze_session_id,
      )?;
    step_results.push(StepResult {
      step: "analyze".into(),
      duration_secs: start.elapsed().as_secs(),
      metadata: Some(analyze_meta.clone()),
    });

    if let Some(ref sid) = analyze_meta.session_id {
      intent.sessions.analyze = Some(sid.clone());
    }

    // Save cross-intent dependencies if detected
    if !depends_on_intents.is_empty() {
      intent.depends_on = depends_on_intents;
      intent.last_step = Some("analyze".into());
      update_intent_file(repo_path, intent)?;

      // Check if all dependencies are done
      let all_intents = Intent::fetch_all(&repo_path.join(".forge").join("intents"))?;
      let deps_satisfied = intent.depends_on.iter().all(|dep| {
        all_intents
          .iter()
          .any(|other| other.id() == dep && other.status == IntentStatus::Done)
      });

      if !deps_satisfied {
        info!(
          "intent {} waiting on depends_on: {:?}",
          intent.id(),
          intent.depends_on
        );
        // Revert to approved so it's picked up next run
        intent.status = IntentStatus::Approved;
        update_intent_file(repo_path, intent)?;
        return Ok(IntentResult {
          flow: flow_names,
          step_results,
          outcome: Outcome::Failed,
          failure_reason: Some("waiting on cross-intent dependencies".into()),
        });
      }
    }

    // Record analyze observations
    if !analyze_observations.is_empty() {
      let obs_path = repo_path.join(".forge").join("observations.yaml");
      for content in &analyze_observations {
        let obs = crate::knowledge::observation::Observation {
          content: content.clone(),
          evidence: vec![],
          source: "analyze".to_string(),
          intent_id: intent.id().to_string(),
          processed: false,
          created_at: Some(chrono::Utc::now().to_rfc3339()),
        };
        if let Err(e) = crate::knowledge::observation::append(&obs_path, &obs) {
          warn!("failed to write analyze observation: {e}");
        }
      }
      info!(
        "analyze: {} observations recorded",
        analyze_observations.len()
      );
    }

    // Handle non-task outcomes
    let task_specs = match analysis_outcome {
      AnalysisOutcome::Tasks(specs) => specs,
      AnalysisOutcome::NeedsClarification { clarifications } => {
        intent.status = IntentStatus::Blocked;
        if let Some(ref sid) = analyze_meta.session_id {
          intent.sessions.analyze = Some(sid.clone());
        }
        intent.last_step = Some("analyze".into());
        for q in &clarifications {
          intent
            .clarifications
            .push(crate::intent::registry::Clarification {
              question: q.clone(),
              answer: None,
            });
        }
        update_intent_file(repo_path, intent)?;
        return Ok(IntentResult {
          flow: flow_names,
          step_results,
          outcome: Outcome::Failed,
          failure_reason: Some("needs clarification".into()),
        });
      }
      AnalysisOutcome::ChildIntents(children) => {
        let intents_dir = repo_path.join(".forge").join("intents");
        for child in &children {
          let child_id = slugify(&child.title);
          let yaml = format!(
            "title: {title}\nbody: {body}\nsource: analyze\nparent: {parent}\n",
            title = child.title,
            body = child.body,
            parent = intent.id(),
          );
          std::fs::write(intents_dir.join(format!("{child_id}.yaml")), &yaml)?;
        }
        intent.status = IntentStatus::Done;
        update_intent_file(repo_path, intent)?;
        return Ok(IntentResult {
          flow: flow_names,
          step_results,
          outcome: Outcome::Success,
          failure_reason: None,
        });
      }
    };

    // Record analyze summary
    if let Some(first) = task_specs.first() {
      exec_summary.analyze = Some(AnalyzeSummary {
        complexity: first.complexity.clone(),
        plan: first.plan.clone(),
        relevant_files: task_specs
          .iter()
          .flat_map(|s| s.relevant_files.iter().cloned())
          .collect(),
        task_count: task_specs.len(),
      });
    }

    // Convert specs to tasks
    let tasks: Vec<Task> = task_specs
      .iter()
      .map(|spec| Task::from_spec(intent, spec))
      .collect();

    // Worktree setup (shared by all tasks)
    let worktree_path = git::worktree::create(
      repo_path,
      &config.worktree_dir,
      &intent.branch_name(),
      &config.base_branch,
    )?;
    git::worktree::ensure_gitignore_forge(&worktree_path)?;
    // Write first task yaml for backward compat
    if let Some(first) = tasks.first() {
      task::write_task_yaml(&worktree_path, first)?;
    }
    // Persist all tasks for resume
    task::write_all_tasks(&worktree_path, &tasks)?;
    run_worktree_setup(&worktree_path, &config.worktree_setup)?;

    intent.last_step = Some("analyze".into());
    update_intent_file(repo_path, intent)?;

    (tasks, worktree_path)
  };

  // Run tasks in dependency order
  let resume_session_id = if resuming {
    intent.sessions.implement.clone()
  } else {
    None
  };
  let timeout = std::time::Duration::from_secs(config.worker_timeout_secs);
  let task_outcomes = run_tasks_in_order(
    intent,
    &mut tasks,
    config,
    claude,
    repo_path,
    &worktree_path,
    timeout,
    &mut step_results,
    resume_session_id.as_deref(),
    &mut exec_summary,
  );

  // Aggregate task outcomes
  let (intent_status, outcome, failure_reason) = aggregate_task_outcomes(&tasks, &task_outcomes);

  intent.status = intent_status;
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

  // Write execution summary for Reflect
  if let Err(e) = summary::write(repo_path, &exec_summary) {
    warn!("failed to write execution summary: {e}");
  }

  // Reflect: run after successful leaf intent completion
  if outcome == Outcome::Success && !has_children(repo_path, intent.id()) {
    let start = Instant::now();
    let reflect_result = reflect::reflect(intent, config, claude, repo_path);
    let reflect_meta = reflect_result.as_ref().ok().map(|(_, m)| m.clone());
    match reflect_result {
      Ok((r, _)) => info!("reflect: generated {} intents", r.intents.len()),
      Err(e) => warn!("reflect failed: {e}"),
    }
    step_results.push(StepResult {
      step: "reflect".into(),
      duration_secs: start.elapsed().as_secs(),
      metadata: reflect_meta,
    });
  }

  Ok(IntentResult {
    flow: flow_names,
    step_results,
    outcome,
    failure_reason,
  })
}

#[derive(Clone)]
enum TaskOutcome {
  Done,
  Failed(String),
  #[allow(dead_code)]
  Blocked(String),
  Escalated(String),
}

fn run_tasks_in_order(
  intent: &mut Intent,
  tasks: &mut [Task],
  config: &Config,
  claude: &impl Claude,
  repo_path: &Path,
  worktree_path: &Path,
  timeout: std::time::Duration,
  step_results: &mut Vec<StepResult>,
  resume_session_id: Option<&str>,
  exec_summary: &mut ExecutionSummary,
) -> Vec<TaskOutcome> {
  let task_ids: Vec<String> = tasks.iter().map(|t| t.id.clone()).collect();
  let mut outcomes: Vec<Option<TaskOutcome>> = vec![None; tasks.len()];
  let mut done_ids: Vec<String> = Vec::new();
  let mut failed_ids: Vec<String> = Vec::new();

  loop {
    // Find next runnable task: pending, all depends_on satisfied
    let next = tasks.iter().position(|t| {
      t.status == WorkStatus::Pending
        && t
          .depends_on
          .iter()
          .all(|dep| done_ids.contains(dep) || !task_ids.contains(dep))
    });

    let Some(idx) = next else {
      // Check for tasks still pending but blocked by failed dependencies
      for (i, t) in tasks.iter_mut().enumerate() {
        if t.status == WorkStatus::Pending {
          let blocked_by_failure = t.depends_on.iter().any(|dep| failed_ids.contains(dep));
          if blocked_by_failure {
            t.status = WorkStatus::Failed;
            failed_ids.push(t.id.clone());
            outcomes[i] = Some(TaskOutcome::Failed("dependency failed".into()));
            info!("task {} skipped: dependency failed", t.id);
          }
        }
      }
      break;
    };

    let task = &mut tasks[idx];
    let selected_model = task.complexity().select_model(&config.models);

    // Write current task yaml for implement agent
    let _ = task::write_task_yaml(worktree_path, task);

    // Use resume session_id only for the first task
    let sid = if idx == 0 { resume_session_id } else { None };
    let (outcome, last_review) = run_implement_review_cycle(
      intent,
      task,
      config,
      claude,
      repo_path,
      worktree_path,
      selected_model,
      timeout,
      step_results,
      sid,
    );

    // Record task summary
    let commits =
      git::branch::commit_messages(worktree_path, &config.base_branch).unwrap_or_default();
    let review_summary = last_review.map(|r| ReviewSummary {
      approved: r.approved,
      issues: r.issues,
      suggestions: r.suggestions,
    });
    exec_summary.tasks.push(TaskSummary {
      task_id: task.id.clone(),
      commits,
      review: review_summary,
    });

    match &outcome {
      TaskOutcome::Done => {
        done_ids.push(task.id.clone());
      }
      TaskOutcome::Failed(_) | TaskOutcome::Blocked(_) | TaskOutcome::Escalated(_) => {
        failed_ids.push(task.id.clone());
      }
    }
    outcomes[idx] = Some(outcome);
  }

  outcomes.into_iter().flatten().collect()
}

fn aggregate_task_outcomes(
  tasks: &[Task],
  outcomes: &[TaskOutcome],
) -> (IntentStatus, Outcome, Option<String>) {
  let total = tasks.len();
  let done_count = tasks
    .iter()
    .filter(|t| t.status == WorkStatus::Completed)
    .count();
  let failed_count = tasks
    .iter()
    .filter(|t| t.status == WorkStatus::Failed)
    .count();

  if done_count == total {
    (IntentStatus::Done, Outcome::Success, None)
  } else if failed_count == total {
    // Preserve specific outcome from the first non-Done outcome
    let first_failure = outcomes.iter().find(|o| !matches!(o, TaskOutcome::Done));
    match first_failure {
      Some(TaskOutcome::Escalated(reason)) => (
        IntentStatus::Error,
        Outcome::Escalated,
        Some(reason.clone()),
      ),
      Some(TaskOutcome::Failed(reason)) => {
        (IntentStatus::Error, Outcome::Failed, Some(reason.clone()))
      }
      _ => (
        IntentStatus::Error,
        Outcome::Failed,
        Some("all tasks failed".into()),
      ),
    }
  } else {
    (
      IntentStatus::Blocked,
      Outcome::Failed,
      Some(format!(
        "{done_count}/{total} tasks done, {failed_count} failed"
      )),
    )
  }
}

fn run_implement_review_cycle(
  intent: &mut Intent,
  task: &mut Task,
  config: &Config,
  claude: &impl Claude,
  repo_path: &Path,
  worktree_path: &Path,
  selected_model: &str,
  timeout: std::time::Duration,
  step_results: &mut Vec<StepResult>,
  resume_session_id: Option<&str>,
) -> (TaskOutcome, Option<ReviewResult>) {
  let mut review_feedback: Option<ReviewResult> = None;
  let max_retries = config.max_review_retries;

  for attempt in 0..=max_retries {
    // Use resume session_id only on the first attempt
    let sid = if attempt == 0 {
      resume_session_id
    } else {
      None
    };

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
      sid,
    );
    let impl_meta = impl_result.as_ref().ok().map(|raw| parse_metadata(raw));
    if let Some(ref meta) = impl_meta {
      if let Some(ref sid) = meta.session_id {
        intent.sessions.implement = Some(sid.clone());
      }
    }
    step_results.push(StepResult {
      step: "implement".into(),
      duration_secs: start.elapsed().as_secs(),
      metadata: impl_meta,
    });

    if let Err(e) = impl_result {
      task.status = WorkStatus::Failed;
      return (TaskOutcome::Failed(format!("implement failed: {e}")), None);
    }

    // Read session_id from worktree if written by implement agent
    let session_id_path = worktree_path.join(".forge").join("session_id");
    if let Ok(sid) = std::fs::read_to_string(&session_id_path) {
      let sid = sid.trim().to_string();
      if !sid.is_empty() {
        intent.sessions.implement = Some(sid);
      }
    }
    intent.last_step = Some("implement".into());
    update_intent_file(repo_path, intent).ok();

    // Rebase
    let start = Instant::now();
    let rebase_ok =
      git::branch::try_rebase(worktree_path, &config.base_branch, intent.id()).unwrap_or(false);
    step_results.push(StepResult {
      step: "rebase".into(),
      duration_secs: start.elapsed().as_secs(),
      metadata: None,
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
        Err(e) => {
          return (
            TaskOutcome::Escalated(format!("worktree recreation failed: {e}")),
            None,
          )
        }
      };
      let _ = git::worktree::ensure_gitignore_forge(&new_wt);
      let _ = task::write_task_yaml(&new_wt, task);

      // Reimplementation attempt
      let start = Instant::now();
      let reimpl = implement::run(
        intent,
        claude,
        selected_model,
        &new_wt,
        Some(timeout),
        None,
        None,
      );
      let reimpl_meta = reimpl.as_ref().ok().map(|raw| parse_metadata(raw));
      step_results.push(StepResult {
        step: "implement".into(),
        duration_secs: start.elapsed().as_secs(),
        metadata: reimpl_meta,
      });

      if reimpl.is_err() {
        task.status = WorkStatus::Failed;
        return (
          TaskOutcome::Escalated("reimplementation failed after rebase conflict".into()),
          None,
        );
      }

      // Rebase again after reimplementation
      let start = Instant::now();
      let rebase_ok2 =
        git::branch::try_rebase(&new_wt, &config.base_branch, intent.id()).unwrap_or(false);
      step_results.push(StepResult {
        step: "rebase".into(),
        duration_secs: start.elapsed().as_secs(),
        metadata: None,
      });

      if !rebase_ok2 {
        task.status = WorkStatus::Failed;
        return (
          TaskOutcome::Escalated("rebase conflict persists after reimplementation".into()),
          None,
        );
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
    let review_meta = review_result.as_ref().ok().map(|(_, m)| m.clone());
    if let Some(ref meta) = review_meta {
      if let Some(ref sid) = meta.session_id {
        intent.sessions.review = Some(sid.clone());
      }
    }
    step_results.push(StepResult {
      step: "review".into(),
      duration_secs: start.elapsed().as_secs(),
      metadata: review_meta,
    });

    // Record review observations
    if let Ok((ref result, _)) = review_result {
      if !result.observations.is_empty() {
        let obs_path = repo_path.join(".forge").join("observations.yaml");
        for content in &result.observations {
          let obs = crate::knowledge::observation::Observation {
            content: content.clone(),
            evidence: vec![],
            source: "review".to_string(),
            intent_id: intent.id().to_string(),
            processed: false,
            created_at: Some(chrono::Utc::now().to_rfc3339()),
          };
          if let Err(e) = crate::knowledge::observation::append(&obs_path, &obs) {
            warn!("failed to write review observation: {e}");
          }
        }
        info!(
          "review: {} observations recorded",
          result.observations.len()
        );
      }
    }

    match review_result {
      Ok((result, _meta)) if result.approved => {
        task.status = WorkStatus::Completed;
        return (TaskOutcome::Done, Some(result));
      }
      Ok((result, _meta)) => {
        info!(
          "review rejected (attempt {}/{})",
          attempt + 1,
          max_retries + 1
        );
        if attempt < max_retries {
          review_feedback = Some(result);
          continue;
        }
        let last = Some(result);
        task.status = WorkStatus::Failed;
        return (
          TaskOutcome::Failed("review rejected after max retries".into()),
          last,
        );
      }
      Err(e) => {
        task.status = WorkStatus::Failed;
        return (TaskOutcome::Failed(format!("review failed: {e}")), None);
      }
    }
  }

  unreachable!()
}

fn run_audit_report_flow(
  intent: &mut Intent,
  config: &Config,
  claude: &impl Claude,
  repo_path: &Path,
  flow_names: Vec<String>,
) -> Result<IntentResult> {
  let mut step_results = Vec::new();

  // Audit: extract target path from intent body if specified
  let target_path = intent
    .body
    .strip_prefix("Audit the codebase at path: ")
    .map(|s| s.trim().to_string());
  let start = Instant::now();
  let audit_result = audit::audit(
    config,
    claude,
    repo_path,
    target_path.as_deref(),
    intent.id(),
  );
  let audit_meta = audit_result.as_ref().ok().map(|(_, m)| m.clone());
  step_results.push(StepResult {
    step: "audit".into(),
    duration_secs: start.elapsed().as_secs(),
    metadata: audit_meta,
  });

  let (outcome, failure_reason) = match audit_result {
    Ok((result, _meta)) => {
      // Report: read observations and output summary
      let start = Instant::now();
      let obs_path = repo_path.join(".forge").join("observations.yaml");
      let observations = crate::knowledge::observation::load(&obs_path).unwrap_or_default();
      info!("report: {} observations", observations.len());
      for obs in &observations {
        info!("  - {}", obs.content);
      }
      step_results.push(StepResult {
        step: "report".into(),
        duration_secs: start.elapsed().as_secs(),
        metadata: None,
      });

      intent.status = IntentStatus::Done;
      info!(
        "audit complete: {} observations found",
        result.observations.len()
      );
      (Outcome::Success, None)
    }
    Err(e) => {
      intent.status = IntentStatus::Error;
      (Outcome::Failed, Some(format!("audit failed: {e}")))
    }
  };

  update_intent_file(repo_path, intent)?;

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

  Ok(IntentResult {
    flow: flow_names,
    step_results,
    outcome,
    failure_reason,
  })
}

fn run_skill_extraction_flow(
  intent: &mut Intent,
  config: &Config,
  claude: &impl Claude,
  repo_path: &Path,
  flow_names: Vec<String>,
) -> Result<IntentResult> {
  let mut step_results = Vec::new();

  // Observe: analyze history to find patterns
  let start = Instant::now();
  let observe_result = skill::observe(config, claude, repo_path);
  let observe_meta = observe_result.as_ref().ok().map(|(_, m)| m.clone());
  step_results.push(StepResult {
    step: "observe".into(),
    duration_secs: start.elapsed().as_secs(),
    metadata: observe_meta,
  });

  let (outcome, failure_reason) = match observe_result {
    Ok((observe, _meta)) => {
      if observe.patterns.is_empty() {
        info!("skill extraction: no patterns found");
        intent.status = IntentStatus::Done;
        (Outcome::Success, None)
      } else {
        // Abstract: generalize patterns into skill templates
        let start = Instant::now();
        let abstract_result =
          skill::abstract_patterns(config, claude, repo_path, &observe.patterns);
        let abstract_meta = abstract_result.as_ref().ok().map(|(_, m)| m.clone());
        step_results.push(StepResult {
          step: "abstract".into(),
          duration_secs: start.elapsed().as_secs(),
          metadata: abstract_meta,
        });

        match abstract_result {
          Ok((abstract_out, _meta)) => {
            // Record: write skill drafts as SKILL.md files
            let start = Instant::now();
            let record_result = skill::record(repo_path, &abstract_out.skills);
            step_results.push(StepResult {
              step: "record".into(),
              duration_secs: start.elapsed().as_secs(),
              metadata: None,
            });

            match record_result {
              Ok(written) => {
                info!("skill extraction: wrote {} skills", written.len());
                intent.status = IntentStatus::Done;
                (Outcome::Success, None)
              }
              Err(e) => {
                intent.status = IntentStatus::Error;
                (Outcome::Failed, Some(format!("record failed: {e}")))
              }
            }
          }
          Err(e) => {
            intent.status = IntentStatus::Error;
            (Outcome::Failed, Some(format!("abstract failed: {e}")))
          }
        }
      }
    }
    Err(e) => {
      intent.status = IntentStatus::Error;
      (Outcome::Failed, Some(format!("observe failed: {e}")))
    }
  };

  update_intent_file(repo_path, intent)?;

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

  Ok(IntentResult {
    flow: flow_names,
    step_results,
    outcome,
    failure_reason,
  })
}

fn run_worktree_setup(worktree_path: &Path, commands: &[String]) -> Result<()> {
  for cmd in commands {
    info!("worktree setup: {cmd}");
    let output = std::process::Command::new("sh")
      .args(["-c", cmd])
      .current_dir(worktree_path)
      .output()?;
    if !output.status.success() {
      let stderr = String::from_utf8_lossy(&output.stderr);
      return Err(crate::error::ForgeError::Git(format!(
        "worktree setup command failed: {cmd}: {stderr}"
      )));
    }
  }
  Ok(())
}

fn has_children(repo_path: &Path, intent_id: &str) -> bool {
  let intents_dir = repo_path.join(".forge").join("intents");
  Intent::fetch_all(&intents_dir)
    .unwrap_or_default()
    .iter()
    .any(|i| i.parent.as_deref() == Some(intent_id))
}

pub fn slugify(s: &str) -> String {
  s.to_lowercase()
    .chars()
    .map(|c| if c.is_alphanumeric() { c } else { '-' })
    .collect::<String>()
    .split('-')
    .filter(|s| !s.is_empty())
    .collect::<Vec<_>>()
    .join("-")
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

fn gather_active_intents(
  repo_path: &Path,
  config: &Config,
  current_id: &str,
) -> Vec<ActiveIntentContext> {
  let intents_dir = repo_path.join(".forge").join("intents");
  let intents = Intent::fetch_all(&intents_dir).unwrap_or_default();

  intents
    .into_iter()
    .filter(|i| {
      i.id() != current_id
        && matches!(
          i.status,
          IntentStatus::Approved | IntentStatus::Implementing
        )
    })
    .map(|i| {
      let status = format!("{:?}", i.status).to_lowercase();
      // Try to read tasks from worktree for relevant_files and plan
      let wt_path = git::worktree::path_for(repo_path, &config.worktree_dir, &i.branch_name());
      let (relevant_files, plan) = task::read_all_tasks(&wt_path)
        .ok()
        .map(|tasks| {
          let files: Vec<String> = tasks
            .iter()
            .flat_map(|t| t.relevant_files.iter().cloned())
            .collect();
          let plan = tasks.first().map(|t| t.plan.clone());
          (files, plan)
        })
        .unwrap_or_default();

      ActiveIntentContext {
        id: i.id().to_string(),
        title: i.title.clone(),
        status,
        relevant_files,
        plan,
      }
    })
    .collect()
}

pub fn create_audit_intent(repo_path: &Path, target: &str) -> Result<Intent> {
  let timestamp = chrono::Utc::now().format("%Y%m%d-%H%M%S");
  let id = format!("audit-{timestamp}");
  let body = if target == "." {
    "Audit the entire codebase.".to_string()
  } else {
    format!("Audit the codebase at path: {target}")
  };

  let yaml =
    format!("title: Audit {target}\nbody: {body}\ntype: audit\nsource: human\nstatus: approved\n");

  let intents_dir = repo_path.join(".forge").join("intents");
  std::fs::create_dir_all(&intents_dir)?;
  std::fs::write(intents_dir.join(format!("{id}.yaml")), &yaml)?;

  // Read back to get a properly parsed Intent with file_stem set
  let intents = Intent::fetch_all(&intents_dir)?;
  intents
    .into_iter()
    .find(|i| i.id() == id)
    .ok_or_else(|| crate::error::ForgeError::Parse("failed to create audit intent".into()))
}
