use std::path::PathBuf;

use clap::Args;
use tokio_util::sync::CancellationToken;

use super::init;
use super::parse_positive_u32;
use crate::backend::{BackendRegistry, BackendRegistryTmuxConfig};
use crate::cli::backend_spec;
use crate::error::RalphError;
use crate::prd::quick::{QuickPrdOptions, QuickPrdPipeline};
use crate::project::lifecycle::{create_project, CreateProjectOptions, PromptSource};
use crate::workflow::orchestrator::{Orchestrator, RunOptions};
use crate::workspace::Workspace;
use crate::Result;

const MAX_PROJECT_ID_LEN: usize = 40;
const MAX_PROJECT_NAME_LEN: usize = 60;

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
}

fn parse_non_empty_idea(value: &str) -> std::result::Result<String, String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err("--idea must not be empty".to_owned());
    }
    Ok(trimmed.to_owned())
}

fn truncate_idea_for_name(idea: &str) -> String {
    idea.chars().take(MAX_PROJECT_NAME_LEN).collect()
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

fn ensure_workspace(workspace_root: Option<&PathBuf>) -> Result<Workspace> {
    if let Some(root) = workspace_root {
        let ralph_dir = root.join(".ralph");
        if ralph_dir.join("ralph.toml").is_file() {
            return Workspace::load(ralph_dir);
        }
        let workspace = init::create_workspace(&ralph_dir)?;
        eprintln!("initialized workspace at {}", ralph_dir.display());
        return Ok(workspace);
    }

    match Workspace::discover() {
        Ok(workspace) => Ok(workspace),
        Err(RalphError::WorkspaceNotFound) => {
            let workspace = init::create_workspace(&std::env::current_dir()?.join(".ralph"))?;
            eprintln!("initialized workspace at .ralph");
            Ok(workspace)
        }
        Err(err) => Err(err),
    }
}

pub async fn execute(args: AutoArgs) -> Result<()> {
    let AutoArgs {
        idea,
        spec_writer,
        spec_reviewer,
        max_spec_revisions,
        project_id,
        backend,
        planner_backend,
        implementer_backend,
        reviewer_backend,
        qa_backend,
        completer_backend,
        skip_commit,
        skip_prompt_review,
        tmux,
        no_tmux,
        dry_run,
        pr_url,
        workspace_root,
    } = args;

    let idea = idea.trim().to_owned();
    if idea.is_empty() {
        return Err(RalphError::Validation(
            "--idea must not be empty".to_owned(),
        ));
    }

    let workspace = ensure_workspace(workspace_root.as_ref())?;

    let writer_spec = if spec_writer.trim().is_empty() {
        workspace.config.workspace.daemon_prd_writer_backend.clone()
    } else {
        spec_writer
    };
    let reviewer_spec = if spec_reviewer.trim().is_empty() {
        workspace
            .config
            .workspace
            .daemon_prd_reviewer_backend
            .clone()
    } else {
        spec_reviewer
    };

    println!("Running quick-prd phase...");
    println!("  idea: {idea}");
    println!("  writer backend: {writer_spec}");
    println!("  reviewer backend: {reviewer_spec}");
    println!("  max revisions: {max_spec_revisions}");

    let mut registry = BackendRegistry::new(
        &workspace.config,
        BackendRegistryTmuxConfig {
            enabled: false,
            session_name: workspace.config.workspace.tmux_session.clone(),
            window_keep_seconds: workspace.config.workspace.tmux_window_keep_seconds,
        },
    );
    registry.set_cwd(Some(std::env::current_dir()?));

    backend_spec::validate_backend_spec(&writer_spec, &workspace.config)?;
    backend_spec::validate_backend_spec(&reviewer_spec, &workspace.config)?;

    let writer = registry.get_or_create_for_spec(&writer_spec)?;
    let reviewer = registry.get_or_create_for_spec(&reviewer_spec)?;
    writer.health_check().await?;
    reviewer.health_check().await?;

    let quick_prd = QuickPrdPipeline::new(
        writer,
        reviewer,
        QuickPrdOptions {
            idea: idea.clone(),
            writer_spec: writer_spec.clone(),
            reviewer_spec: reviewer_spec.clone(),
            max_revisions: max_spec_revisions,
            dry_run: false,
        },
    );
    let quick_prd_result = quick_prd.run().await?;

    println!("Quick-prd completed.");
    println!("  spec: {}", quick_prd_result.spec_path.display());
    println!("  cache: {}", quick_prd_result.cache_dir.display());
    println!("  revisions: {}", quick_prd_result.revision_count);
    println!("  {}", quick_prd_result.summary);

    if dry_run {
        let spec = std::fs::read_to_string(&quick_prd_result.spec_path)?;
        println!();
        println!("{spec}");
        return Ok(());
    }

    if let Some(spec) = backend.as_deref() {
        backend_spec::validate_backend_spec(spec, &workspace.config)?;
    }
    if let Some(spec) = planner_backend.as_deref() {
        backend_spec::validate_backend_spec(spec, &workspace.config)?;
    }
    if let Some(spec) = implementer_backend.as_deref() {
        backend_spec::validate_backend_spec(spec, &workspace.config)?;
    }
    if let Some(spec) = reviewer_backend.as_deref() {
        backend_spec::validate_backend_spec(spec, &workspace.config)?;
    }
    if let Some(spec) = qa_backend.as_deref() {
        backend_spec::validate_backend_spec(spec, &workspace.config)?;
    }
    if let Some(spec) = completer_backend.as_deref() {
        backend_spec::validate_backend_spec(spec, &workspace.config)?;
    }

    let project_id = project_id.unwrap_or_else(|| slugify_idea(&idea));
    if project_id.is_empty() {
        return Err(RalphError::Validation(
            "derived project id from --idea is empty; pass --project-id".to_owned(),
        ));
    }
    let project_name = truncate_idea_for_name(&idea);

    println!();
    println!("Creating project...");
    create_project(
        &workspace,
        CreateProjectOptions {
            id: project_id.clone(),
            name: project_name,
            source: PromptSource::File(quick_prd_result.spec_path),
            starting_backend: backend.clone(),
        },
    )?;
    println!("  project id: {project_id}");
    println!("  project created");

    println!();
    println!("Running orchestration until completion...");
    let workspace = ensure_workspace(workspace_root.as_ref())?;
    let mut orchestrator = Orchestrator::new(workspace);
    let run_result = orchestrator
        .run(RunOptions {
            project: Some(project_id),
            loops: None,
            until_review: false,
            until_complete: true,
            dry_run: false,
            backend,
            planner_backend,
            implementer_backend,
            reviewer_backend,
            qa_backend,
            completer_backend,
            tmux: tmux.or(no_tmux),
            on_prompt_change: None,
            skip_commit,
            skip_prompt_review,
            pr_url,
            cancel: CancellationToken::new(),
            max_backend_retries: None,
        })
        .await?;
    println!("{}", run_result.summary);

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::{Mutex, OnceLock};

    use clap::Parser;
    use tempfile::tempdir;

    use super::{ensure_workspace, slugify_idea};
    use crate::cli::{Cli, Commands};
    use crate::config::GlobalConfig;

    fn cwd_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    struct CwdGuard {
        original: PathBuf,
    }

    impl CwdGuard {
        fn set(path: &std::path::Path) -> Self {
            let original = std::env::current_dir().expect("get current dir");
            std::env::set_current_dir(path).expect("set current dir");
            Self { original }
        }
    }

    impl Drop for CwdGuard {
        fn drop(&mut self) {
            let _ = std::env::set_current_dir(&self.original);
        }
    }

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
        let _cwd_guard = cwd_lock().lock().expect("cwd lock");
        let temp = tempdir().expect("temp dir");
        let _guard = CwdGuard::set(temp.path());

        let workspace = ensure_workspace(None).expect("workspace should be created");
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

        let workspace =
            ensure_workspace(Some(&root)).expect("workspace should be created at explicit root");
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

        // Create a workspace first
        let _ = crate::cli::init::create_workspace(&root.join(".ralph")).expect("create workspace");

        // Loading it again should succeed without re-creating
        let workspace =
            ensure_workspace(Some(&root)).expect("workspace should load from explicit root");
        assert_eq!(workspace.root, root.join(".ralph"));
    }
}
