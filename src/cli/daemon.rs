use std::process::Command;

use clap::{Args, Subcommand};

use crate::config::resolve_daemon_config;
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
    Status,
    Abort(DaemonAbortArgs),
}

#[derive(Debug, Args)]
pub struct DaemonStartArgs {
    #[arg(long)]
    pub repo: Option<String>,
    #[arg(long, value_parser = super::parse_positive_u64)]
    pub poll_seconds: Option<u64>,
    #[arg(long, value_parser = super::parse_positive_u32)]
    pub max_concurrent: Option<u32>,
    #[arg(long = "label")]
    pub labels: Vec<String>,
}

#[derive(Debug, Args)]
pub struct DaemonAbortArgs {
    pub task_id_or_number: String,
}

pub fn execute(args: DaemonArgs) -> Result<()> {
    match args.command {
        DaemonCommand::Start(start_args) => execute_start(start_args),
        DaemonCommand::Status => execute_status(),
        DaemonCommand::Abort(abort_args) => execute_abort(abort_args),
    }
}

fn execute_start(args: DaemonStartArgs) -> Result<()> {
    let workspace = Workspace::discover()?;
    let daemon_cfg = effective_daemon_config(&workspace)?;

    let poll_seconds = args.poll_seconds.unwrap_or(daemon_cfg.poll_seconds);
    let max_concurrent = args.max_concurrent.unwrap_or(daemon_cfg.max_concurrent);
    let labels = if args.labels.is_empty() {
        daemon_cfg.labels
    } else {
        args.labels
    };

    let repo = match args.repo {
        Some(repo) => {
            validate_repo_slug(&repo)?;
            repo
        }
        None => match daemon_cfg.repo {
            Some(repo) => {
                validate_repo_slug(&repo)?;
                repo
            }
            None => resolve_repo_from_gh()?,
        },
    };

    println!(
        "daemon start validated for repo {} (poll={}s, max_concurrent={}, labels={}); polling loop is not implemented in this iteration",
        repo,
        poll_seconds,
        max_concurrent,
        labels.join(",")
    );
    Ok(())
}

fn execute_status() -> Result<()> {
    let workspace = Workspace::discover()?;
    let store = TaskStore::new(&workspace.root);
    let tasks = store.load()?;

    if tasks.is_empty() {
        println!("no daemon tasks");
        return Ok(());
    }

    println!("DAEMON TASKS");
    println!(
        "{:<36} {:<12} {:<8} {:<20} {:<8} {:<8}",
        "TASK ID", "STATE", "ISSUE", "REPO", "PID", "PGID"
    );
    for task in tasks {
        println!(
            "{:<36} {:<12} {:<8} {:<20} {:<8} {:<8}",
            task.task_id,
            task.state,
            task.issue_number,
            format!("{}/{}", task.owner, task.repo),
            task.child_pid
                .map(|value| value.to_string())
                .unwrap_or_else(|| "-".to_owned()),
            task.child_pgid
                .map(|value| value.to_string())
                .unwrap_or_else(|| "-".to_owned())
        );
    }

    Ok(())
}

fn execute_abort(args: DaemonAbortArgs) -> Result<()> {
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

    Ok(resolve_daemon_config(&workspace.config, project_config.as_ref()))
}

fn validate_repo_slug(repo: &str) -> Result<()> {
    let trimmed = repo.trim();
    let mut parts = trimmed.split('/');

    let owner = parts.next().unwrap_or_default();
    let name = parts.next().unwrap_or_default();

    if owner.is_empty() || name.is_empty() || parts.next().is_some() {
        return Err(RalphError::Validation(format!(
            "invalid repo '{}': expected owner/repo",
            repo
        )));
    }

    Ok(())
}

fn resolve_repo_from_gh() -> Result<String> {
    let output = Command::new("gh")
        .args(["repo", "view", "--json", "nameWithOwner", "-q", ".nameWithOwner"])
        .output()
        .map_err(|err| {
            RalphError::Validation(format!(
                "could not resolve repo from gh; set --repo or workspace.daemon_repo: {}",
                err
            ))
        })?;

    if !output.status.success() {
        return Err(RalphError::Validation(format!(
            "could not resolve repo from gh; set --repo or workspace.daemon_repo: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }

    let repo = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    validate_repo_slug(&repo)?;
    Ok(repo)
}
