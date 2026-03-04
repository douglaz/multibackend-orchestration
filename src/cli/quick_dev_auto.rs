use std::path::PathBuf;

use clap::Args;

use super::init;
use super::parse_positive_u32;
use crate::backend::{BackendRegistry, BackendRegistryTmuxConfig};
use crate::cli::backend_spec;
use crate::error::RalphError;
use crate::prd::quick::{QuickPrdOptions, QuickPrdPipeline};
use crate::project::lifecycle::{create_project, CreateProjectOptions, PromptSource};
use crate::workflow::quick_dev_orchestrator::{QuickDevOrchestrator, QuickDevRunOptions};
use crate::workspace::Workspace;
use crate::Result;

const MAX_PROJECT_ID_LEN: usize = 40;
const MAX_PROJECT_NAME_LEN: usize = 60;

#[derive(Debug, Args)]
pub struct QuickDevAutoArgs {
    #[arg(long, value_parser = parse_non_empty_idea)]
    pub idea: String,
    #[arg(long = "implementer-backend")]
    pub implementer_backend: Option<String>,
    #[arg(long = "reviewer-backend")]
    pub reviewer_backend: Option<String>,
    #[arg(long)]
    pub project_id: Option<String>,
    #[arg(long = "pr-url")]
    pub pr_url: Option<String>,
    /// Workspace root directory. When set, config is loaded from this
    /// directory instead of walking up the directory tree. Used by the
    /// daemon to isolate each worktree's configuration.
    #[arg(long = "workspace-root")]
    pub workspace_root: Option<PathBuf>,
    #[arg(long)]
    pub skip_commit: bool,
    #[arg(long, value_parser = parse_positive_u32)]
    pub max_review_iterations: Option<u32>,
    #[arg(long, value_parser = parse_positive_u32)]
    pub max_final_review_retries: Option<u32>,
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

pub async fn execute(args: QuickDevAutoArgs) -> Result<()> {
    let QuickDevAutoArgs {
        idea,
        implementer_backend,
        reviewer_backend,
        project_id,
        pr_url,
        workspace_root,
        skip_commit,
        max_review_iterations,
        max_final_review_retries,
    } = args;

    let idea = idea.trim().to_owned();
    if idea.is_empty() {
        return Err(RalphError::Validation(
            "--idea must not be empty".to_owned(),
        ));
    }

    let workspace = ensure_workspace(workspace_root.as_ref())?;

    let writer_spec = workspace.config.workspace.daemon_prd_writer_backend.clone();
    let reviewer_spec = workspace
        .config
        .workspace
        .daemon_prd_reviewer_backend
        .clone();

    println!("Running quick-prd phase...");
    println!("  idea: {idea}");
    println!("  writer backend: {writer_spec}");
    println!("  reviewer backend: {reviewer_spec}");
    println!("  max revisions: 1");

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
            max_revisions: 1,
            dry_run: false,
        },
    );
    let quick_prd_result = quick_prd.run().await?;

    println!("Quick-prd completed.");
    println!("  spec: {}", quick_prd_result.spec_path.display());
    println!("  cache: {}", quick_prd_result.cache_dir.display());
    println!("  revisions: {}", quick_prd_result.revision_count);
    println!("  {}", quick_prd_result.summary);

    if let Some(spec) = implementer_backend.as_deref() {
        backend_spec::validate_backend_spec(spec, &workspace.config)?;
    }
    if let Some(spec) = reviewer_backend.as_deref() {
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
            starting_backend: implementer_backend.clone(),
        },
    )?;
    println!("  project id: {project_id}");
    println!("  project created");

    println!();
    println!("Running quick-dev orchestration...");
    let mut orchestrator = QuickDevOrchestrator::new(workspace);
    let run_result = orchestrator
        .run(QuickDevRunOptions {
            project: Some(project_id),
            implementer_backend,
            reviewer_backend,
            pr_url,
            skip_commit,
            max_review_iterations,
            max_final_review_retries,
        })
        .await?;
    println!("{}", run_result.summary);

    Ok(())
}
