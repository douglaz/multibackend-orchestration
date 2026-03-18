use std::fs;
use std::path::Path;

use crate::cli::RollbackArgs;
use crate::git::branch::{
    branch_exists, checkout_branch, create_branch, remote_branch_exists_on_remote,
    resolve_branch_name, RemoteBranchProbeResult,
};
use crate::git::commit::{merge_base, ref_exists, reset_hard};
use crate::git::ralph_commit::list_ralph_commits;
use crate::git::{is_git_repo, run_git};
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

    // Compute removal targets from BOTH state-derived loop numbers AND on-disk
    // loop directories.  Reconstruction may hide loops above the rollback
    // ceiling (via `loop_dirs.retain`), so relying solely on state would miss
    // artifact directories that exist on disk but are invisible to the capped
    // state.  The union ensures `rollback 0` removes all loop artifacts.
    let mut removal_set: std::collections::HashSet<u32> = state
        .loops
        .iter()
        .map(|l| l.loop_number)
        .chain(state.completion_attempts.iter().map(|l| l.loop_number))
        .filter(|num| *num > args.loop_number)
        .collect();
    let loops_dir = project_dir.join("loops");
    if loops_dir.is_dir() {
        if let Ok(entries) = fs::read_dir(&loops_dir) {
            for entry in entries.flatten() {
                if !entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false) {
                    continue;
                }
                let name = entry.file_name().to_string_lossy().to_string();
                if let Some((num_str, _)) = name.split_once('-') {
                    if let Ok(n) = num_str.parse::<u32>() {
                        if n > args.loop_number {
                            removal_set.insert(n);
                        }
                    }
                }
            }
        }
    }
    let mut to_remove: Vec<u32> = removal_set.into_iter().collect();
    to_remove.sort();

    // Capture the latest checkpoint commit hash BEFORE any git mutations.
    // This is stored in the `.rollback-ceiling` marker for provenance-based
    // staleness detection during reconstruction (P1).
    let latest_checkpoint_hash: Option<String> = workspace
        .root
        .parent()
        .filter(|r| is_git_repo(r))
        .and_then(|repo_root| {
            let branch = resolve_branch_name(&workspace.config.git.branch_format, &project_id);
            list_ralph_commits(repo_root, &branch, &project_id)
                .ok()
                .and_then(|commits| commits.first().and_then(|c| c.commit_hash.clone()))
        });

    // Dry-run: resolve ref early for display (read-only, no branch mutations).
    if args.dry_run {
        if args.hard {
            let repo_root = workspace.root.parent().ok_or_else(|| {
                RalphError::Orchestration("workspace root has no parent path".to_owned())
            })?;
            if is_git_repo(repo_root) {
                // Mirror the execution path's branch-visibility check: only
                // resolve the reset ref when the project branch already exists
                // locally.  If it would need recovery (create from remote-
                // tracking / fetch) we cannot determine the ref without side
                // effects, so print a placeholder instead of a wrong value.
                let branch = resolve_branch_name(&workspace.config.git.branch_format, &project_id);
                if branch_exists(repo_root, &branch)? {
                    let reference = resolve_hard_reset_ref(
                        &workspace,
                        &original_state,
                        &project_id,
                        args.loop_number,
                        repo_root,
                    )?;
                    println!(
                        "dry-run (hard rollback): would remove loops {:?}, set current loop to {}, and git reset --hard {}",
                        to_remove, args.loop_number, reference
                    );
                } else {
                    match remote_branch_exists_on_remote(repo_root, &branch)? {
                        RemoteBranchProbeResult::Exists => {
                            // Branch can be recovered from remote — exact ref
                            // unavailable without side effects.
                            println!(
                                "dry-run (hard rollback): would remove loops {:?}, set current loop to {}, and git reset --hard <ref> (branch '{}' requires recovery; exact ref unavailable in dry-run)",
                                to_remove, args.loop_number, branch
                            );
                        }
                        RemoteBranchProbeResult::Missing => {
                            return Err(RalphError::Validation(format!(
                                "cannot hard-rollback: project branch '{}' does not exist locally or on origin",
                                branch
                            )));
                        }
                        RemoteBranchProbeResult::ProbeFailed(stderr) => {
                            return Err(RalphError::Orchestration(format!(
                                "cannot hard-rollback: failed to probe remote for branch '{}': {}",
                                branch, stderr
                            )));
                        }
                    }
                }
            } else {
                println!(
                    "dry-run (hard rollback): would remove loops {:?} and set current loop to {} (no git repo)",
                    to_remove, args.loop_number
                );
            }
        } else {
            println!(
                "dry-run (soft rollback): would remove loops {:?} and set current loop to {} (no git reset)",
                to_remove, args.loop_number
            );
        }
        return Ok(());
    }

    #[derive(PartialEq)]
    enum PushOutcome {
        Succeeded,
        Failed,
        Skipped,
    }
    let mut push_outcome = PushOutcome::Skipped;

    // For hard rollback: recover/create branch -> checkout -> resolve ref -> reset -> push.
    // The ref is resolved AFTER branch recovery so that checkpoint commits on the
    // project branch are visible to `resolve_hard_reset_ref`.
    let hard_ref = if args.hard {
        let repo_root = workspace.root.parent().ok_or_else(|| {
            RalphError::Orchestration("workspace root has no parent path".to_owned())
        })?;
        if is_git_repo(repo_root) {
            // Ensure we reset on the project's branch (not an unrelated branch
            // that happens to be checked out).
            let branch = resolve_branch_name(&workspace.config.git.branch_format, &project_id);
            if !branch_exists(repo_root, &branch)? {
                // Always check the actual remote (authoritative) — local
                // tracking refs can be stale if the branch was deleted upstream.
                match remote_branch_exists_on_remote(repo_root, &branch)? {
                    RemoteBranchProbeResult::Exists => {
                        run_git(repo_root, &["fetch", "origin", &branch])?;
                        create_branch(repo_root, &branch, &format!("origin/{branch}"))?;
                    }
                    RemoteBranchProbeResult::Missing => {
                        return Err(RalphError::Validation(format!(
                            "cannot hard-rollback: project branch '{}' does not exist locally or on origin",
                            branch
                        )));
                    }
                    RemoteBranchProbeResult::ProbeFailed(stderr) => {
                        return Err(RalphError::Orchestration(format!(
                            "cannot hard-rollback: failed to probe remote for branch '{}': {}",
                            branch, stderr
                        )));
                    }
                }
            }
            checkout_branch(repo_root, &branch)?;

            // Resolve the hard reset ref AFTER branch recovery so that
            // checkpoint commits are visible.
            let reference = resolve_hard_reset_ref(
                &workspace,
                &original_state,
                &project_id,
                args.loop_number,
                repo_root,
            )?;

            // Reset the local branch so that checkpoint-derived state
            // reconstruction sees the rolled-back position.
            reset_hard(repo_root, &reference)?;
            restore_workspace_files(
                &workspace,
                &project_id,
                prompt_backup.as_deref(),
                project_config_backup.as_deref(),
            )?;

            // Force-push the reset branch so that checkpoint-derived state
            // reconstruction (which may read `origin/<branch>`) sees the
            // rolled-back position.  Without this the remote would retain
            // stale checkpoint commits and reconstruction would undo the rollback.
            let branch = resolve_branch_name(&workspace.config.git.branch_format, &project_id);
            if branch_exists(repo_root, &branch)? {
                if let Err(e) = run_git(
                    repo_root,
                    &["push", "--force", "origin", &format!("{branch}:{branch}")],
                ) {
                    eprintln!("warning: force-push failed: {e}");
                    push_outcome = PushOutcome::Failed;
                } else {
                    push_outcome = PushOutcome::Succeeded;
                }
            } else {
                eprintln!(
                    "warning: force-push skipped — branch '{}' does not exist",
                    branch
                );
            }

            Some(reference)
        } else {
            None
        }
    } else {
        None
    };

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

    // Manage the .rollback-ceiling marker.
    // Include the latest checkpoint hash for provenance-based staleness
    // detection (see P1 in lifecycle.rs).
    let ceiling_marker_content = match &latest_checkpoint_hash {
        Some(hash) => format!("{}\n{}", args.loop_number, hash),
        None => args.loop_number.to_string(),
    };
    let ceiling_path = project_dir.join(".rollback-ceiling");
    if let Some(reference) = &hard_ref {
        if push_outcome == PushOutcome::Succeeded {
            // Hard rollback succeeded fully — remove any stale ceiling marker.
            let _ = fs::remove_file(&ceiling_path);
            println!(
                "rolled back project {} to loop {} and reset git to {}",
                project_id, args.loop_number, reference
            );
        } else {
            // Push failed or was skipped — retain (or write) the ceiling marker
            // to guard against checkpoint resurrection from the remote on next
            // reconstruction.
            let reason = if push_outcome == PushOutcome::Failed {
                "force-push failed"
            } else {
                "force-push skipped"
            };
            fs::write(&ceiling_path, &ceiling_marker_content)?;
            println!(
                "rolled back project {} to loop {} and reset git to {} (warning: {}; .rollback-ceiling marker retained)",
                project_id, args.loop_number, reference, reason
            );
        }
    } else {
        // Soft rollback — write the ceiling marker so that
        // reconstruct_project_state caps checkpoint-derived position.
        fs::write(&ceiling_path, &ceiling_marker_content)?;
        println!(
            "soft-rolled back project {} to loop {} (no git reset)",
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

    if target_feature.is_none() && target_completion.is_none() && target_loop_number != 0 {
        return Err(RalphError::Validation(format!(
            "loop {} does not exist",
            target_loop_number
        )));
    }

    // Prefer checkpoint commit for the target loop — this is authoritative
    // and works for both legacy-tagged and checkpoint-only loops, avoiding
    // the risk of find_prior_tag returning an older loop's tag.
    let branch = resolve_branch_name(&workspace.config.git.branch_format, project_id);
    if let Some(hash) = find_checkpoint_commit(repo_root, &branch, project_id, target_loop_number)?
    {
        return Ok(hash);
    }

    // Fall back to tag-based resolution.
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
        None
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

/// Search the remote project branch for the most recent ralph checkpoint commit
/// at or before `target_loop_number` and return its hash.
fn find_checkpoint_commit(
    repo_root: &Path,
    branch: &str,
    project_id: &str,
    target_loop_number: u32,
) -> Result<Option<String>> {
    let commits = list_ralph_commits(repo_root, branch, project_id)?;
    for commit in &commits {
        if commit.loop_number <= target_loop_number {
            if let Some(hash) = commit.commit_hash.as_deref() {
                return Ok(Some(hash.to_owned()));
            }
        }
    }
    Ok(None)
}
