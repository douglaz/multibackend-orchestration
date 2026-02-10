use std::io::IsTerminal;
use std::path::PathBuf;
use std::sync::Arc;

use clap::Args;

use crate::backend::{BackendRegistry, BackendRegistryTmuxConfig};
use crate::cli::backend_spec;
use crate::prd::{
    AnswerStore, CacheManager, NonInteractiveInteraction, PlainInteraction, PrdOptions,
    PrdPipeline,
};
use crate::workspace::Workspace;
use crate::Result;

#[derive(Debug, Args)]
pub struct PrdArgs {
    #[arg(long)]
    pub idea: String,
    #[arg(long, conflicts_with = "interactive")]
    pub non_interactive: bool,
    #[arg(long, conflicts_with = "non_interactive")]
    pub interactive: bool,
    #[arg(long, default_value_t = 3)]
    pub ask_max: u32,
    #[arg(long)]
    pub answers: Option<PathBuf>,
    #[arg(long)]
    pub resume: bool,
    #[arg(long)]
    pub dry_run: bool,
    #[arg(long)]
    pub backend: Option<String>,
}

pub async fn execute(args: PrdArgs) -> Result<()> {
    let workspace = Workspace::discover()?;

    let mut registry = BackendRegistry::new(
        &workspace.config,
        BackendRegistryTmuxConfig {
            enabled: false,
            session_name: workspace.config.workspace.tmux_session.clone(),
            window_keep_seconds: workspace.config.workspace.tmux_window_keep_seconds,
        },
    );

    let backend_spec = args
        .backend
        .clone()
        .unwrap_or_else(|| workspace.config.workspace.default_backend.clone());
    backend_spec::validate_backend_spec(&backend_spec, &workspace.config)?;

    let backend = registry.get_or_create_for_spec(&backend_spec)?;
    backend.health_check().await?;

    // Auto-detect TTY mode: non-interactive if --non-interactive or stdin is not a terminal
    // (unless --interactive is explicitly passed).
    let non_interactive =
        args.non_interactive || (!std::io::stdin().is_terminal() && !args.interactive);

    println!("Starting PRD pipeline...");
    println!("  idea: {}", args.idea);
    println!("  backend: {}", backend.name());
    println!(
        "  mode: {}",
        if non_interactive {
            "non-interactive"
        } else {
            "interactive"
        }
    );
    println!("  ask max rounds: {}", args.ask_max);
    println!("  resume: {}", args.resume);
    println!();

    // Create cache manager for the idea.
    let cache = CacheManager::new(&workspace.root, &args.idea)?;

    // Create answer store, loading from --answers if provided.
    let answers_path = if let Some(ref path) = args.answers {
        path.clone()
    } else {
        cache.cache_dir().join("answers.yaml")
    };

    let mut answer_store = AnswerStore::new(&answers_path);
    if args.answers.is_some() {
        // Pre-load answers from the provided file.
        answer_store.load()?;
    }

    // Create interaction layer based on mode.
    let interaction: Box<dyn crate::prd::UserInteraction> = if non_interactive {
        Box::new(NonInteractiveInteraction::new())
    } else {
        Box::new(PlainInteraction::new())
    };

    // Create pipeline options.
    let options = PrdOptions {
        idea: args.idea.clone(),
        backend_spec: backend_spec.clone(),
        ask_max: args.ask_max,
        resume: args.resume,
        dry_run: args.dry_run,
    };

    // Create and run the pipeline.
    let pipeline = PrdPipeline::new(
        Arc::clone(&backend) as Arc<dyn crate::backend::Backend>,
        interaction,
        cache,
        answer_store,
        options,
    )?;

    let result = pipeline.run().await?;

    // Print summary.
    println!();
    println!("PRD pipeline completed successfully!");
    println!("  PRD written to: {}", result.prd_path.display());
    println!("  Cache directory: {}", result.cache_dir.display());
    println!("  {}", result.summary);
    println!();

    Ok(())
}
