mod config;
mod history;
pub mod init;
mod project;
mod rollback;
mod run;
mod status;

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

use crate::config::PromptChangeAction;
use crate::Result;

#[derive(Debug, Parser)]
#[command(name = "ralph")]
#[command(about = "AI backend orchestration tool")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Debug, Subcommand)]
pub enum Commands {
    Init(InitArgs),
    Project(ProjectArgs),
    Run(RunArgs),
    Status(StatusArgs),
    History(HistoryArgs),
    Rollback(RollbackArgs),
    Config(ConfigArgs),
}

#[derive(Debug, Args)]
pub struct InitArgs {
    #[arg(long, default_value = ".ralph")]
    pub dir: PathBuf,
}

#[derive(Debug, Args)]
pub struct ProjectArgs {
    #[command(subcommand)]
    pub command: ProjectCommand,
}

#[derive(Debug, Subcommand)]
pub enum ProjectCommand {
    New(ProjectNewArgs),
    List,
    Use(ProjectUseArgs),
    Show(ProjectShowArgs),
}

#[derive(Debug, Args)]
pub struct ProjectNewArgs {
    #[arg(long)]
    pub id: String,
    #[arg(long)]
    pub name: String,
    #[arg(long)]
    pub prompt: Option<PathBuf>,
    #[arg(long)]
    pub from: Option<String>,
    #[arg(long)]
    pub backend: Option<String>,
}

#[derive(Debug, Args)]
pub struct ProjectUseArgs {
    pub project_id: String,
}

#[derive(Debug, Args)]
pub struct ProjectShowArgs {
    pub project_id: Option<String>,
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct RunArgs {
    #[arg(long)]
    pub project: Option<String>,
    #[arg(long)]
    pub loops: Option<u32>,
    #[arg(long)]
    pub until_review: bool,
    #[arg(long)]
    pub until_complete: bool,
    #[arg(long)]
    pub dry_run: bool,
    #[arg(long)]
    pub backend: Option<String>,
    #[arg(long)]
    pub on_prompt_change: Option<PromptChangeAction>,
    #[arg(long)]
    pub skip_commit: bool,
}

#[derive(Debug, Args)]
pub struct StatusArgs {
    #[arg(long)]
    pub project: Option<String>,
}

#[derive(Debug, Args)]
pub struct HistoryArgs {
    #[arg(long)]
    pub project: Option<String>,
    #[arg(long)]
    pub verbose: bool,
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct RollbackArgs {
    pub loop_number: u32,
    #[arg(long)]
    pub project: Option<String>,
    #[arg(long)]
    pub hard: bool,
    #[arg(long)]
    pub dry_run: bool,
}

#[derive(Debug, Args)]
pub struct ConfigArgs {
    #[command(subcommand)]
    pub command: ConfigCommand,
}

#[derive(Debug, Subcommand)]
pub enum ConfigCommand {
    Show(ConfigShowArgs),
    Get(ConfigGetArgs),
    Set(ConfigSetArgs),
    Edit(ConfigEditArgs),
}

#[derive(Debug, Args)]
pub struct ConfigGetArgs {
    pub key: String,
    #[command(flatten)]
    pub scope: ConfigScopeArgs,
}

#[derive(Debug, Args)]
pub struct ConfigSetArgs {
    pub key: String,
    pub value: String,
    #[command(flatten)]
    pub scope: ConfigScopeArgs,
}

#[derive(Debug, Args)]
pub struct ConfigShowArgs {
    #[command(flatten)]
    pub scope: ConfigScopeArgs,
}

#[derive(Debug, Args)]
pub struct ConfigEditArgs {
    #[command(flatten)]
    pub scope: ConfigScopeArgs,
}

#[derive(Debug, Args, Clone)]
pub struct ConfigScopeArgs {
    #[arg(long, conflicts_with = "project")]
    pub global: bool,
    #[arg(long)]
    pub project: Option<String>,
}

pub async fn run(cli: Cli) -> Result<()> {
    match cli.command {
        Commands::Init(args) => init::execute(args),
        Commands::Project(args) => project::execute(args),
        Commands::Run(args) => run::execute(args).await,
        Commands::Status(args) => status::execute(args),
        Commands::History(args) => history::execute(args),
        Commands::Rollback(args) => rollback::execute(args),
        Commands::Config(args) => config::execute(args),
    }
}
