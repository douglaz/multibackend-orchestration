use crate::cli::HistoryArgs;
use crate::project::lifecycle::load_project_state;
use crate::workspace::Workspace;
use crate::{error::RalphError, Result};

pub fn execute(args: HistoryArgs) -> Result<()> {
    let workspace = Workspace::discover()?;
    let project_id = if let Some(project) = args.project {
        project
    } else {
        workspace
            .index
            .active_project
            .clone()
            .ok_or(RalphError::ActiveProjectNotSet)?
    };

    let project_ref = workspace
        .index
        .get_project(&project_id)
        .ok_or_else(|| RalphError::ProjectNotFound(project_id.clone()))?;
    let state = load_project_state(&workspace.project_dir(&project_id))?;

    if args.json {
        println!("{}", serde_json::to_string_pretty(&state)?);
        return Ok(());
    }

    println!("PROJECT: {} ({})", project_ref.id, project_ref.name);
    println!(
        "PARENT: {}",
        project_ref.parent_project.as_deref().unwrap_or("none")
    );
    println!("PROMPT: {}", state.prompt_file);
    println!();
    println!("LOOP HISTORY:");

    let mut entries = Vec::new();
    for loop_state in &state.loops {
        entries.push(HistoryEntry::Feature(loop_state));
    }
    for completion in &state.completion_attempts {
        entries.push(HistoryEntry::Completion(completion));
    }
    entries.sort_by_key(|entry| entry.loop_number());

    for entry in entries {
        println!();
        match entry {
            HistoryEntry::Feature(loop_state) => {
                if args.verbose {
                    println!(
                        "Loop {}: {} ({})",
                        loop_state.loop_number,
                        loop_state.feature_name,
                        loop_status_label(&loop_state.status)
                    );
                    println!("  Started: {}", loop_state.started_at.to_rfc3339());
                    println!(
                        "  Completed: {}",
                        loop_state
                            .completed_at
                            .map(|v| v.to_rfc3339())
                            .unwrap_or_else(|| "in-progress".to_owned())
                    );
                    println!(
                        "  Backends: planner={}, implementer={}, reviewer={}",
                        loop_state.backends.planner,
                        loop_state.backends.implementer,
                        loop_state.backends.reviewer
                    );
                    println!(
                        "  Reviews: {} iterations",
                        loop_state.artifacts.reviews.len()
                    );
                    println!("  Spec: {}", loop_state.artifacts.spec);
                } else {
                    println!(
                        "Loop {}: {} ({})",
                        loop_state.loop_number,
                        loop_state.feature_name,
                        loop_status_label(&loop_state.status)
                    );
                }
            }
            HistoryEntry::Completion(attempt) => {
                if args.verbose {
                    println!(
                        "Loop {}: completion ({})",
                        attempt.loop_number,
                        loop_status_label(&attempt.status)
                    );
                    println!("  Started: {}", attempt.started_at.to_rfc3339());
                    println!(
                        "  Completed: {}",
                        attempt
                            .completed_at
                            .map(|v| v.to_rfc3339())
                            .unwrap_or_else(|| "in-progress".to_owned())
                    );
                    println!(
                        "  Backends: planner={}, completer={}",
                        attempt.backends.planner, attempt.backends.completer
                    );
                    println!(
                        "  Verdict: {}",
                        attempt
                            .verdict
                            .as_ref()
                            .map(verdict_label)
                            .unwrap_or_else(|| "pending".to_owned())
                    );
                    println!(
                        "  Termination Request: {}",
                        attempt.artifacts.termination_request
                    );
                } else {
                    println!(
                        "Loop {}: completion ({}) verdict={}",
                        attempt.loop_number,
                        loop_status_label(&attempt.status),
                        attempt
                            .verdict
                            .as_ref()
                            .map(verdict_label)
                            .unwrap_or_else(|| "pending".to_owned())
                    );
                }
            }
        }
    }

    if state.loops.is_empty() && state.completion_attempts.is_empty() {
        println!("(no loops yet)");
    }

    Ok(())
}

fn loop_status_label(status: &crate::project::state::LoopStatus) -> &'static str {
    match status {
        crate::project::state::LoopStatus::Pending => "pending",
        crate::project::state::LoopStatus::InProgress => "in_progress",
        crate::project::state::LoopStatus::Completed => "completed",
    }
}

fn verdict_label(verdict: &crate::project::state::CompletionVerdict) -> String {
    match verdict {
        crate::project::state::CompletionVerdict::Continue => "continue".to_owned(),
        crate::project::state::CompletionVerdict::Complete => "complete".to_owned(),
    }
}

enum HistoryEntry<'a> {
    Feature(&'a crate::project::state::FeatureLoopState),
    Completion(&'a crate::project::state::CompletionLoopState),
}

impl<'a> HistoryEntry<'a> {
    fn loop_number(&self) -> u32 {
        match self {
            Self::Feature(loop_state) => loop_state.loop_number,
            Self::Completion(completion) => completion.loop_number,
        }
    }
}
