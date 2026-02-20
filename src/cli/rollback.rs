use std::fs;
use std::path::Path;

use crate::cli::RollbackArgs;
use crate::git::branch::{branch_exists, checkout_branch, resolve_branch_name};
use crate::git::commit::{merge_base, ref_exists, reset_hard};
use crate::git::is_git_repo;
use crate::git::ralph_commit::list_ralph_commits;
use crate::project::lifecycle::reconstruct_project_state;
use crate::project::load_project_config_if_exists;
use crate::project::state::{CompletionVerdict, LoopStatus, Phase, ProjectStatus};
use crate::util::lock::ProjectLock;
use crate::workspace::Workspace;
use crate::{error::RalphError, Result};

pub fn execute(args: RollbackArgs) -> Result<()> {
    let workspace = Workspace::discover()?;
    let project_id = workspace.resolve_project_id(args.project.as_deref())?;

    let project_dir = workspace.project_dir(&project_id);
    let _lock = ProjectLock::acquire(&project_dir, &project_id)?;

    let mut state = reconstruct_project_state(&workspace, &project_id)?;
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

    let hard_ref = if args.hard {
        let repo_root = workspace.root.parent().ok_or_else(|| {
            RalphError::Orchestration("workspace root has no parent path".to_owned())
        })?;
        if !is_git_repo(repo_root) {
            return Err(RalphError::Orchestration(
                "--hard rollback requires a git repository".to_owned(),
            ));
        }

        Some(resolve_hard_reset_ref(
            &workspace,
            &original_state,
            &project_id,
            args.loop_number,
            repo_root,
        )?)
    } else {
        None
    };

    if args.dry_run {
        if let Some(reference) = hard_ref {
            println!(
                "dry-run: would remove loops {:?}, set current loop to {}, and git reset --hard {}",
                to_remove, args.loop_number, reference
            );
        } else {
            println!(
                "dry-run: would remove loops {:?} and set current loop to {}",
                to_remove, args.loop_number
            );
        }
        return Ok(());
    }

    if let Some(reference) = hard_ref.as_deref() {
        let repo_root = workspace.root.parent().ok_or_else(|| {
            RalphError::Orchestration("workspace root has no parent path".to_owned())
        })?;

        // Ensure we reset on the project's branch when branch management is enabled.
        if workspace.config.git.auto_branch {
            let branch = resolve_branch_name(&workspace.config.git.branch_format, &project_id);
            if branch_exists(repo_root, &branch)? {
                checkout_branch(repo_root, &branch)?;
            }
        }

        reset_hard(repo_root, reference)?;
        restore_workspace_files(
            &workspace,
            &project_id,
            prompt_backup.as_deref(),
            project_config_backup.as_deref(),
        )?;
    }

    for &loop_number in &to_remove {
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

    state.loops.retain(|l| l.loop_number <= args.loop_number);
    state
        .completion_attempts
        .retain(|l| l.loop_number <= args.loop_number);

    // Session invalidation: unconditionally remove sessions for loops > target.
    for loop_number in &to_remove {
        state.session_store.remove_for_loop(*loop_number);
    }

    // For the target loop: clear sessions when session_reuse_reset_on_rollback is true.
    // Read config values directly to avoid resolve_effective_config which validates
    // backend specs — rollback must not fail on unrelated backend-config errors.
    if args.loop_number > 0 {
        let project_config = load_project_config_if_exists(&project_dir)?;
        let reset_on_rollback = project_config
            .and_then(|p| p.workflow.session_reuse_reset_on_rollback)
            .unwrap_or(workspace.config.workflow.session_reuse_reset_on_rollback);
        if reset_on_rollback {
            state.session_store.remove_for_loop(args.loop_number);
        }
    }

    state.current_loop = args.loop_number;
    state.current_phase = Phase::Planning;
    state.phase_iteration = 1;

    if state.loops.is_empty() && state.completion_attempts.is_empty() {
        state.status = ProjectStatus::Pending;
    } else if state.completion_attempts.iter().any(|a| {
        a.status == LoopStatus::Completed && a.verdict == Some(CompletionVerdict::Complete)
    }) {
        state.status = ProjectStatus::Completed;
    } else {
        state.status = ProjectStatus::InProgress;
    }

    if let Some(reference) = hard_ref {
        println!(
            "rolled back project {} to loop {} and reset git to {}",
            project_id, args.loop_number, reference
        );
    } else {
        println!(
            "rolled back project {} to loop {}",
            project_id, args.loop_number
        );
    }
    Ok(())
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

    // Fall back to checkpoint commits when tags are not available.
    let branch = resolve_branch_name(&workspace.config.git.branch_format, project_id);
    if let Some(hash) = find_checkpoint_commit(repo_root, &branch, project_id, target_loop_number)?
    {
        return Ok(hash);
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

/// Search the remote project branch for the most recent ralph checkpoint commit
/// at or before `target_loop_number` and return its hash.
fn find_checkpoint_commit(
    repo_root: &Path,
    branch: &str,
    project_id: &str,
    target_loop_number: u32,
) -> Result<Option<String>> {
    let commits = list_ralph_commits(repo_root, branch)?;
    for commit in &commits {
        if commit.project_id == project_id && commit.loop_number <= target_loop_number {
            if let Some(hash) = commit.commit_hash.as_deref() {
                return Ok(Some(hash.to_owned()));
            }
        }
    }
    Ok(None)
}
