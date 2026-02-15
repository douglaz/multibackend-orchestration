use std::collections::HashSet;
use std::path::PathBuf;
use std::process::Command;

use clap::{Args, Subcommand};

use crate::config::resolve_daemon_config;
use crate::daemon::runtime::{spawn_blocking_op, DaemonRuntimeConfig};
use crate::daemon::{abort_task, TaskStore};
use crate::project::load_project_config_if_exists;
use crate::workspace::Workspace;
use crate::{error::RalphError, Result};

#[derive(Debug, Args)]
pub struct DaemonArgs {
    #[command(subcommand)]
    pub command: DaemonCommand,
}

#[derive(Debug, Subcommand)]
pub enum DaemonCommand {
    Start(DaemonStartArgs),
    Status(DaemonStatusArgs),
    Abort(DaemonAbortArgs),
}

#[derive(Debug, Args)]
pub struct DaemonStartArgs {
    #[arg(long)]
    pub data_dir: PathBuf,
    #[arg(long = "repo")]
    pub repo: Vec<String>,
    #[arg(long, value_parser = super::parse_positive_u64)]
    pub poll_seconds: Option<u64>,
    #[arg(long, value_parser = super::parse_positive_u32)]
    pub max_concurrent: Option<u32>,
    #[arg(long = "label")]
    pub labels: Vec<String>,
    /// Run a single poll/claim/dispatch/collect iteration and exit.
    /// Used by conformance tests for deterministic behavior.
    #[arg(long)]
    pub single_iteration: bool,
    #[arg(long)]
    pub verbose: bool,
}

#[derive(Debug, Args)]
pub struct DaemonStatusArgs {
    #[arg(long)]
    pub data_dir: PathBuf,
}

#[derive(Debug, Args)]
pub struct DaemonAbortArgs {
    #[arg(long)]
    pub data_dir: PathBuf,
    pub task_id_or_number: String,
}

pub async fn execute(args: DaemonArgs) -> Result<()> {
    match args.command {
        DaemonCommand::Start(start_args) => execute_start(start_args).await,
        DaemonCommand::Status(status_args) => {
            spawn_blocking_op(move || execute_status(status_args)).await
        }
        DaemonCommand::Abort(abort_args) => {
            spawn_blocking_op(move || execute_abort(abort_args)).await
        }
    }
}

async fn execute_start(args: DaemonStartArgs) -> Result<()> {
    if args.repo.is_empty() {
        return Err(RalphError::Validation(
            "at least one --repo owner/repo is required".to_owned(),
        ));
    }

    let mut normalized_repos = Vec::with_capacity(args.repo.len());
    let mut seen = HashSet::new();
    for repo in &args.repo {
        validate_repo_slug(repo)?;
        let normalized = repo.trim().to_ascii_lowercase();
        if !seen.insert(normalized.clone()) {
            return Err(RalphError::Validation(format!(
                "duplicate --repo: {}",
                normalized
            )));
        }
        normalized_repos.push(normalized);
    }

    let _ = &args.data_dir;

    preflight_check_gh()?;
    let workspace = Workspace::discover()?;
    let daemon_cfg = effective_daemon_config(&workspace)?;

    let poll_seconds = args.poll_seconds.unwrap_or(daemon_cfg.poll_seconds);
    let max_concurrent = args.max_concurrent.unwrap_or(daemon_cfg.max_concurrent);
    let labels = if args.labels.is_empty() {
        daemon_cfg.labels
    } else {
        args.labels
    };

    let repo = normalized_repos
        .first()
        .expect("repo list is non-empty after validation")
        .to_owned();

    let (owner, repo_name) = parse_repo_slug(&repo)?;

    println!(
        "daemon start validated for repo {} (poll={}s, max_concurrent={}, labels={})",
        repo,
        poll_seconds,
        max_concurrent,
        labels.join(",")
    );

    // Resolve ralph binary path (env override for testing, else current executable)
    let ralph_bin = match std::env::var("RALPH_DAEMON_BIN") {
        Ok(path) if !path.is_empty() => std::path::PathBuf::from(path),
        _ => std::env::current_exe().map_err(|err| {
            RalphError::Orchestration(format!("cannot determine ralph binary path: {err}"))
        })?,
    };

    // Resolve git repo root for worktree operations
    let repo_root = resolve_git_root(&workspace)?;

    let store = TaskStore::new(&workspace.root);
    let runtime_config = DaemonRuntimeConfig {
        owner,
        repo: repo_name,
        poll_seconds,
        max_concurrent,
        labels,
        single_iteration: args.single_iteration,
        verbose: args.verbose,
        ralph_bin,
        repo_root,
        refinement_enabled: daemon_cfg.refinement_enabled,
        refinement_backend: daemon_cfg.refinement_backend,
        global_config: workspace.config.clone(),
        auto_rebase_enabled: daemon_cfg.auto_rebase_enabled,
        rebase_interval_seconds: daemon_cfg.rebase_interval_seconds,
        max_rebases_per_cycle: daemon_cfg.max_rebases_per_cycle,
        rebase_timeout_seconds: daemon_cfg.rebase_timeout_seconds,
    };

    crate::daemon::runtime::run(&store, &runtime_config).await
}

fn execute_status(args: DaemonStatusArgs) -> Result<()> {
    let _ = args.data_dir;
    let workspace = Workspace::discover()?;
    let store = TaskStore::new(&workspace.root);
    let tasks = store.load()?;

    if tasks.is_empty() {
        println!("no daemon tasks");
        return Ok(());
    }

    println!("DAEMON TASKS");
    println!(
        "{:<36} {:<12} {:<8} {:<20} {:<8} {:<8} {:<20}",
        "TASK ID", "STATE", "ISSUE", "REPO", "PID", "PGID", "LAST REBASE"
    );
    for task in tasks {
        println!(
            "{:<36} {:<12} {:<8} {:<20} {:<8} {:<8} {:<20}",
            task.task_id,
            task.state,
            task.issue_number,
            format!("{}/{}", task.owner, task.repo),
            task.child_pid
                .map(|value| value.to_string())
                .unwrap_or_else(|| "-".to_owned()),
            task.child_pgid
                .map(|value| value.to_string())
                .unwrap_or_else(|| "-".to_owned()),
            task.last_rebase_at.unwrap_or_else(|| "-".to_owned())
        );
    }

    Ok(())
}

fn execute_abort(args: DaemonAbortArgs) -> Result<()> {
    let _ = args.data_dir;
    let workspace = Workspace::discover()?;
    let store = TaskStore::new(&workspace.root);
    let task = abort_task(&store, &args.task_id_or_number)?;

    println!("aborted task {}", task.task_id);
    Ok(())
}

fn effective_daemon_config(workspace: &Workspace) -> Result<crate::config::EffectiveDaemonConfig> {
    let project_config = match workspace.active_project_id() {
        Some(project_id) if workspace.project_exists(&project_id) => {
            load_project_config_if_exists(&workspace.project_dir(&project_id))?
        }
        Some(project_id) => {
            eprintln!(
                "warning: active project '{}' no longer exists; using workspace daemon config",
                project_id
            );
            None
        }
        None => None,
    };

    Ok(resolve_daemon_config(
        &workspace.config,
        project_config.as_ref(),
    ))
}

fn validate_repo_slug(repo: &str) -> Result<()> {
    let trimmed = repo.trim();
    let Some((owner, name)) = trimmed.split_once('/') else {
        return Err(RalphError::Validation(format!(
            "invalid repo '{}': expected owner/repo",
            repo
        )));
    };

    let invalid = owner.is_empty()
        || name.is_empty()
        || name.contains('/')
        || owner == "."
        || owner == ".."
        || name == "."
        || name == ".."
        || !is_valid_repo_component(owner)
        || !is_valid_repo_component(name);
    if invalid {
        return Err(RalphError::Validation(format!(
            "invalid repo '{}': expected owner/repo",
            repo
        )));
    }

    Ok(())
}

fn parse_repo_slug(repo: &str) -> Result<(String, String)> {
    validate_repo_slug(repo)?;
    let trimmed = repo.trim();
    let (owner, name) = trimmed.split_once('/').unwrap_or_default();
    Ok((owner.to_owned(), name.to_owned()))
}

fn is_valid_repo_component(component: &str) -> bool {
    component
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '.' || ch == '_' || ch == '-')
}

fn preflight_check_gh() -> Result<()> {
    match Command::new("gh").arg("--version").output() {
        Ok(_) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            Err(RalphError::Validation(
                "gh (GitHub CLI) not found in PATH. The daemon requires gh to poll issues, \
                 post comments, and create PRs. Install it from https://cli.github.com/ \
                 or run inside `nix develop`."
                    .to_owned(),
            ))
        }
        Err(err) => Err(RalphError::Validation(format!(
            "gh (GitHub CLI) check failed: {err}"
        ))),
    }
}

fn resolve_git_root(workspace: &Workspace) -> Result<std::path::PathBuf> {
    // The workspace root is typically inside .ralph/ under the repo root.
    // Walk up to find the git root.
    let output = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(&workspace.root)
        .output()
        .map_err(|err| RalphError::Orchestration(format!("failed to find git root: {err}")))?;

    if !output.status.success() {
        // Fallback: workspace root parent
        return Ok(workspace
            .root
            .parent()
            .unwrap_or(&workspace.root)
            .to_path_buf());
    }

    Ok(std::path::PathBuf::from(
        String::from_utf8_lossy(&output.stdout).trim(),
    ))
}
