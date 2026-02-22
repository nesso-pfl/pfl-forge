use std::path::PathBuf;

use clap::{Parser, Subcommand};
use tracing::{error, info};

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
      let claude = ClaudeRunner::new(config.worker_tools.clone());
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
      info!("watch: not yet implemented in new architecture");
      Ok(())
    }
    Commands::Status => {
      info!("status: not yet implemented in new architecture");
      Ok(())
    }
    Commands::Clean => {
      let repo_path = Config::repo_path();
      let worktrees = git::worktree::list(&repo_path)?;
      info!("{} worktree(s) found", worktrees.len());
      Ok(())
    }
    Commands::Parent { model } => agent::operator::launch(&config, model.as_deref()),
    Commands::Create { title, body } => {
      info!("create: {} - {}", title, body);
      info!("not yet implemented in new architecture");
      Ok(())
    }
    Commands::Audit { path } => {
      let repo_path = Config::repo_path();
      let claude = ClaudeRunner::new(config.analyze_tools.clone());

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
      info!("inbox: not yet implemented in new architecture");
      Ok(())
    }
    Commands::Approve { ids } => {
      info!("approve: {ids}");
      info!("not yet implemented in new architecture");
      Ok(())
    }
  }
}
