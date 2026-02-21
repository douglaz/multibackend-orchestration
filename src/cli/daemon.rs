use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Command;

use clap::{Args, Subcommand};

use crate::config::resolve_daemon_config;
use crate::daemon::bootstrap;
use crate::daemon::github;
use crate::daemon::rebase_agent::parse_rebase_agent_backend;
use crate::daemon::runtime::{retrigger_failed_task, spawn_blocking_op, DaemonRuntimeConfig};
use crate::project::load_project_config_if_exists;
use crate::util::lock::DaemonLock;
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
    Retrigger(DaemonRetriggerArgs),
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
    /// Owner/repo to query status for. If omitted, scans all repos in data-dir.
    #[arg(long = "repo")]
    pub repo: Vec<String>,
}

#[derive(Debug, Args)]
pub struct DaemonAbortArgs {
    #[arg(long)]
    pub data_dir: PathBuf,
    /// Issue number to abort (fetches current labels from GitHub).
    pub issue_number: String,
    /// Owner/repo of the issue to abort.
    #[arg(long)]
    pub repo: Option<String>,
}

#[derive(Debug, Args)]
pub struct DaemonRetriggerArgs {
    /// Issue number to retrigger.
    pub issue_number: String,
    /// Owner/repo of the issue.
    #[arg(long)]
    pub repo: Option<String>,
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
        DaemonCommand::Retrigger(retrigger_args) => {
            spawn_blocking_op(move || execute_retrigger(retrigger_args)).await
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

    // Guard: --data-dir must not be inside a git working tree
    guard_not_git_repo(&args.data_dir)?;

    preflight_check_gh()?;

    // Create data-dir after guard passes
    std::fs::create_dir_all(&args.data_dir).map_err(|err| {
        RalphError::Orchestration(format!(
            "failed to create data-dir {}: {err}",
            args.data_dir.display()
        ))
    })?;

    // Resolve ralph binary path
    let ralph_bin = match std::env::var("RALPH_DAEMON_BIN") {
        Ok(path) if !path.is_empty() => PathBuf::from(path),
        _ => std::env::current_exe().map_err(|err| {
            RalphError::Orchestration(format!("cannot determine ralph binary path: {err}"))
        })?,
    };

    let mut deprecation_warned = false;

    let mut repo_configs: Vec<DaemonRuntimeConfig> = Vec::new();
    let mut daemon_locks: Vec<DaemonLock> = Vec::new();

    for slug in &normalized_repos {
        let (owner, repo_name) = parse_repo_slug(slug)?;
        let repo_dir = args.data_dir.join(&owner).join(&repo_name);

        // Clone or bootstrap the repo
        clone_or_bootstrap(&owner, &repo_name, &repo_dir)?;

        // Ensure lifecycle labels exist (best-effort, non-blocking)
        github::ensure_labels_best_effort(&owner, &repo_name);

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
        let rebase_agent_backend =
            parse_rebase_agent_backend(&daemon_cfg.rebase_agent_backend).map_err(|err| {
                RalphError::Validation(format!(
                    "invalid daemon rebase agent backend for {slug}: {err}"
                ))
            })?;

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

        let runtime_config = DaemonRuntimeConfig {
            owner,
            repo: repo_name,
            base_branch: workspace.config.git.base_branch.clone(),
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
            rebase_agent_backend,
            workspace_root: workspace.root.clone(),
        };

        let daemon_lock = DaemonLock::acquire(&runtime_config.repo_root)?;
        daemon_locks.push(daemon_lock);
        repo_configs.push(runtime_config);
    }

    // Run one runtime::run() per repo using JoinSet
    let mut join_set = tokio::task::JoinSet::new();

    for config in repo_configs {
        join_set.spawn(async move { crate::daemon::runtime::run(&config).await });
    }

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

    drop(daemon_locks);
    Ok(())
}

/// Status: query GitHub labels to show current issue lifecycle state.
fn execute_status(args: DaemonStatusArgs) -> Result<()> {
    let repos = if args.repo.is_empty() {
        // Scan data-dir for owner/repo directories
        scan_repo_slugs(&args.data_dir)?
    } else {
        args.repo
            .iter()
            .map(|r| {
                validate_repo_slug(r)?;
                Ok(r.trim().to_ascii_lowercase())
            })
            .collect::<Result<Vec<_>>>()?
    };

    if repos.is_empty() {
        println!("no daemon repos found");
        return Ok(());
    }

    let mut found_any = false;

    for slug in &repos {
        let (owner, repo_name) = parse_repo_slug(slug)?;

        // Query issues with lifecycle labels from GitHub.
        // Each label requires a separate query because `gh issue list --label`
        // uses AND semantics (repeated --label only returns issues matching ALL).
        let mut issues: Vec<github::GhIssue> = Vec::new();
        let mut seen: std::collections::HashSet<u32> = std::collections::HashSet::new();
        for label in &["ralph:ready", "ralph:in-progress"] {
            let query_labels = vec![label.to_string()];
            match github::poll_issues(&owner, &repo_name, &query_labels) {
                Ok((batch, _overflow)) => {
                    for issue in batch {
                        if seen.insert(issue.number) {
                            issues.push(issue);
                        }
                    }
                }
                Err(err) => {
                    eprintln!("warning: failed to query {label} issues for {slug}: {err}");
                }
            }
        }

        if issues.is_empty() {
            continue;
        }

        if !found_any {
            println!("DAEMON ISSUES");
            println!(
                "{:<20} {:<8} {:<30}",
                "REPO", "ISSUE", "LIFECYCLE LABELS"
            );
            found_any = true;
        }

        for issue in &issues {
            let lifecycle = github::classify_lifecycle_labels(&issue.labels);
            println!(
                "{:<20} {:<8} {:<30}",
                slug,
                issue.number,
                lifecycle.join(", ")
            );
        }
    }

    if !found_any {
        println!("no daemon tasks");
    }

    Ok(())
}

/// Abort: kill child (if running locally) and swap label to `ralph:failed`.
fn execute_abort(args: DaemonAbortArgs) -> Result<()> {
    let issue_number: u32 = args.issue_number.parse().map_err(|_| {
        RalphError::Validation(format!(
            "invalid issue number: {}",
            args.issue_number
        ))
    })?;

    let slug = args.repo.ok_or_else(|| {
        RalphError::Validation("--repo is required for abort".to_owned())
    })?;
    let (owner, repo_name) = parse_repo_slug(&slug)?;

    // Verify the issue is currently in-progress
    let labels = github::fetch_issue_labels(&owner, &repo_name, issue_number)?;
    let lifecycle = github::classify_lifecycle_labels(&labels);

    if !lifecycle.iter().any(|l| l == "ralph:in-progress") {
        return Err(RalphError::Validation(format!(
            "issue {slug}#{issue_number} is not in-progress (labels: {})",
            lifecycle.join(", ")
        )));
    }

    // Swap label: in-progress -> failed (no PID info available from CLI)
    crate::daemon::abort_task_by_labels(&owner, &repo_name, issue_number, None, None)?;

    println!("aborted issue {slug}#{issue_number}");
    Ok(())
}

fn execute_retrigger(args: DaemonRetriggerArgs) -> Result<()> {
    let issue_number: u32 = args.issue_number.parse().map_err(|_| {
        RalphError::Validation(format!(
            "invalid issue number: {}",
            args.issue_number
        ))
    })?;

    let slug = args.repo.ok_or_else(|| {
        RalphError::Validation("--repo is required for retrigger".to_owned())
    })?;
    let (owner, repo_name) = parse_repo_slug(&slug)?;

    retrigger_failed_task(&owner, &repo_name, issue_number)?;
    println!("retriggered issue {slug}#{issue_number}");
    Ok(())
}

/// Scan `<data-dir>/<owner>/<repo>` directories and return owner/repo slugs.
fn scan_repo_slugs(data_dir: &Path) -> Result<Vec<String>> {
    let mut slugs = Vec::new();

    let owners = match std::fs::read_dir(data_dir) {
        Ok(entries) => entries,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(slugs),
        Err(err) => return Err(err.into()),
    };

    for owner_entry in owners {
        let owner_entry = owner_entry?;
        if !owner_entry.file_type()?.is_dir() {
            continue;
        }

        let owner_name = owner_entry.file_name().to_string_lossy().into_owned();
        let repos = match std::fs::read_dir(owner_entry.path()) {
            Ok(entries) => entries,
            Err(_) => continue,
        };

        for repo_entry in repos {
            let repo_entry = repo_entry?;
            if !repo_entry.file_type()?.is_dir() {
                continue;
            }

            let repo_name = repo_entry.file_name().to_string_lossy().into_owned();
            // Check if it looks like a git repo
            if repo_entry.path().join(".git").exists() {
                slugs.push(format!("{owner_name}/{repo_name}"));
            }
        }
    }

    Ok(slugs)
}

/// Reject `--data-dir` paths inside a git working tree.
fn guard_not_git_repo(data_dir: &Path) -> Result<()> {
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

fn clone_or_bootstrap(owner: &str, repo: &str, repo_dir: &Path) -> Result<()> {
    if repo_dir.join(".git").exists() {
        bootstrap::ensure_repo_ready_sync(repo_dir)?;
        return Ok(());
    }

    if let Some(parent) = repo_dir.parent() {
        std::fs::create_dir_all(parent).map_err(|err| {
            RalphError::Orchestration(format!(
                "failed to create parent directory for {}: {err}",
                repo_dir.display()
            ))
        })?;
    }

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

    let ssh_url = format!("git@github.com:{owner}/{repo}.git");
    let _ = Command::new("git")
        .args(["remote", "set-url", "origin", &ssh_url])
        .current_dir(repo_dir)
        .output();

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
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Err(RalphError::Validation(
            "gh (GitHub CLI) not found in PATH. The daemon requires gh to poll issues, \
                 post comments, and create PRs. Install it from https://cli.github.com/ \
                 or run inside `nix develop`."
                .to_owned(),
        )),
        Err(err) => Err(RalphError::Validation(format!(
            "gh (GitHub CLI) check failed: {err}"
        ))),
    }
}
