use std::fs;
use std::path::Path;

use crate::cli::RollbackArgs;
use crate::git::branch::{branch_exists, checkout_branch, resolve_branch_name};
use crate::git::commit::{merge_base, ref_exists, reset_hard, rev_parse};
use crate::git::{is_git_repo, run_git, run_git_status};
use crate::project::lifecycle::{
    load_project_state, remove_rollback_target_marker, save_project_state,
    write_rollback_target_marker, ROLLBACK_TARGET_MARKER_FILE,
};
use crate::project::load_project_config_if_exists;
use crate::project::state::{CompletionVerdict, LoopStatus, Phase, ProjectState, ProjectStatus};
use crate::util::lock::ProjectLock;
use crate::workspace::Workspace;
use crate::{error::RalphError, Result};

#[derive(Debug, Clone, Copy)]
enum HardPushOutcome {
    Pushed,
    SkippedNoUpstream,
}

pub fn execute(args: RollbackArgs) -> Result<()> {
    let workspace = Workspace::discover()?;
    let project_id = workspace.resolve_project_id(args.project.as_deref())?;

    let project_dir = workspace.project_dir(&project_id);
    let _lock = ProjectLock::acquire(&project_dir, &project_id)?;

    let mut state = load_project_state(&project_dir)?;
    let original_state = state.clone();
    let prompt_backup = fs::read_to_string(project_dir.join("prompt.md")).ok();
    let project_config_backup = fs::read_to_string(project_dir.join("config.toml")).ok();

    let existing_numbers = state
        .loops
        .iter()
        .map(|l| l.loop_number)
        .chain(state.completion_attempts.iter().map(|l| l.loop_number))
        .collect::<Vec<_>>();

    if args.loop_number > 0 && !existing_numbers.contains(&args.loop_number) {
        return Err(RalphError::Validation(format!(
            "loop {} does not exist in project state",
            args.loop_number
        )));
    }

    let to_remove: Vec<u32> = state
        .loops
        .iter()
        .map(|l| l.loop_number)
        .chain(state.completion_attempts.iter().map(|l| l.loop_number))
        .filter(|num| *num > args.loop_number)
        .collect();

    if args.dry_run {
        if args.hard {
            println!(
                "dry-run: hard rollback to loop {} would remove loops {:?}, reset git (--hard), and force-push upstream; no files/state/marker would be mutated",
                args.loop_number, to_remove
            );
        } else {
            println!(
                "dry-run: soft rollback to loop {} would remove loops {:?}, invalidate sessions, update state, and write {}; no git reset/push",
                args.loop_number, to_remove, ROLLBACK_TARGET_MARKER_FILE
            );
        }
        return Ok(());
    }

    if args.hard {
        let repo_root = workspace.root.parent().ok_or_else(|| {
            RalphError::Orchestration("workspace root has no parent path".to_owned())
        })?;
        if !is_git_repo(repo_root) {
            return Err(RalphError::Orchestration(
                "--hard rollback requires a git repository".to_owned(),
            ));
        }

        let reference = resolve_hard_reset_ref(
            &workspace,
            &original_state,
            &project_id,
            args.loop_number,
            repo_root,
        )?;

        // Ensure we reset on the project's branch when branch management is enabled.
        if workspace.config.git.auto_branch {
            let branch = resolve_branch_name(&workspace.config.git.branch_format, &project_id);
            if branch_exists(repo_root, &branch)? {
                checkout_branch(repo_root, &branch)?;
            }
        }

        let original_head = rev_parse(repo_root, "HEAD")?;

        reset_hard(repo_root, &reference)?;
        restore_workspace_files(
            &workspace,
            &project_id,
            prompt_backup.as_deref(),
            project_config_backup.as_deref(),
        )?;

        let push_outcome = match force_push_current_upstream(repo_root) {
            Ok(outcome) => outcome,
            Err(push_err) => {
                let revert_result = reset_hard(repo_root, &original_head).and_then(|_| {
                    restore_workspace_files(
                        &workspace,
                        &project_id,
                        prompt_backup.as_deref(),
                        project_config_backup.as_deref(),
                    )
                });

                let cleanup_err = apply_soft_rollback_state(
                    args.loop_number,
                    &workspace,
                    &project_dir,
                    &mut state,
                    &to_remove,
                )
                .err();

                return match revert_result {
                    Ok(()) => {
                        let marker_err =
                            write_rollback_target_marker(&project_dir, args.loop_number).err();
                        Err(RalphError::Orchestration(format!(
                            "hard rollback failed to force-push and reverted local HEAD to original commit; applied soft fallback rollback to loop {} (push error: {}; cleanup error: {}; marker error: {})",
                            args.loop_number,
                            push_err,
                            format_optional_error(cleanup_err.as_ref()),
                            format_optional_error(marker_err.as_ref())
                        )))
                    }
                    Err(revert_err) => Err(RalphError::Orchestration(format!(
                        "hard rollback failed to force-push and could not restore local HEAD; repository may be inconsistent (push error: {}; revert error: {}; cleanup error: {})",
                        push_err,
                        revert_err,
                        format_optional_error(cleanup_err.as_ref())
                    ))),
                };
            }
        };

        apply_soft_rollback_state(
            args.loop_number,
            &workspace,
            &project_dir,
            &mut state,
            &to_remove,
        )?;
        remove_rollback_target_marker(&project_dir)?;

        match push_outcome {
            HardPushOutcome::Pushed => {
                println!(
                    "rolled back project {} to loop {} with hard reset {} and force-pushed upstream",
                    project_id, args.loop_number, reference
                );
            }
            HardPushOutcome::SkippedNoUpstream => {
                println!(
                    "rolled back project {} to loop {} with hard reset {} (no upstream configured; skipped force-push)",
                    project_id, args.loop_number, reference
                );
            }
        }
        return Ok(());
    }

    apply_soft_rollback_state(
        args.loop_number,
        &workspace,
        &project_dir,
        &mut state,
        &to_remove,
    )?;
    write_rollback_target_marker(&project_dir, args.loop_number)?;
    println!(
        "rolled back project {} to loop {} (soft rollback; wrote {})",
        project_id, args.loop_number, ROLLBACK_TARGET_MARKER_FILE
    );
    Ok(())
}

fn apply_soft_rollback_state(
    target_loop: u32,
    workspace: &Workspace,
    project_dir: &Path,
    state: &mut ProjectState,
    to_remove: &[u32],
) -> Result<()> {
    for &loop_number in to_remove {
        let pattern = format!("{loop_number:03}-");
        let loops_dir = project_dir.join("loops");
        if loops_dir.is_dir() {
            for entry in fs::read_dir(&loops_dir)? {
                let entry = entry?;
                let name = entry.file_name().to_string_lossy().to_string();
                if name.starts_with(&pattern) {
                    fs::remove_dir_all(entry.path())?;
                }
            }
        }
    }

    state.loops.retain(|l| l.loop_number <= target_loop);
    state
        .completion_attempts
        .retain(|l| l.loop_number <= target_loop);

    // Session invalidation: unconditionally remove sessions for loops > target.
    for loop_number in to_remove {
        state.session_store.remove_for_loop(*loop_number);
    }

    // For the target loop: clear sessions when session_reuse_reset_on_rollback is true.
    // Read config values directly to avoid resolve_effective_config which validates
    // backend specs — rollback must not fail on unrelated backend-config errors.
    if target_loop > 0 {
        let project_config = load_project_config_if_exists(project_dir)?;
        let reset_on_rollback = project_config
            .and_then(|p| p.workflow.session_reuse_reset_on_rollback)
            .unwrap_or(workspace.config.workflow.session_reuse_reset_on_rollback);
        if reset_on_rollback {
            state.session_store.remove_for_loop(target_loop);
        }
    }

    state.current_loop = target_loop;
    state.current_phase = Phase::Planning;
    state.phase_iteration = 1;
    state.status = compute_project_status(state);

    save_project_state(project_dir, state)?;
    Ok(())
}

fn compute_project_status(state: &ProjectState) -> ProjectStatus {
    if state.loops.is_empty() && state.completion_attempts.is_empty() {
        ProjectStatus::Pending
    } else if state.completion_attempts.iter().any(|attempt| {
        attempt.status == LoopStatus::Completed
            && attempt.verdict == Some(CompletionVerdict::Complete)
    }) {
        ProjectStatus::Completed
    } else {
        ProjectStatus::InProgress
    }
}

fn force_push_current_upstream(repo_root: &Path) -> Result<HardPushOutcome> {
    let has_upstream = run_git_status(
        repo_root,
        &["rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{upstream}"],
    )?
    .success();
    if !has_upstream {
        return Ok(HardPushOutcome::SkippedNoUpstream);
    }

    run_git(repo_root, &["push", "--force"])?;
    Ok(HardPushOutcome::Pushed)
}

fn format_optional_error(err: Option<&RalphError>) -> String {
    err.map(|e| e.to_string())
        .unwrap_or_else(|| "none".to_owned())
}

fn restore_workspace_files(
    workspace: &Workspace,
    project_id: &str,
    prompt_backup: Option<&str>,
    project_config_backup: Option<&str>,
) -> Result<()> {
    fs::create_dir_all(
        workspace
            .root
            .join("projects")
            .join(project_id)
            .join("loops"),
    )?;
    if !workspace.root.join("ralph.toml").exists() {
        workspace.save_config()?;
    }

    let prompt_path = workspace
        .root
        .join("projects")
        .join(project_id)
        .join("prompt.md");
    if !prompt_path.exists() {
        if let Some(content) = prompt_backup {
            fs::write(&prompt_path, content)?;
        }
    }

    let project_config_path = workspace
        .root
        .join("projects")
        .join(project_id)
        .join("config.toml");
    if !project_config_path.exists() {
        if let Some(content) = project_config_backup {
            fs::write(&project_config_path, content)?;
        }
    }

    Ok(())
}

fn resolve_hard_reset_ref(
    workspace: &Workspace,
    state: &crate::project::state::ProjectState,
    project_id: &str,
    target_loop_number: u32,
    repo_root: &Path,
) -> Result<String> {
    let tag_format = &workspace.config.workflow.commit_tag_format;

    let target_feature = state
        .loops
        .iter()
        .find(|loop_state| loop_state.loop_number == target_loop_number);
    let target_completion = state
        .completion_attempts
        .iter()
        .find(|loop_state| loop_state.loop_number == target_loop_number);

    let desired_ref = if target_loop_number == 0 {
        None
    } else if let Some(feature) = target_feature {
        if feature.status == LoopStatus::Completed {
            let expected_tag = render_tag(tag_format, project_id, target_loop_number);
            if ref_exists(repo_root, &expected_tag)? {
                Some(expected_tag)
            } else {
                find_prior_tag(
                    repo_root,
                    tag_format,
                    project_id,
                    target_loop_number.saturating_sub(1),
                )?
            }
        } else {
            find_prior_tag(
                repo_root,
                tag_format,
                project_id,
                target_loop_number.saturating_sub(1),
            )?
        }
    } else if target_completion.is_some() {
        find_prior_tag(
            repo_root,
            tag_format,
            project_id,
            target_loop_number.saturating_sub(1),
        )?
    } else {
        return Err(RalphError::Validation(format!(
            "loop {} does not exist",
            target_loop_number
        )));
    };

    if let Some(reference) = desired_ref {
        return Ok(reference);
    }

    resolve_project_base_commit(workspace, state, project_id, repo_root)
}

fn find_prior_tag(
    repo_root: &Path,
    tag_format: &str,
    project_id: &str,
    from_loop: u32,
) -> Result<Option<String>> {
    for loop_number in (1..=from_loop).rev() {
        let tag = render_tag(tag_format, project_id, loop_number);
        if ref_exists(repo_root, &tag)? {
            return Ok(Some(tag));
        }
    }

    Ok(None)
}

fn render_tag(format: &str, project_id: &str, loop_number: u32) -> String {
    format
        .replace("{project_id}", project_id)
        .replace("{loop_number}", &loop_number.to_string())
}

fn resolve_project_base_commit(
    workspace: &Workspace,
    state: &crate::project::state::ProjectState,
    project_id: &str,
    repo_root: &Path,
) -> Result<String> {
    let project_branch = resolve_branch_name(&workspace.config.git.branch_format, project_id);
    let base_ref = if let Some(parent) = state.parent_project.as_deref() {
        resolve_branch_name(&workspace.config.git.branch_format, parent)
    } else {
        workspace.config.git.base_branch.clone()
    };

    if ref_exists(repo_root, &project_branch)? && ref_exists(repo_root, &base_ref)? {
        return merge_base(repo_root, &project_branch, &base_ref);
    }

    if ref_exists(repo_root, &base_ref)? {
        return Ok(base_ref);
    }

    Err(RalphError::Orchestration(format!(
        "could not determine base commit for project {}; missing refs '{}' and/or '{}'",
        project_id, project_branch, base_ref
    )))
}
