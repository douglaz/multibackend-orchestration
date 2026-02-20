use serde::Serialize;

use crate::cli::HistoryArgs;
use crate::git::ralph_commit::list_ralph_commits;
use crate::project::lifecycle::{project_git_context, reconstruct_project_state};
use crate::workspace::Workspace;
use crate::{error::RalphError, Result};

#[derive(Debug, Clone, Serialize)]
struct CheckpointEntry {
    loop_number: u32,
    from_phase: String,
    to_phase: String,
    commit_hash: Option<String>,
}

pub fn execute(args: HistoryArgs) -> Result<()> {
    let workspace = Workspace::discover()?;
    let project_id = workspace.resolve_project_id(args.project.as_deref())?;

    if !workspace.project_exists(&project_id) {
        return Err(RalphError::ProjectNotFound(project_id));
    }

    let state = reconstruct_project_state(&workspace, &project_id)?;
    let mut checkpoints = collect_checkpoint_history(&workspace, &project_id)?;
    checkpoints.sort_by_key(|entry| entry.loop_number);

    if args.json {
        if checkpoints.is_empty() {
            let mut entries: Vec<serde_json::Value> = Vec::new();
            for loop_state in &state.loops {
                entries.push(serde_json::to_value(loop_state)?);
            }
            for completion in &state.completion_attempts {
                entries.push(serde_json::to_value(completion)?);
            }
            entries.sort_by_key(|value| {
                value
                    .get("loop_number")
                    .and_then(|loop_number| loop_number.as_u64())
                    .unwrap_or(0)
            });
            println!("{}", serde_json::to_string_pretty(&entries)?);
        } else {
            println!("{}", serde_json::to_string_pretty(&checkpoints)?);
        }
        return Ok(());
    }

    println!("PROJECT: {} ({})", state.project_id, state.project_name);
    println!();
    println!("CHECKPOINT HISTORY:");

    if checkpoints.is_empty() {
        if state.loops.is_empty() && state.completion_attempts.is_empty() {
            println!("(no loops yet)");
        } else {
            for loop_state in &state.loops {
                println!(
                    "loop {}: {} ({})",
                    loop_state.loop_number,
                    loop_state.feature_name,
                    loop_status_label(&loop_state.status)
                );
            }
            for completion in &state.completion_attempts {
                println!(
                    "loop {}: completion ({})",
                    completion.loop_number,
                    loop_status_label(&completion.status)
                );
            }
        }
        return Ok(());
    }

    for entry in checkpoints {
        if args.verbose {
            println!(
                "loop {}: {} -> {} ({})",
                entry.loop_number,
                entry.from_phase,
                entry.to_phase,
                entry.commit_hash.as_deref().unwrap_or("unknown")
            );
        } else {
            println!(
                "loop {}: {} -> {}",
                entry.loop_number, entry.from_phase, entry.to_phase
            );
        }
    }

    Ok(())
}

fn collect_checkpoint_history(workspace: &Workspace, project_id: &str) -> Result<Vec<CheckpointEntry>> {
    let Some(git_ctx) = project_git_context(workspace, project_id) else {
        return Ok(Vec::new());
    };

    let commits = list_ralph_commits(&git_ctx.repo_root, &git_ctx.branch)?;
    if commits.is_empty() {
        return Ok(Vec::new());
    }

    let mut entries = Vec::new();
    for commit in commits.into_iter().rev() {
        entries.push(CheckpointEntry {
            loop_number: commit.loop_number,
            from_phase: phase_label(&commit.from_phase).to_owned(),
            to_phase: phase_label(&commit.phase).to_owned(),
            commit_hash: commit.commit_hash,
        });
    }

    Ok(entries)
}

fn phase_label(phase: &crate::project::state::Phase) -> &'static str {
    match phase {
        crate::project::state::Phase::Planning => "planning",
        crate::project::state::Phase::Implementing => "implementing",
        crate::project::state::Phase::QA => "qa",
        crate::project::state::Phase::Reviewing => "reviewing",
        crate::project::state::Phase::Committing => "committing",
        crate::project::state::Phase::Completing => "completing",
    }
}

fn loop_status_label(status: &crate::project::state::LoopStatus) -> &'static str {
    match status {
        crate::project::state::LoopStatus::Pending => "pending",
        crate::project::state::LoopStatus::InProgress => "in_progress",
        crate::project::state::LoopStatus::Completed => "completed",
    }
}
