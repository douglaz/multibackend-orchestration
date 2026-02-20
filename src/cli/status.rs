use std::fs;
use std::path::Path;

use crate::cli::StatusArgs;
use crate::daemon::github::{classify_lifecycle_labels, fetch_issue_labels};
use crate::project::artifacts::resolve_artifact_path_by_suffix;
use crate::project::lifecycle::{
    parse_github_repo_slug, parse_issue_number, project_git_context, reconstruct_project_state,
};
use crate::project::state::ProjectStatus;
use crate::workspace::Workspace;
use crate::{error::RalphError, Result};

pub fn execute(args: StatusArgs) -> Result<()> {
    let workspace = Workspace::discover()?;
    let project_id = workspace.resolve_project_id(args.project.as_deref())?;

    if !workspace.project_exists(&project_id) {
        return Err(RalphError::ProjectNotFound(project_id));
    }
    let mut state = reconstruct_project_state(&workspace, &project_id)?;

    // Position is already derived from checkpoint commits in
    // reconstruct_project_state — no additional remap needed here.

    if let Some(label_status) = derive_project_status_from_labels(&workspace, &project_id) {
        state.status = label_status;
    }

    println!("WORKSPACE: {}", workspace.root.display());
    println!(
        "ACTIVE PROJECT: {} ({})",
        state.project_id, state.project_name
    );
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
            "Backends: planner={}, implementer={}, reviewer={}, qa={}",
            loop_state.backends.planner,
            loop_state.backends.implementer,
            loop_state.backends.reviewer,
            loop_state.backends.qa
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

        if let Some(latest_qa) = loop_state.artifacts.qa_results.last() {
            let verdict = if latest_qa.passed { "PASS" } else { "FAIL" };
            println!(
                "Latest QA (iteration {}): {} [{}]",
                latest_qa.iteration, verdict, latest_qa.report
            );
            let lines = extract_qa_summary_lines(&project_dir, latest_qa);
            if lines.is_empty() {
                println!("  (no summary available)");
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
        crate::project::state::ProjectStatus::Failed => "failed",
    }
}

fn derive_project_status_from_labels(workspace: &Workspace, project_id: &str) -> Option<ProjectStatus> {
    let issue_number = parse_issue_number(project_id)?;
    let git_context = project_git_context(workspace, project_id)?;
    let (owner, repo) = parse_github_repo_slug(&git_context.repo_root)?;

    let labels = fetch_issue_labels(&owner, &repo, issue_number).ok()?;
    let lifecycle = classify_lifecycle_labels(&labels);
    if lifecycle.len() > 1 {
        return Some(ProjectStatus::Failed);
    }

    match lifecycle.first().map(String::as_str) {
        Some("ralph:ready") => Some(ProjectStatus::Pending),
        Some("ralph:in-progress") => Some(ProjectStatus::InProgress),
        Some("ralph:completed") => Some(ProjectStatus::Completed),
        Some("ralph:failed") => Some(ProjectStatus::Failed),
        _ => None,
    }
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

fn extract_qa_summary_lines(
    project_dir: &Path,
    qa: &crate::project::state::QaExchange,
) -> Vec<String> {
    let full_path = project_dir.join(&qa.report);
    let raw = match fs::read_to_string(&full_path) {
        Ok(content) => content,
        Err(_) => return Vec::new(),
    };
    let body = strip_frontmatter(&raw);
    let section = if qa.passed {
        "## Acceptance Criteria Verification"
    } else {
        "## Failures"
    };
    extract_section_lines(&body, section)
}

fn extract_section_lines(body: &str, section_header: &str) -> Vec<String> {
    let mut in_section = false;
    let mut lines = Vec::new();

    for line in body.lines() {
        let trimmed = line.trim();
        if trimmed == section_header {
            in_section = true;
            continue;
        }

        if in_section {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_section_lines_returns_up_to_3_lines_from_acceptance_criteria_verification() {
        let body = "\
# QA: PASS

## Manual Testing
- ran ralph init, verified config created

## Acceptance Criteria Verification
All acceptance criteria satisfied.
Build succeeds with no warnings.
Integration tests cover the new endpoint.
Extra line that should be excluded.

## Notes
Cleanup suggestions.
";
        let lines = extract_section_lines(body, "## Acceptance Criteria Verification");
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0], "All acceptance criteria satisfied.");
        assert_eq!(lines[1], "Build succeeds with no warnings.");
        assert_eq!(lines[2], "Integration tests cover the new endpoint.");
    }

    #[test]
    fn extract_section_lines_returns_up_to_3_lines_from_failures() {
        let body = "\
# QA: FAIL

## Failures
- cargo test fails: 2 tests broken
- missing validation for empty input

## Suggested Fixes
- Add empty-input guard
";
        let lines = extract_section_lines(body, "## Failures");
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0], "- cargo test fails: 2 tests broken");
        assert_eq!(lines[1], "- missing validation for empty input");
    }

    #[test]
    fn extract_section_lines_returns_empty_when_section_missing() {
        let body = "\
# QA: PASS

## Manual Testing
- all good
";
        let lines = extract_section_lines(body, "## Acceptance Criteria Verification");
        assert!(lines.is_empty());
    }

    #[test]
    fn extract_section_lines_returns_empty_when_section_has_no_content() {
        let body = "\
# QA: FAIL

## Failures

## Suggested Fixes
- something
";
        let lines = extract_section_lines(body, "## Failures");
        assert!(lines.is_empty());
    }

    #[test]
    fn extract_section_lines_handles_body_with_frontmatter_stripped() {
        let raw = "\
---
artifact: qa-pass
loop: 1
---

# QA: PASS

## Acceptance Criteria Verification
Everything works.
";
        let body = strip_frontmatter(raw);
        let lines = extract_section_lines(&body, "## Acceptance Criteria Verification");
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0], "Everything works.");
    }

    #[test]
    fn extract_required_change_lines_unchanged_behavior() {
        let body = "\
# Review: SUGGESTIONS

## Required Changes
- Fix the error handling in auth module
- Add unit tests for the new endpoint
- Update the API documentation

## Optional Improvements
- Consider caching
";
        let lines = extract_required_change_lines(body);
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0], "- Fix the error handling in auth module");
    }
}
