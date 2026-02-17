use std::io::IsTerminal;
use std::path::PathBuf;
use std::sync::Arc;

use clap::{Args, ValueEnum};

use crate::backend::{BackendRegistry, BackendRegistryTmuxConfig};
use crate::cli::backend_spec;
use crate::prd::{
    AnswerStore, CacheManager, NonInteractiveInteraction, PlainInteraction, PrdOptions, PrdPipeline,
};
use crate::workspace::Workspace;
use crate::Result;

const DEFAULT_ASK_MAX: u32 = 3;

#[derive(Debug, Clone, Copy, ValueEnum, PartialEq, Eq)]
#[clap(rename_all = "kebab-case")]
pub enum PrdPreset {
    Full,
    Discuss,
    Fast,
}

impl PrdPreset {
    fn default_ask_max(self) -> u32 {
        match self {
            Self::Full => 3,
            Self::Discuss => 1,
            Self::Fast => 0,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Full => "full",
            Self::Discuss => "discuss",
            Self::Fast => "fast",
        }
    }
}

#[derive(Debug, Args)]
pub struct PrdArgs {
    #[arg(long)]
    pub idea: String,
    #[arg(long, conflicts_with = "interactive")]
    pub non_interactive: bool,
    #[arg(long, conflicts_with = "non_interactive")]
    pub interactive: bool,
    #[arg(long)]
    pub ask_max: Option<u32>,
    #[arg(long)]
    pub preset: Option<PrdPreset>,
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
    let ask_max = resolved_ask_max(&args);

    let preset = args.preset.unwrap_or(PrdPreset::Full);

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
    println!("  ask preset: {}", preset.name());
    println!("  ask max rounds: {}", ask_max);
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
        ask_max,
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

fn resolved_ask_max(args: &PrdArgs) -> u32 {
    args.ask_max
        .or_else(|| args.preset.map(|preset| preset.default_ask_max()))
        .unwrap_or(DEFAULT_ASK_MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolved_ask_max_prefers_explicit_flag_over_preset() {
        let args = PrdArgs {
            idea: "x".into(),
            non_interactive: false,
            interactive: false,
            ask_max: Some(7),
            preset: Some(PrdPreset::Fast),
            answers: None,
            resume: false,
            dry_run: false,
            backend: None,
        };

        assert_eq!(resolved_ask_max(&args), 7);
    }

    #[test]
    fn resolved_ask_max_falls_back_to_preset_default() {
        let args = PrdArgs {
            idea: "x".into(),
            non_interactive: false,
            interactive: false,
            ask_max: None,
            preset: Some(PrdPreset::Discuss),
            answers: None,
            resume: false,
            dry_run: false,
            backend: None,
        };

        assert_eq!(
            resolved_ask_max(&args),
            PrdPreset::Discuss.default_ask_max()
        );
    }

    #[test]
    fn resolved_ask_max_falls_back_to_full_default() {
        let args = PrdArgs {
            idea: "x".into(),
            non_interactive: false,
            interactive: false,
            ask_max: None,
            preset: None,
            answers: None,
            resume: false,
            dry_run: false,
            backend: None,
        };

        assert_eq!(resolved_ask_max(&args), DEFAULT_ASK_MAX);
    }
}
