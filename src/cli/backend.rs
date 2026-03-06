use std::path::PathBuf;

use clap::{Args, Subcommand};

use crate::backend::output_normalizer;
use crate::backend::{claude, codex, openrouter, parse_backend_spec, CliBackend};
use crate::workspace::Workspace;
use crate::Result;

#[derive(Debug, Args)]
pub struct BackendArgs {
    #[command(subcommand)]
    pub command: BackendCommand,
}

#[derive(Debug, Subcommand)]
pub enum BackendCommand {
    /// Execute a backend with the same args/env as the orchestrator
    Exec(BackendExecArgs),
}

#[derive(Debug, Args)]
pub struct BackendExecArgs {
    /// Backend spec, e.g. "claude", "claude(opus)", "openrouter"
    pub backend: String,

    /// Role for model selection (planner, implementer, reviewer, etc.)
    #[arg(long)]
    pub role: Option<String>,

    /// Read prompt from file instead of stdin
    #[arg(long)]
    pub prompt: Option<PathBuf>,

    /// Show raw backend output instead of normalized text
    #[arg(long)]
    pub raw: bool,
}

pub async fn execute(args: BackendArgs) -> Result<()> {
    match args.command {
        BackendCommand::Exec(exec_args) => execute_exec(exec_args).await,
    }
}

async fn execute_exec(args: BackendExecArgs) -> Result<()> {
    let workspace = Workspace::discover()?;
    let config = &workspace.config;

    let spec = parse_backend_spec(&args.backend)?;
    let model = spec.model.as_deref();
    let role = args.role.as_deref();

    let backend: CliBackend = match spec.name.as_str() {
        "claude" => claude::backend_from_config(config, model, role, None),
        "codex" => codex::backend_from_config(config, model, role, None),
        "openrouter" => openrouter::backend_from_config(config, model, role, None),
        _ => {
            return Err(crate::error::RalphError::Validation(format!(
                "unknown backend: {}",
                spec.name
            )));
        }
    };

    // Show the command that will be run
    let resolved = backend.resolved_command_path();
    eprintln!(
        "command: {} {}",
        resolved.display(),
        backend
            .args()
            .iter()
            .map(|a| shell_escape(a))
            .collect::<Vec<_>>()
            .join(" ")
    );

    // Read prompt
    let prompt = if let Some(path) = args.prompt {
        std::fs::read_to_string(&path).map_err(|e| {
            crate::error::RalphError::Io(std::io::Error::new(
                e.kind(),
                format!("failed to read prompt file {}: {e}", path.display()),
            ))
        })?
    } else {
        use std::io::Read;
        let mut buf = String::new();
        std::io::stdin().read_to_string(&mut buf).map_err(|e| {
            crate::error::RalphError::Io(std::io::Error::new(
                e.kind(),
                format!("failed to read prompt from stdin: {e}"),
            ))
        })?;
        buf
    };

    if prompt.trim().is_empty() {
        return Err(crate::error::RalphError::Validation(
            "prompt is empty".to_owned(),
        ));
    }

    // Spawn the backend process
    use tokio::io::AsyncWriteExt;
    use tokio::process::Command;

    let mut cmd = Command::new(&resolved);
    cmd.args(backend.args())
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::inherit())
        .envs(backend.env().clone());

    let mut child = cmd
        .spawn()
        .map_err(|e| crate::error::RalphError::BackendCommandFailed {
            backend: args.backend.clone(),
            details: format!("failed to spawn: {e}"),
        })?;

    // Write prompt to stdin
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(prompt.as_bytes()).await.map_err(|e| {
            crate::error::RalphError::BackendCommandFailed {
                backend: args.backend.clone(),
                details: format!("failed to write prompt to stdin: {e}"),
            }
        })?;
    }

    // Read stdout
    let output = child.wait_with_output().await.map_err(|e| {
        crate::error::RalphError::BackendCommandFailed {
            backend: args.backend.clone(),
            details: format!("failed to wait for child: {e}"),
        }
    })?;

    let raw = String::from_utf8_lossy(&output.stdout).to_string();

    if !output.status.success() {
        eprintln!("exit code: {}", output.status);
    }

    if args.raw {
        print!("{raw}");
    } else {
        match output_normalizer::normalize_output(&raw) {
            Ok(normalized) => {
                print!("{}", normalized.text);
                eprintln!();
                eprintln!("--- Metrics ---");
                if let Some(sid) = &normalized.session_id {
                    eprintln!("session_id:  {sid}");
                }
                eprintln!(
                    "tokens_in:   {}",
                    normalized
                        .tokens_in
                        .map_or("-".to_owned(), |v| v.to_string())
                );
                eprintln!(
                    "tokens_out:  {}",
                    normalized
                        .tokens_out
                        .map_or("-".to_owned(), |v| v.to_string())
                );
                eprintln!(
                    "cached_in:   {}",
                    normalized
                        .cached_in
                        .map_or("-".to_owned(), |v| v.to_string())
                );
            }
            Err(e) => {
                eprintln!("normalization failed: {e}");
                print!("{raw}");
            }
        }
    }

    Ok(())
}

fn shell_escape(s: &str) -> String {
    if s.contains(' ') || s.contains('"') || s.contains('\'') || s.is_empty() {
        format!("'{}'", s.replace('\'', "'\\''"))
    } else {
        s.to_owned()
    }
}
