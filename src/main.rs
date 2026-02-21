mod agent;
mod claude;
mod config;
mod error;
mod git;
mod intent;
mod knowledge;
mod prompt;
mod runner;
mod task;

use std::path::PathBuf;

use clap::{Parser, Subcommand};
use tracing::{error, info};

use crate::config::Config;
use crate::error::Result;

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
    Commands::Run { dry_run: _ } => {
      info!("run: not yet implemented in new architecture");
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
      info!("audit target: {:?}", path.as_deref().unwrap_or("."));
      info!("not yet implemented in new architecture");
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
