use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Command;

use clap::{Args, Subcommand};

use crate::config::resolve_daemon_config;
use crate::daemon::bootstrap;
use crate::daemon::runtime::{spawn_blocking_op, DaemonRuntimeConfig};
use crate::daemon::{abort_task, resolve_task_index, TaskStore};
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

    // Guard: --data-dir must not be inside a git working tree (runs before
    // preflight_check_gh so it works even when gh is not installed)
    guard_not_git_repo(&args.data_dir)?;

    preflight_check_gh()?;

    // Create data-dir after guard passes
    std::fs::create_dir_all(&args.data_dir).map_err(|err| {
        RalphError::Orchestration(format!(
            "failed to create data-dir {}: {err}",
            args.data_dir.display()
        ))
    })?;

    // Resolve ralph binary path (env override for testing, else current executable)
    let ralph_bin = match std::env::var("RALPH_DAEMON_BIN") {
        Ok(path) if !path.is_empty() => PathBuf::from(path),
        _ => std::env::current_exe().map_err(|err| {
            RalphError::Orchestration(format!("cannot determine ralph binary path: {err}"))
        })?,
    };

    // Track whether we've already printed the deprecation warning
    let mut deprecation_warned = false;

    // Per-repo provisioning
    let mut repo_configs: Vec<(TaskStore, DaemonRuntimeConfig)> = Vec::new();

    for slug in &normalized_repos {
        let (owner, repo_name) = parse_repo_slug(slug)?;
        let repo_dir = args.data_dir.join(&owner).join(&repo_name);

        // Clone or bootstrap the repo
        clone_or_bootstrap(&owner, &repo_name, &repo_dir)?;

        // Load workspace from repo's .ralph/
        let workspace = Workspace::load(repo_dir.join(".ralph"))?;

        // Load project config if an active project exists
        let project_config = match workspace.active_project_id() {
            Some(project_id) if workspace.project_exists(&project_id) => {
                load_project_config_if_exists(&workspace.project_dir(&project_id))?
            }
            _ => None,
        };

        let daemon_cfg = resolve_daemon_config(&workspace.config, project_config.as_ref());

        // Deprecation warning for daemon.repo config key
        if !deprecation_warned && daemon_cfg.repo.is_some() {
            eprintln!(
                "warning: daemon.repo config key is ignored by `daemon start`; use --repo flag instead"
            );
            deprecation_warned = true;
        }

        let poll_seconds = args.poll_seconds.unwrap_or(daemon_cfg.poll_seconds);
        let max_concurrent = args.max_concurrent.unwrap_or(daemon_cfg.max_concurrent);
        let labels = if args.labels.is_empty() {
            daemon_cfg.labels
        } else {
            args.labels.clone()
        };

        println!(
            "daemon start validated for repo {}/{} (poll={}s, max_concurrent={}, labels={})",
            owner,
            repo_name,
            poll_seconds,
            max_concurrent,
            labels.join(",")
        );

        let store = TaskStore::new(&workspace.root);
        let runtime_config = DaemonRuntimeConfig {
            owner,
            repo: repo_name,
            poll_seconds,
            max_concurrent,
            labels,
            single_iteration: args.single_iteration,
            verbose: args.verbose,
            ralph_bin: ralph_bin.clone(),
            repo_root: repo_dir,
            refinement_enabled: daemon_cfg.refinement_enabled,
            refinement_backend: daemon_cfg.refinement_backend,
            global_config: workspace.config.clone(),
            auto_rebase_enabled: daemon_cfg.auto_rebase_enabled,
            rebase_interval_seconds: daemon_cfg.rebase_interval_seconds,
            max_rebases_per_cycle: daemon_cfg.max_rebases_per_cycle,
            rebase_timeout_seconds: daemon_cfg.rebase_timeout_seconds,
        };

        repo_configs.push((store, runtime_config));
    }

    // Run one runtime::run() per repo using JoinSet
    let mut join_set = tokio::task::JoinSet::new();

    for (store, config) in repo_configs {
        join_set.spawn(async move {
            crate::daemon::runtime::run(&store, &config).await
        });
    }

    // Wait for tasks; first error triggers abort_all
    while let Some(result) = join_set.join_next().await {
        match result {
            Err(join_err) => {
                join_set.abort_all();
                return Err(RalphError::Orchestration(format!(
                    "daemon runtime task panicked: {join_err}"
                )));
            }
            Ok(Err(err)) => {
                join_set.abort_all();
                return Err(err);
            }
            Ok(Ok(())) => {}
        }
    }

    Ok(())
}

fn execute_status(args: DaemonStatusArgs) -> Result<()> {
    let stores = scan_task_stores(&args.data_dir)?;

    if stores.is_empty() {
        println!("no daemon tasks");
        return Ok(());
    }

    let mut all_tasks = Vec::new();
    for store in &stores {
        let tasks = store.load()?;
        all_tasks.extend(tasks);
    }

    if all_tasks.is_empty() {
        println!("no daemon tasks");
        return Ok(());
    }

    println!("DAEMON TASKS");
    println!(
        "{:<36} {:<12} {:<8} {:<20} {:<8} {:<8} {:<20}",
        "TASK ID", "STATE", "ISSUE", "REPO", "PID", "PGID", "LAST REBASE"
    );
    for task in all_tasks {
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
    let stores = scan_task_stores(&args.data_dir)?;

    // Collect all tasks across all stores to handle ambiguity
    let mut all_tasks = Vec::new();
    let mut task_store_map: Vec<(usize, usize)> = Vec::new(); // (store_idx, task_idx_in_store)

    for (store_idx, store) in stores.iter().enumerate() {
        let tasks = store.load()?;
        for (task_idx, task) in tasks.iter().enumerate() {
            task_store_map.push((store_idx, task_idx));
            all_tasks.push(task.clone());
        }
    }

    // Use resolve_task_index on the combined list to get the index
    let index = resolve_task_index(&all_tasks, &args.task_id_or_number)?;
    let (store_idx, _) = task_store_map[index];
    let store = &stores[store_idx];

    let task = abort_task(store, &args.task_id_or_number)?;
    println!("aborted task {}", task.task_id);
    Ok(())
}

/// Scan `<data-dir>/*/*/.ralph/daemon/tasks.json` for task stores.
fn scan_task_stores(data_dir: &Path) -> Result<Vec<TaskStore>> {
    let mut stores = Vec::new();

    let owners = match std::fs::read_dir(data_dir) {
        Ok(entries) => entries,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(stores),
        Err(err) => return Err(err.into()),
    };

    for owner_entry in owners {
        let owner_entry = owner_entry?;
        if !owner_entry.file_type()?.is_dir() {
            continue;
        }

        let repos = match std::fs::read_dir(owner_entry.path()) {
            Ok(entries) => entries,
            Err(_) => continue,
        };

        for repo_entry in repos {
            let repo_entry = repo_entry?;
            if !repo_entry.file_type()?.is_dir() {
                continue;
            }

            let ralph_dir = repo_entry.path().join(".ralph");
            let tasks_path = ralph_dir.join("daemon").join("tasks.json");
            if tasks_path.exists() {
                stores.push(TaskStore::new(&ralph_dir));
            }
        }
    }

    Ok(stores)
}

/// Reject `--data-dir` paths inside a git working tree.
fn guard_not_git_repo(data_dir: &Path) -> Result<()> {
    // Walk up from data_dir to find the nearest existing ancestor
    let check_dir = nearest_existing_ancestor(data_dir);

    let output = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(&check_dir)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output();

    match output {
        Ok(out) if out.status.success() => Err(RalphError::Validation(
            "--data-dir must not be inside a git repository".to_owned(),
        )),
        _ => Ok(()),
    }
}

/// Walk up from `path` to find the nearest existing ancestor directory.
fn nearest_existing_ancestor(path: &Path) -> PathBuf {
    let mut current = path.to_path_buf();
    loop {
        if current.exists() {
            return current;
        }
        if !current.pop() {
            return PathBuf::from("/");
        }
    }
}

/// Clone a repo from GitHub or skip if already cloned, then bootstrap.
fn clone_or_bootstrap(owner: &str, repo: &str, repo_dir: &Path) -> Result<()> {
    if repo_dir.join(".git").exists() {
        // Already cloned — skip to bootstrap
        bootstrap::ensure_repo_ready_sync(repo_dir)?;
        return Ok(());
    }

    // Create parent directories
    if let Some(parent) = repo_dir.parent() {
        std::fs::create_dir_all(parent).map_err(|err| {
            RalphError::Orchestration(format!(
                "failed to create parent directory for {}: {err}",
                repo_dir.display()
            ))
        })?;
    }

    // Clone via gh
    let slug = format!("{owner}/{repo}");
    let repo_dir_str = repo_dir.to_string_lossy();
    let output = Command::new("gh")
        .args(["repo", "clone", &slug, &repo_dir_str])
        .output()
        .map_err(|err| {
            RalphError::Orchestration(format!("failed to run gh repo clone {slug}: {err}"))
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(RalphError::Orchestration(format!(
            "gh repo clone {slug} failed: {}",
            stderr.trim()
        )));
    }

    // Bootstrap after successful clone
    bootstrap::ensure_repo_ready_sync(repo_dir)?;
    Ok(())
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
