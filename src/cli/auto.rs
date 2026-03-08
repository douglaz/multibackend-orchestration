use std::path::{Path, PathBuf};

use clap::Args;
use tokio_util::sync::CancellationToken;
use tracing::instrument::WithSubscriber;

use super::init;
use super::parse_positive_u32;
use crate::daemon::tasks::{self, AutoTaskParams};
use crate::error::RalphError;
use crate::workspace::Workspace;
use crate::Result;

const MAX_PROJECT_ID_LEN: usize = 40;

#[derive(Debug, Args)]
pub struct AutoArgs {
    #[arg(long, value_parser = parse_non_empty_idea)]
    pub idea: String,
    #[arg(long, default_value = "")]
    pub spec_writer: String,
    #[arg(long, default_value = "")]
    pub spec_reviewer: String,
    #[arg(long, default_value_t = 1, value_parser = parse_positive_u32)]
    pub max_spec_revisions: u32,
    #[arg(long)]
    pub project_id: Option<String>,
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
    #[arg(long)]
    pub dry_run: bool,
    /// PR URL to pass through to the orchestration context.
    #[arg(long = "pr-url")]
    pub pr_url: Option<String>,
    /// Workspace root directory. When set, config is loaded from this
    /// directory instead of walking up the directory tree. Used by the
    /// daemon to isolate each worktree's configuration.
    #[arg(long = "workspace-root")]
    pub workspace_root: Option<PathBuf>,
    /// Maximum number of backend timeout retries per invocation.
    /// Defaults to 3 when omitted.
    #[arg(long = "max-backend-retries")]
    pub max_backend_retries: Option<u8>,
}

fn parse_non_empty_idea(value: &str) -> std::result::Result<String, String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err("--idea must not be empty".to_owned());
    }
    Ok(trimmed.to_owned())
}

/// Slugify an idea string into a project ID.
/// Public for use by `daemon::tasks`.
pub fn slugify_idea_public(idea: &str) -> String {
    slugify_idea(idea)
}

fn slugify_idea(idea: &str) -> String {
    let mut slug = String::new();
    let mut previous_dash = false;

    for ch in idea.chars() {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch.to_ascii_lowercase());
            previous_dash = false;
            continue;
        }

        if !previous_dash {
            slug.push('-');
            previous_dash = true;
        }
    }

    let mut slug = slug.trim_matches('-').to_owned();
    if slug.len() > MAX_PROJECT_ID_LEN {
        slug.truncate(MAX_PROJECT_ID_LEN);
        slug = slug.trim_end_matches('-').to_owned();
    }

    slug
}

fn ensure_workspace(workspace_root: Option<&PathBuf>, fallback_cwd: &Path) -> Result<Workspace> {
    if let Some(root) = workspace_root {
        let ralph_dir = root.join(".ralph");
        if ralph_dir.join("ralph.toml").is_file() {
            return Workspace::load(ralph_dir);
        }
        let workspace = init::create_workspace(&ralph_dir)?;
        eprintln!("initialized workspace at {}", ralph_dir.display());
        return Ok(workspace);
    }

    // Discover from fallback_cwd (not ambient CWD) so this function is
    // hermetic with respect to the caller's chosen root.
    match Workspace::discover_from(fallback_cwd) {
        Ok(workspace) => Ok(workspace),
        Err(RalphError::WorkspaceNotFound) => {
            let workspace = init::create_workspace(&fallback_cwd.join(".ralph"))?;
            eprintln!("initialized workspace at .ralph");
            Ok(workspace)
        }
        Err(err) => Err(err),
    }
}

pub async fn execute(args: AutoArgs) -> Result<()> {
    let idea = args.idea.trim().to_owned();
    if idea.is_empty() {
        return Err(RalphError::Validation(
            "--idea must not be empty".to_owned(),
        ));
    }

    // Resolve CWD once at the CLI boundary for workspace fallback.
    let cwd = std::env::current_dir()?;

    // Ensure workspace exists and resolve workspace_root for the task.
    let workspace = ensure_workspace(args.workspace_root.as_ref(), &cwd)?;
    let workspace_root = args.workspace_root.unwrap_or_else(|| {
        workspace
            .root
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| workspace.root.clone())
    });

    let spec_writer = if args.spec_writer.trim().is_empty() {
        None
    } else {
        Some(args.spec_writer)
    };
    let spec_reviewer = if args.spec_reviewer.trim().is_empty() {
        None
    } else {
        Some(args.spec_reviewer)
    };

    let dispatch = tasks::cli_stderr_dispatch();
    let result = tasks::run_auto_task(AutoTaskParams {
        workspace_root,
        idea,
        project_id: args.project_id,
        pr_url: args.pr_url,
        cancel: CancellationToken::new(),
        max_backend_retries: args.max_backend_retries,
        spec_writer,
        spec_reviewer,
        max_spec_revisions: args.max_spec_revisions,
        backend: args.backend,
        planner_backend: args.planner_backend,
        implementer_backend: args.implementer_backend,
        reviewer_backend: args.reviewer_backend,
        qa_backend: args.qa_backend,
        completer_backend: args.completer_backend,
        tmux: args.tmux.or(args.no_tmux),
        skip_commit: args.skip_commit,
        skip_prompt_review: args.skip_prompt_review,
        dry_run: args.dry_run,
    })
    .with_subscriber(dispatch)
    .await?;

    println!("{}", result.summary);
    Ok(())
}

#[cfg(test)]
mod tests {
    use clap::Parser;
    use tempfile::tempdir;

    use super::{ensure_workspace, slugify_idea};
    use crate::cli::{Cli, Commands};
    use crate::config::GlobalConfig;

    #[test]
    fn test_slugify_idea_basic() {
        assert_eq!(slugify_idea("add retry logic"), "add-retry-logic");
    }

    #[test]
    fn test_slugify_idea_special_chars() {
        assert_eq!(slugify_idea("fix bug #123 (urgent!)"), "fix-bug-123-urgent");
    }

    #[test]
    fn test_slugify_idea_truncation() {
        let slug = slugify_idea("a very long feature idea that should definitely truncate cleanly");
        assert_eq!(slug.len(), 40);
        assert!(!slug.ends_with('-'));
    }

    #[test]
    fn test_slugify_idea_consecutive_dashes() {
        assert_eq!(slugify_idea("hello   world---test"), "hello-world-test");
    }

    #[test]
    fn parses_auto_command_with_defaults() {
        let cli = Cli::parse_from(["ralph", "auto", "--idea", "test feature"]);
        let Commands::Auto(args) = cli.command else {
            panic!("expected auto command");
        };

        assert_eq!(args.idea, "test feature");
        assert_eq!(args.spec_writer, "");
        assert_eq!(args.spec_reviewer, "");
        assert_eq!(args.max_spec_revisions, 1);
        assert!(args.project_id.is_none());
        assert!(args.backend.is_none());
        assert!(args.planner_backend.is_none());
        assert!(args.implementer_backend.is_none());
        assert!(args.reviewer_backend.is_none());
        assert!(args.qa_backend.is_none());
        assert!(args.completer_backend.is_none());
        assert!(!args.skip_commit);
        assert_eq!(args.tmux, None);
        assert_eq!(args.no_tmux, None);
        assert!(!args.dry_run);
    }

    #[test]
    fn parses_auto_command_with_all_args() {
        let cli = Cli::parse_from([
            "ralph",
            "auto",
            "--idea",
            "test feature",
            "--spec-writer",
            "claude(opus)",
            "--spec-reviewer",
            "codex(gpt-5)",
            "--max-spec-revisions",
            "5",
            "--project-id",
            "retry-backoff",
            "--backend",
            "claude(sonnet)",
            "--planner-backend",
            "codex(gpt-5-codex)",
            "--implementer-backend",
            "claude",
            "--reviewer-backend",
            "codex",
            "--qa-backend",
            "claude(opus)",
            "--completer-backend",
            "codex(gpt-5)",
            "--skip-commit",
            "--tmux",
            "--dry-run",
        ]);
        let Commands::Auto(args) = cli.command else {
            panic!("expected auto command");
        };

        assert_eq!(args.idea, "test feature");
        assert_eq!(args.spec_writer, "claude(opus)");
        assert_eq!(args.spec_reviewer, "codex(gpt-5)");
        assert_eq!(args.max_spec_revisions, 5);
        assert_eq!(args.project_id.as_deref(), Some("retry-backoff"));
        assert_eq!(args.backend.as_deref(), Some("claude(sonnet)"));
        assert_eq!(args.planner_backend.as_deref(), Some("codex(gpt-5-codex)"));
        assert_eq!(args.implementer_backend.as_deref(), Some("claude"));
        assert_eq!(args.reviewer_backend.as_deref(), Some("codex"));
        assert_eq!(args.qa_backend.as_deref(), Some("claude(opus)"));
        assert_eq!(args.completer_backend.as_deref(), Some("codex(gpt-5)"));
        assert!(args.skip_commit);
        assert_eq!(args.tmux, Some(true));
        assert_eq!(args.no_tmux, None);
        assert!(args.dry_run);
    }

    #[test]
    fn rejects_auto_with_empty_idea() {
        let result = Cli::try_parse_from(["ralph", "auto", "--idea", ""]);
        assert!(result.is_err());
    }

    #[test]
    fn ensure_workspace_creates_workspace_when_missing() {
        let temp = tempdir().expect("temp dir");
        let fallback_cwd = temp.path();

        let workspace =
            ensure_workspace(None, fallback_cwd).expect("workspace should be created");
        let workspace_root = temp.path().join(".ralph");

        assert_eq!(workspace.root, workspace_root);
        assert!(workspace_root.join("ralph.toml").exists());
        assert!(workspace_root.join("projects").is_dir());
        assert!(!workspace_root.join("templates").exists());
        assert_eq!(workspace.config, GlobalConfig::default());
    }

    #[test]
    fn ensure_workspace_with_explicit_root_creates_workspace() {
        let temp = tempdir().expect("temp dir");
        let root = temp.path().to_path_buf();
        let fallback_cwd = temp.path();

        let workspace = ensure_workspace(Some(&root), fallback_cwd)
            .expect("workspace should be created at explicit root");
        let ralph_dir = root.join(".ralph");

        assert_eq!(workspace.root, ralph_dir);
        assert!(ralph_dir.join("ralph.toml").exists());
        assert!(ralph_dir.join("projects").is_dir());
        assert_eq!(workspace.config, GlobalConfig::default());
    }

    #[test]
    fn ensure_workspace_with_explicit_root_loads_existing() {
        let temp = tempdir().expect("temp dir");
        let root = temp.path().to_path_buf();
        let fallback_cwd = temp.path();

        // Create a workspace first
        let _ = crate::cli::init::create_workspace(&root.join(".ralph")).expect("create workspace");

        // Loading it again should succeed without re-creating
        let workspace = ensure_workspace(Some(&root), fallback_cwd)
            .expect("workspace should load from explicit root");
        assert_eq!(workspace.root, root.join(".ralph"));
    }
}
