use std::path::PathBuf;

use clap::{Parser, Subcommand};
use tracing::{error, info, warn};

use pfl_forge::agent;
use pfl_forge::claude::runner::ClaudeRunner;
use pfl_forge::config::Config;
use pfl_forge::error::Result;
use pfl_forge::git;
use pfl_forge::runner;

#[derive(Parser)]
#[command(
  name = "pfl-forge",
  about = "Multi-agent task processor powered by Claude Code"
)]
struct Cli {
  #[command(subcommand)]
  command: Commands,

  /// Path to config file
  #[arg(short, long, default_value = "pfl-forge.yaml")]
  config: PathBuf,
}

#[derive(Subcommand)]
enum Commands {
  /// Process approved intents
  Run {
    /// Only analyze, don't execute
    #[arg(long)]
    dry_run: bool,
  },
  /// Watch for new intents and process them periodically
  Watch,
  /// Show current processing status
  Status,
  /// Clean up worktrees for completed tasks
  Clean,
  /// Launch operator agent (interactive Claude Code session)
  Parent {
    /// Claude model to use
    #[arg(long)]
    model: Option<String>,
  },
  /// Create a new intent draft in .forge/intent-drafts/
  Create {
    /// Intent title
    title: String,
    /// Intent body (description)
    body: String,
  },
  /// Run codebase audit
  Audit {
    /// Target path (default: entire codebase)
    path: Option<String>,
  },
  /// Show inbox (intents awaiting human action)
  Inbox,
  /// Approve intents by ID
  Approve {
    /// Comma-separated intent IDs
    ids: String,
  },
  /// Answer a clarification question on a blocked intent
  Answer {
    /// Intent ID
    id: String,
    /// Answer text
    answer: String,
  },
  /// Run prompt evaluation fixtures
  Eval {
    /// Agent to evaluate (analyze, review)
    agent: String,
    /// Specific fixture name (default: all)
    #[arg(long)]
    fixture: Option<String>,
  },
}

#[tokio::main]
async fn main() {
  tracing_subscriber::fmt()
    .with_env_filter(
      tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
    )
    .init();

  let cli = Cli::parse();

  if let Err(e) = run(cli).await {
    error!("{e}");
    std::process::exit(1);
  }
}

async fn run(cli: Cli) -> Result<()> {
  let config = Config::load(&cli.config)?;

  match cli.command {
    Commands::Run { dry_run } => {
      let repo_path = Config::repo_path();
      let claude = ClaudeRunner::new(config.implement_tools.clone(), config.mcp_config.clone());
      let results = runner::run_intents(&config, &claude, &repo_path, dry_run)?;
      for (id, result) in &results {
        let status = match &result.outcome {
          pfl_forge::knowledge::history::Outcome::Success => "success",
          pfl_forge::knowledge::history::Outcome::Failed => "failed",
          pfl_forge::knowledge::history::Outcome::Escalated => "escalated",
        };
        println!("{id}: {status}");
      }
      if results.is_empty() && !dry_run {
        println!("no approved intents to process");
      }
      Ok(())
    }
    Commands::Watch => {
      let repo_path = Config::repo_path();
      let claude = ClaudeRunner::new(config.implement_tools.clone(), config.mcp_config.clone());
      let interval = std::time::Duration::from_secs(config.poll_interval_secs);

      info!("watch: polling every {}s", config.poll_interval_secs);
      loop {
        match runner::run_intents(&config, &claude, &repo_path, false) {
          Ok(results) => {
            for (id, result) in &results {
              let status = match &result.outcome {
                pfl_forge::knowledge::history::Outcome::Success => "success",
                pfl_forge::knowledge::history::Outcome::Failed => "failed",
                pfl_forge::knowledge::history::Outcome::Escalated => "escalated",
              };
              info!("{id}: {status}");
            }
          }
          Err(e) => {
            warn!("watch cycle error: {e}");
          }
        }
        std::thread::sleep(interval);
      }
    }
    Commands::Status => {
      let repo_path = Config::repo_path();
      let intents_dir = repo_path.join(".forge").join("intents");
      let intents = pfl_forge::intent::registry::Intent::fetch_all(&intents_dir)?;

      if intents.is_empty() {
        println!("no intents");
        return Ok(());
      }

      for i in &intents {
        let status = format!("{:?}", i.status).to_lowercase();
        println!("{id}  {status}  {title}", id = i.id(), title = i.title);
      }
      println!("\n{} intent(s)", intents.len());
      Ok(())
    }
    Commands::Clean => {
      let repo_path = Config::repo_path();
      let intents_dir = repo_path.join(".forge").join("intents");
      let intents = pfl_forge::intent::registry::Intent::fetch_all(&intents_dir)?;
      let done_branches: Vec<String> = intents
        .iter()
        .filter(|i| matches!(i.status, pfl_forge::intent::registry::IntentStatus::Done))
        .map(|i| i.branch_name())
        .collect();

      if done_branches.is_empty() {
        println!("no completed worktrees to clean");
        return Ok(());
      }

      let mut cleaned = 0;
      for branch in &done_branches {
        let wt_path = git::worktree::path_for(&repo_path, &config.worktree_dir, branch);
        if wt_path.exists() {
          match git::worktree::remove(&repo_path, &wt_path) {
            Ok(()) => {
              println!("removed: {}", wt_path.display());
              cleaned += 1;
            }
            Err(e) => eprintln!("failed to remove {}: {e}", wt_path.display()),
          }
        }
      }
      println!("{cleaned} worktree(s) cleaned");
      Ok(())
    }
    Commands::Parent { model } => {
      let repo_path = Config::repo_path();
      agent::operator::launch(&config, model.as_deref(), &repo_path)
    }
    Commands::Create { title, body } => {
      let repo_path = Config::repo_path();
      let intents_dir = repo_path.join(".forge").join("intents");
      std::fs::create_dir_all(&intents_dir)?;

      let id = runner::slugify(&title);
      let path = intents_dir.join(format!("{id}.yaml"));
      if path.exists() {
        eprintln!("intent already exists: {id}");
        std::process::exit(1);
      }

      let yaml =
        format!("title: \"{title}\"\nbody: |\n  {body}\nsource: human\nstatus: proposed\n");
      std::fs::write(&path, yaml)?;
      println!("created: {id}");
      Ok(())
    }
    Commands::Audit { path } => {
      let repo_path = Config::repo_path();
      let claude = ClaudeRunner::new(config.analyze_tools.clone(), config.mcp_config.clone());

      // Create internal audit intent
      let target = path.as_deref().unwrap_or(".");
      let mut intent = runner::create_audit_intent(&repo_path, target)?;

      let result = runner::process_intent(&mut intent, &config, &claude, &repo_path)?;

      // Display observations
      let obs_path = repo_path.join(".forge").join("observations.yaml");
      let observations = pfl_forge::knowledge::observation::load(&obs_path)?;
      let audit_obs: Vec<_> = observations
        .iter()
        .filter(|o| o.intent_id == intent.id())
        .collect();

      if audit_obs.is_empty() {
        println!("no observations found");
      } else {
        println!("{} observation(s):", audit_obs.len());
        for obs in &audit_obs {
          println!("  - {}", obs.content);
          for ev in &obs.evidence {
            println!("    [{:?}] {}", ev.evidence_type, ev.reference);
          }
        }
      }

      let status = match &result.outcome {
        pfl_forge::knowledge::history::Outcome::Success => "success",
        pfl_forge::knowledge::history::Outcome::Failed => "failed",
        pfl_forge::knowledge::history::Outcome::Escalated => "escalated",
      };
      println!("audit: {status}");
      Ok(())
    }
    Commands::Inbox => {
      let repo_path = Config::repo_path();
      let intents_dir = repo_path.join(".forge").join("intents");
      let intents = pfl_forge::intent::registry::Intent::fetch_all(&intents_dir)?;

      let inbox: Vec<_> = intents
        .iter()
        .filter(|i| {
          matches!(
            i.status,
            pfl_forge::intent::registry::IntentStatus::Proposed
          ) || i.needs_clarification()
            || matches!(
              i.status,
              pfl_forge::intent::registry::IntentStatus::Blocked
                | pfl_forge::intent::registry::IntentStatus::Error
            )
        })
        .collect();

      if inbox.is_empty() {
        println!("inbox is empty");
      } else {
        for i in &inbox {
          let risk = i.risk.as_deref().unwrap_or("-");
          let source = &i.source;
          let status = format!("{:?}", i.status).to_lowercase();
          let clarification = if i.needs_clarification() {
            " [needs clarification]"
          } else {
            ""
          };
          println!(
            "{id}  {status}  risk={risk}  source={source}{clarification}",
            id = i.id(),
          );
          println!("  {}", i.title);
          // Show unanswered clarification questions
          for c in &i.clarifications {
            if c.answer.is_none() {
              println!("  Q: {}", c.question);
            }
          }
        }
        println!("\n{} item(s)", inbox.len());
      }
      Ok(())
    }
    Commands::Eval { agent, fixture } => {
      let repo_path = Config::repo_path();
      let evals_dir = repo_path.join("evals").join(&agent).join("fixtures");
      let fixtures = pfl_forge::eval::load_fixtures(&evals_dir)?;

      if fixtures.is_empty() {
        println!("no fixtures found in {}", evals_dir.display());
        return Ok(());
      }

      let claude = ClaudeRunner::new(config.analyze_tools.clone(), config.mcp_config.clone());
      let mut total = 0;
      let mut passed = 0;

      for (name, fix) in &fixtures {
        if let Some(ref f) = fixture {
          if name != f {
            continue;
          }
        }

        let result = match agent.as_str() {
          "analyze" => pfl_forge::eval::eval_analyze(name, fix, &config, &claude, &repo_path)?,
          "review" => pfl_forge::eval::eval_review(name, fix, &config, &claude, &repo_path)?,
          other => {
            eprintln!("eval not implemented for agent: {other}");
            return Ok(());
          }
        };

        let status = if result.all_passed() { "PASS" } else { "FAIL" };
        println!("{status} {name}");
        for check in &result.checks {
          let mark = if check.passed { "  +" } else { "  -" };
          println!("{mark} {} ({})", check.name, check.detail);
        }

        total += 1;
        if result.all_passed() {
          passed += 1;
        }
      }

      println!("\n{passed}/{total} fixtures passed");
      if passed < total {
        std::process::exit(1);
      }
      Ok(())
    }
    Commands::Answer { id, answer } => {
      let repo_path = Config::repo_path();
      let intents_dir = repo_path.join(".forge").join("intents");
      let intents = pfl_forge::intent::registry::Intent::fetch_all(&intents_dir)?;

      match intents.iter().find(|i| i.id() == id) {
        Some(intent) => {
          let mut updated = intent.clone();
          // Find the first unanswered clarification and fill it
          let unanswered = updated
            .clarifications
            .iter_mut()
            .find(|c| c.answer.is_none());
          match unanswered {
            Some(c) => {
              println!("Q: {}", c.question);
              println!("A: {answer}");
              c.answer = Some(answer);

              // If all clarifications are now answered, auto-approve
              if !updated.needs_clarification() {
                updated.status = pfl_forge::intent::registry::IntentStatus::Approved;
                println!("{id}: all clarifications answered, approved");
              } else {
                let remaining = updated
                  .clarifications
                  .iter()
                  .filter(|c| c.answer.is_none())
                  .count();
                println!("{id}: answered ({remaining} question(s) remaining)");
              }
              runner::update_intent_file(&repo_path, &updated)?;
            }
            None => {
              println!("{id}: no unanswered clarifications");
            }
          }
        }
        None => {
          eprintln!("{id}: not found");
        }
      }
      Ok(())
    }
    Commands::Approve { ids } => {
      let repo_path = Config::repo_path();
      let intents_dir = repo_path.join(".forge").join("intents");
      let intents = pfl_forge::intent::registry::Intent::fetch_all(&intents_dir)?;

      for raw_id in ids.split(',') {
        let id = raw_id.trim();
        if id.is_empty() {
          continue;
        }
        match intents.iter().find(|i| i.id() == id) {
          Some(intent) => {
            let mut updated = intent.clone();
            updated.status = pfl_forge::intent::registry::IntentStatus::Approved;
            runner::update_intent_file(&repo_path, &updated)?;
            println!("{id}: approved");
          }
          None => {
            eprintln!("{id}: not found");
          }
        }
      }
      Ok(())
    }
  }
}
