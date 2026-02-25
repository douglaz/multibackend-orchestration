mod auto;
pub(crate) mod backend_spec;
mod config;
mod daemon;
pub mod history;
pub mod init;
mod prd;
mod project;
mod quick_prd;
mod rollback;
mod run;
mod status;
pub mod tail;

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

use crate::config::PromptChangeAction;
use crate::validate;
use crate::Result;

#[derive(Debug, Parser)]
#[command(name = "ralph")]
#[command(about = "AI backend orchestration tool")]
#[command(version)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Debug, Subcommand)]
pub enum Commands {
    Init(InitArgs),
    Project(ProjectArgs),
    Run(RunArgs),
    Prd(prd::PrdArgs),
    QuickPrd(quick_prd::QuickPrdArgs),
    Auto(auto::AutoArgs),
    Validate(validate::ValidateArgs),
    Status(StatusArgs),
    History(HistoryArgs),
    Tail(TailArgs),
    Rollback(RollbackArgs),
    Config(ConfigArgs),
    Daemon(daemon::DaemonArgs),
}

#[derive(Debug, Args)]
pub struct InitArgs {
    #[arg(long, default_value = ".ralph")]
    pub dir: PathBuf,
    #[arg(short = 'n', long = "dry-run")]
    pub dry_run: bool,
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
    Delete(ProjectDeleteArgs),
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
pub struct ProjectDeleteArgs {
    pub project_id: String,
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
    #[arg(long = "planner-backend")]
    pub planner_backend: Option<String>,
    #[arg(long = "implementer-backend")]
    pub implementer_backend: Option<String>,
    #[arg(long = "reviewer-backend")]
    pub reviewer_backend: Option<String>,
    #[arg(long = "qa-backend")]
    pub qa_backend: Option<String>,
    #[arg(long = "completer-backend")]
    pub completer_backend: Option<String>,
    #[arg(long)]
    pub on_prompt_change: Option<PromptChangeAction>,
    #[arg(long)]
    pub skip_commit: bool,
    #[arg(long)]
    pub skip_prompt_review: bool,
    #[arg(
        long,
        num_args = 0..=1,
        default_missing_value = "true",
        value_parser = clap::value_parser!(bool),
        conflicts_with = "no_tmux"
    )]
    pub tmux: Option<bool>,
    #[arg(
        long = "no-tmux",
        num_args = 0..=1,
        default_missing_value = "false",
        value_parser = clap::value_parser!(bool),
        conflicts_with = "tmux"
    )]
    pub no_tmux: Option<bool>,
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

#[derive(Debug, Args, Clone)]
pub struct TailArgs {
    #[arg(long)]
    pub project: Option<String>,
    #[arg(short = 'n', long = "last", value_parser = parse_positive_usize)]
    pub last: Option<usize>,
    #[arg(short = 'F', long)]
    pub follow: bool,
    #[arg(long, default_value_t = 1000, value_parser = parse_positive_u64)]
    pub poll_interval_ms: u64,
    #[arg(long)]
    pub json: bool,
    /// Attach to the ralph tmux session instead of showing artifact events
    #[arg(long)]
    pub tmux: bool,
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

fn parse_positive_usize(value: &str) -> std::result::Result<usize, String> {
    let parsed = value
        .parse::<usize>()
        .map_err(|_| format!("invalid value '{value}': expected positive integer"))?;
    if parsed == 0 {
        return Err("must be greater than 0".to_owned());
    }
    Ok(parsed)
}

fn parse_positive_u64(value: &str) -> std::result::Result<u64, String> {
    let parsed = value
        .parse::<u64>()
        .map_err(|_| format!("invalid value '{value}': expected positive integer"))?;
    if parsed == 0 {
        return Err("must be greater than 0".to_owned());
    }
    Ok(parsed)
}

fn parse_positive_u32(value: &str) -> std::result::Result<u32, String> {
    let parsed = value
        .parse::<u32>()
        .map_err(|_| format!("invalid value '{value}': expected positive integer"))?;
    if parsed == 0 {
        return Err("must be greater than 0".to_owned());
    }
    Ok(parsed)
}

pub async fn run(cli: Cli) -> Result<()> {
    match cli.command {
        Commands::Init(args) => init::execute(args),
        Commands::Project(args) => project::execute(args),
        Commands::Run(args) => run::execute(args).await,
        Commands::Prd(args) => prd::execute(args).await,
        Commands::QuickPrd(args) => quick_prd::execute(args).await,
        Commands::Auto(args) => auto::execute(args).await,
        Commands::Validate(args) => validate::execute(args),
        Commands::Status(args) => status::execute(args),
        Commands::History(args) => history::execute(args),
        Commands::Tail(args) => tail::execute(args).await,
        Commands::Rollback(args) => rollback::execute(args),
        Commands::Config(args) => config::execute(args),
        Commands::Daemon(args) => daemon::execute(args).await,
    }
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::{Cli, Commands};

    #[test]
    fn parses_run_with_tmux_flag() {
        let cli = Cli::parse_from(["ralph", "run", "--tmux"]);
        let Commands::Run(args) = cli.command else {
            panic!("expected run command");
        };

        assert_eq!(args.tmux, Some(true));
        assert_eq!(args.no_tmux, None);
    }

    #[test]
    fn parses_run_with_no_tmux_flag() {
        let cli = Cli::parse_from(["ralph", "run", "--no-tmux"]);
        let Commands::Run(args) = cli.command else {
            panic!("expected run command");
        };

        assert_eq!(args.tmux, None);
        assert_eq!(args.no_tmux, Some(false));
    }

    #[test]
    fn parses_run_without_tmux_flags() {
        let cli = Cli::parse_from(["ralph", "run"]);
        let Commands::Run(args) = cli.command else {
            panic!("expected run command");
        };

        assert_eq!(args.tmux, None);
        assert_eq!(args.no_tmux, None);
    }

    #[test]
    fn rejects_run_with_conflicting_tmux_flags() {
        let result = Cli::try_parse_from(["ralph", "run", "--tmux", "--no-tmux"]);
        assert!(result.is_err());
    }

    #[test]
    fn parses_init_with_dry_run_long_flag() {
        let cli = Cli::parse_from(["ralph", "init", "--dry-run"]);
        let Commands::Init(args) = cli.command else {
            panic!("expected init command");
        };

        assert_eq!(args.dir, std::path::PathBuf::from(".ralph"));
        assert!(args.dry_run);
    }

    #[test]
    fn parses_init_with_dry_run_short_flag() {
        let cli = Cli::parse_from(["ralph", "init", "-n"]);
        let Commands::Init(args) = cli.command else {
            panic!("expected init command");
        };

        assert_eq!(args.dir, std::path::PathBuf::from(".ralph"));
        assert!(args.dry_run);
    }

    #[test]
    fn parses_run_with_role_backend_overrides() {
        let cli = Cli::parse_from([
            "ralph",
            "run",
            "--planner-backend",
            "claude(opus)",
            "--implementer-backend",
            "codex(gpt-5)",
            "--reviewer-backend",
            "claude",
            "--completer-backend",
            "codex",
        ]);
        let Commands::Run(args) = cli.command else {
            panic!("expected run command");
        };

        assert_eq!(args.planner_backend.as_deref(), Some("claude(opus)"));
        assert_eq!(args.implementer_backend.as_deref(), Some("codex(gpt-5)"));
        assert_eq!(args.reviewer_backend.as_deref(), Some("claude"));
        assert_eq!(args.completer_backend.as_deref(), Some("codex"));
    }

    #[test]
    fn parses_prd_command_with_expected_arguments() {
        let cli = Cli::parse_from([
            "ralph",
            "prd",
            "--idea",
            "smart onboarding",
            "--non-interactive",
            "--ask-max",
            "5",
            "--answers",
            "answers.yaml",
            "--resume",
            "--dry-run",
            "--backend",
            "claude(opus)",
        ]);
        let Commands::Prd(args) = cli.command else {
            panic!("expected prd command");
        };

        assert_eq!(args.idea, "smart onboarding");
        assert!(args.non_interactive);
        assert!(!args.interactive);
        assert_eq!(args.ask_max, Some(5));
        assert_eq!(args.preset, None);
        assert_eq!(
            args.answers.as_deref(),
            Some(std::path::Path::new("answers.yaml"))
        );
        assert!(args.resume);
        assert!(args.dry_run);
        assert_eq!(args.backend.as_deref(), Some("claude(opus)"));
    }

    #[test]
    fn parses_prd_command_with_implicit_defaults() {
        let cli = Cli::parse_from(["ralph", "prd", "--idea", "smart onboarding"]);
        let Commands::Prd(args) = cli.command else {
            panic!("expected prd command");
        };

        assert_eq!(args.ask_max, None);
        assert_eq!(args.preset, None);
    }

    #[test]
    fn parses_prd_command_with_preset() {
        let cli = Cli::parse_from([
            "ralph",
            "prd",
            "--idea",
            "smart onboarding",
            "--preset",
            "discuss",
        ]);
        let Commands::Prd(args) = cli.command else {
            panic!("expected prd command");
        };

        assert_eq!(args.preset, Some(super::prd::PrdPreset::Discuss));
        assert_eq!(args.ask_max, None);
    }

    #[test]
    fn rejects_prd_with_conflicting_interactive_flags() {
        let result = Cli::try_parse_from([
            "ralph",
            "prd",
            "--idea",
            "smart onboarding",
            "--interactive",
            "--non-interactive",
        ]);
        assert!(result.is_err());
    }

    #[test]
    fn parses_quick_prd_with_defaults() {
        let cli = Cli::parse_from(["ralph", "quick-prd", "--idea", "add retry logic"]);
        let Commands::QuickPrd(args) = cli.command else {
            panic!("expected quick-prd command");
        };

        assert_eq!(args.idea, "add retry logic");
        assert_eq!(args.writer_backend, "claude");
        assert_eq!(args.reviewer_backend, "codex");
        assert_eq!(args.max_revisions, 1);
        assert!(!args.non_interactive);
        assert!(!args.interactive);
        assert!(!args.dry_run);
    }

    #[test]
    fn parses_quick_prd_with_all_args() {
        let cli = Cli::parse_from([
            "ralph",
            "quick-prd",
            "--idea",
            "add retry logic to backend execute()",
            "--writer-backend",
            "claude(opus)",
            "--reviewer-backend",
            "codex(gpt-5)",
            "--max-revisions",
            "4",
            "--interactive",
            "--dry-run",
        ]);
        let Commands::QuickPrd(args) = cli.command else {
            panic!("expected quick-prd command");
        };

        assert_eq!(args.idea, "add retry logic to backend execute()");
        assert_eq!(args.writer_backend, "claude(opus)");
        assert_eq!(args.reviewer_backend, "codex(gpt-5)");
        assert_eq!(args.max_revisions, 4);
        assert!(!args.non_interactive);
        assert!(args.interactive);
        assert!(args.dry_run);
    }

    #[test]
    fn rejects_quick_prd_with_conflicting_interactive_flags() {
        let result = Cli::try_parse_from([
            "ralph",
            "quick-prd",
            "--idea",
            "add retry logic",
            "--interactive",
            "--non-interactive",
        ]);
        assert!(result.is_err());
    }

    #[test]
    fn test_quick_prd_dry_run_skips_backends() {
        let cli = Cli::parse_from(["ralph", "quick-prd", "--idea", "test", "--dry-run"]);
        let Commands::QuickPrd(args) = cli.command else {
            panic!("expected quick-prd command");
        };

        assert_eq!(args.idea, "test");
        assert!(args.dry_run);
    }

    #[test]
    fn parses_run_with_skip_prompt_review() {
        let cli = Cli::parse_from(["ralph", "run", "--skip-prompt-review"]);
        let Commands::Run(args) = cli.command else {
            panic!("expected run command");
        };

        assert!(args.skip_prompt_review);
    }

    #[test]
    fn parses_auto_with_skip_prompt_review() {
        let cli = Cli::parse_from(["ralph", "auto", "--idea", "test", "--skip-prompt-review"]);
        let Commands::Auto(args) = cli.command else {
            panic!("expected auto command");
        };

        assert!(args.skip_prompt_review);
    }

    #[test]
    fn parses_validate_command_with_expected_arguments() {
        let cli = Cli::parse_from([
            "ralph",
            "validate",
            "--bin",
            "/tmp/ralph-under-test",
            "--filter",
            "run::",
            "--list",
            "--verbose",
        ]);
        let Commands::Validate(args) = cli.command else {
            panic!("expected validate command");
        };

        assert_eq!(args.bin, std::path::PathBuf::from("/tmp/ralph-under-test"));
        assert_eq!(args.filter.as_deref(), Some("run::"));
        assert!(args.list);
        assert!(args.verbose);
    }

    #[test]
    fn parses_daemon_start_with_all_overrides() {
        let cli = Cli::parse_from([
            "ralph",
            "daemon",
            "start",
            "--data-dir",
            "/tmp/test",
            "--repo",
            "acme/widgets",
            "--git-bin",
            "/opt/tools/git",
            "--gh-bin",
            "/opt/tools/gh",
            "--poll-seconds",
            "30",
            "--max-concurrent",
            "2",
            "--label",
            "ralph:ready",
            "--label",
            "bug",
        ]);
        let Commands::Daemon(args) = cli.command else {
            panic!("expected daemon command");
        };
        let super::daemon::DaemonCommand::Start(start_args) = args.command else {
            panic!("expected daemon start command");
        };

        assert_eq!(start_args.data_dir, std::path::PathBuf::from("/tmp/test"));
        assert_eq!(start_args.repo, vec!["acme/widgets".to_owned()]);
        assert_eq!(
            start_args.git_bin,
            Some(std::path::PathBuf::from("/opt/tools/git"))
        );
        assert_eq!(
            start_args.gh_bin,
            Some(std::path::PathBuf::from("/opt/tools/gh"))
        );
        assert_eq!(start_args.poll_seconds, Some(30));
        assert_eq!(start_args.max_concurrent, Some(2));
        assert_eq!(
            start_args.labels,
            vec!["ralph:ready".to_owned(), "bug".to_owned()]
        );
    }

    #[test]
    fn parses_daemon_start_with_max_concurrent_one() {
        let cli = Cli::parse_from([
            "ralph",
            "daemon",
            "start",
            "--data-dir",
            "/tmp/test",
            "--repo",
            "acme/widgets",
            "--max-concurrent",
            "1",
        ]);
        let Commands::Daemon(args) = cli.command else {
            panic!("expected daemon command");
        };
        let super::daemon::DaemonCommand::Start(start_args) = args.command else {
            panic!("expected daemon start command");
        };

        assert_eq!(start_args.data_dir, std::path::PathBuf::from("/tmp/test"));
        assert_eq!(start_args.max_concurrent, Some(1));
    }

    #[test]
    fn parses_daemon_status_with_data_dir() {
        let cli = Cli::parse_from(["ralph", "daemon", "status", "--data-dir", "/tmp/test"]);
        let Commands::Daemon(args) = cli.command else {
            panic!("expected daemon command");
        };
        let super::daemon::DaemonCommand::Status(status_args) = args.command else {
            panic!("expected daemon status command");
        };

        assert_eq!(status_args.data_dir, std::path::PathBuf::from("/tmp/test"));
    }

    #[test]
    fn parses_daemon_abort_with_task_selector() {
        let cli = Cli::parse_from([
            "ralph",
            "daemon",
            "abort",
            "--data-dir",
            "/tmp/test",
            "acme-widgets-42",
        ]);
        let Commands::Daemon(args) = cli.command else {
            panic!("expected daemon command");
        };
        let super::daemon::DaemonCommand::Abort(abort_args) = args.command else {
            panic!("expected daemon abort command");
        };

        assert_eq!(abort_args.data_dir, std::path::PathBuf::from("/tmp/test"));
        assert_eq!(abort_args.issue_number, "acme-widgets-42");
    }

    #[test]
    fn parses_daemon_retrigger_with_task_id() {
        let cli = Cli::parse_from(["ralph", "daemon", "retrigger", "acme-widgets-42"]);
        let Commands::Daemon(args) = cli.command else {
            panic!("expected daemon command");
        };
        let super::daemon::DaemonCommand::Retrigger(retrigger_args) = args.command else {
            panic!("expected daemon retrigger command");
        };

        assert_eq!(retrigger_args.issue_number, "acme-widgets-42");
    }
}
