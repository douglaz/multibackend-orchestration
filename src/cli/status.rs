use std::fs;
use std::path::Path;

use crate::cli::StatusArgs;
use crate::project::artifacts::resolve_artifact_path_by_suffix;
use crate::project::lifecycle::load_project_state;
use crate::workspace::Workspace;
use crate::{error::RalphError, Result};

pub fn execute(args: StatusArgs) -> Result<()> {
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

    println!("WORKSPACE: {}", workspace.root.display());
    println!("ACTIVE PROJECT: {} ({})", project_ref.id, project_ref.name);
    println!();
    println!("Project Status: {}", project_status_label(&state.status));
    println!("Current Loop: {}", state.current_loop);
    println!(
        "Current Phase: {} (iteration {})",
        phase_label(&state.current_phase),
        state.phase_iteration
    );
    println!();
    let project_dir = workspace.project_dir(&project_id);
    if let Some(loop_state) = state.current_feature_loop() {
        println!(
            "Loop {}: {} ({})",
            loop_state.loop_number,
            loop_state.feature_name,
            loop_status_label(&loop_state.status)
        );
        println!(
            "Backends: planner={}, implementer={}, reviewer={}",
            loop_state.backends.planner,
            loop_state.backends.implementer,
            loop_state.backends.reviewer
        );

        if let Some((iteration, lines)) = latest_feedback(&project_dir, &state, loop_state) {
            println!("Latest Feedback (iteration {iteration}):");
            if lines.is_empty() {
                println!("  (present but could not extract summary lines)");
            } else {
                for line in lines {
                    println!("  • {line}");
                }
            }
        }
    } else if let Some(attempt) = state.current_completion_attempt() {
        println!(
            "Completion Attempt Loop {} ({})",
            attempt.loop_number,
            loop_status_label(&attempt.status)
        );
        println!(
            "Backends: planner={}, completer={}",
            attempt.backends.planner, attempt.backends.completer
        );
        println!(
            "Verdict: {}",
            attempt
                .verdict
                .as_ref()
                .map(completion_verdict_label)
                .unwrap_or("pending")
        );
    } else {
        println!("No active loop context found.");
    }

    println!();
    println!("Previous Feature Loops:");
    let mut previous = 0_u32;
    for loop_state in &state.loops {
        if loop_state.loop_number == state.current_loop {
            continue;
        }
        previous += 1;
        let marker = if loop_state.status == crate::project::state::LoopStatus::Completed {
            "[✓]"
        } else {
            "[ ]"
        };
        println!(
            "  {} Loop {}: {} ({} feedback iterations)",
            marker,
            loop_state.loop_number,
            loop_state.feature_name,
            loop_state.artifacts.reviews.len()
        );
    }

    if previous == 0 {
        println!("  (none)");
    }

    println!();
    println!("Loop artifacts: {}/loops", project_dir.display());
    for loop_state in &state.loops {
        let current = if loop_state.loop_number == state.current_loop {
            " (current)"
        } else {
            ""
        };
        println!("  • {}{}", loop_state.artifacts.spec, current);
    }

    Ok(())
}

fn project_status_label(status: &crate::project::state::ProjectStatus) -> &'static str {
    match status {
        crate::project::state::ProjectStatus::Pending => "pending",
        crate::project::state::ProjectStatus::InProgress => "in_progress",
        crate::project::state::ProjectStatus::Completed => "completed",
    }
}

fn phase_label(phase: &crate::project::state::Phase) -> &'static str {
    match phase {
        crate::project::state::Phase::Planning => "planning",
        crate::project::state::Phase::Implementing => "implementing",
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

fn completion_verdict_label(verdict: &crate::project::state::CompletionVerdict) -> &'static str {
    match verdict {
        crate::project::state::CompletionVerdict::Continue => "continue",
        crate::project::state::CompletionVerdict::Complete => "complete",
    }
}

fn latest_feedback(
    project_dir: &Path,
    state: &crate::project::state::ProjectState,
    loop_state: &crate::project::state::FeatureLoopState,
) -> Option<(u32, Vec<String>)> {
    let pending_iteration = if state.current_phase == crate::project::state::Phase::Implementing
        && loop_state.artifacts.impl_notes.is_some()
    {
        Some(state.phase_iteration)
    } else {
        None
    };

    let (iteration, rel_path) = if let Some(iteration) = pending_iteration {
        let suffix = format!("review-{iteration:03}-feedback.md");
        let rel = resolve_artifact_path_by_suffix(
            project_dir,
            loop_state.loop_number,
            &loop_state.slug,
            &suffix,
        )
        .ok()
        .flatten()?;
        (iteration, rel)
    } else if let Some(last) = loop_state.artifacts.reviews.last() {
        (last.iteration, last.feedback.clone())
    } else {
        return None;
    };

    let raw = fs::read_to_string(project_dir.join(rel_path)).ok()?;
    let body = strip_frontmatter(&raw);
    Some((iteration, extract_required_change_lines(&body)))
}

fn extract_required_change_lines(body: &str) -> Vec<String> {
    let mut in_required = false;
    let mut lines = Vec::new();

    for line in body.lines() {
        let trimmed = line.trim();
        if trimmed == "## Required Changes" {
            in_required = true;
            continue;
        }

        if in_required {
            if trimmed.starts_with("## ") {
                break;
            }
            if trimmed.is_empty() {
                continue;
            }
            lines.push(trimmed.to_owned());
            if lines.len() == 3 {
                break;
            }
        }
    }

    lines
}

fn strip_frontmatter(raw: &str) -> String {
    let trimmed = raw.trim();
    if !trimmed.starts_with("---") {
        return trimmed.to_owned();
    }

    let mut lines = trimmed.lines();
    if lines.next() != Some("---") {
        return trimmed.to_owned();
    }

    let mut in_frontmatter = true;
    let mut out = String::new();
    for line in lines {
        if in_frontmatter {
            if line.trim() == "---" {
                in_frontmatter = false;
            }
            continue;
        }
        out.push_str(line);
        out.push('\n');
    }

    if in_frontmatter {
        trimmed.to_owned()
    } else {
        out.trim().to_owned()
    }
}
