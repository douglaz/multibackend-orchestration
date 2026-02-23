use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::config::{effective_completion_consensus, GlobalConfig, ProjectConfig};
use crate::git::branch::{branch_exists, create_branch, remote_ref_exists, resolve_branch_name};
use crate::git::is_git_repo;
use crate::git::ralph_commit::{derive_position, list_ralph_commits};
use crate::project::artifacts::parse_artifact_filename_timestamp;
use crate::project::state::{
    AcceptanceQaResult, CompletionLoopArtifacts, CompletionLoopBackends, CompletionLoopState,
    CompletionVerdict, FeatureLoopArtifacts, FeatureLoopBackends, FeatureLoopState, LoopStatus,
    LoopType, Phase, ProjectState, ProjectStatus, QaExchange, ReviewExchange,
};
use crate::util::hash::sha256_hex;
use crate::util::lock::ProjectLock;
use crate::workspace::Workspace;
use crate::{error::RalphError, Result};

pub enum PromptSource {
    File(PathBuf),
    ParentProject(String),
}

pub struct CreateProjectOptions {
    pub id: String,
    pub name: String,
    pub source: PromptSource,
    pub starting_backend: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ProjectGitContext {
    pub repo_root: PathBuf,
    pub branch: String,
}

#[derive(Debug, Clone)]
struct ArtifactEntry {
    rel_path: String,
    file_name: String,
    base_name: String,
    frontmatter: BTreeMap<String, String>,
    body: String,
    observed_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct ProjectMetadata {
    name: Option<String>,
    parent_project: Option<String>,
    created_at: Option<String>,
}

pub fn create_project(workspace: &Workspace, options: CreateProjectOptions) -> Result<()> {
    let id = options.id;
    let _name = options.name;

    if workspace.project_exists(&id) {
        return Err(RalphError::Validation(format!(
            "project '{id}' already exists"
        )));
    }

    validate_project_id(&id)?;

    let (prompt_content, parent_project) = match options.source {
        PromptSource::File(path) => {
            if !path.exists() {
                return Err(RalphError::Validation(format!(
                    "prompt file not found: '{}'\n\
                     hint: --prompt expects a file path (e.g., --prompt PLAN.md)",
                    path.display()
                )));
            }
            (fs::read_to_string(&path)?, None)
        }
        PromptSource::ParentProject(parent_id) => {
            let parent_dir = workspace.project_dir(&parent_id);
            if !parent_dir.exists() {
                return Err(RalphError::ProjectNotFound(parent_id));
            }
            let content = fs::read_to_string(parent_dir.join("prompt.md"))?;
            (content, Some(parent_id))
        }
    };

    let project_dir = workspace.project_dir(&id);
    fs::create_dir_all(&project_dir)?;
    let _lock = ProjectLock::acquire(&project_dir, &id)?;
    fs::create_dir_all(project_dir.join("loops"))?;
    fs::write(project_dir.join("prompt.md"), &prompt_content)?;
    let metadata = ProjectMetadata {
        name: Some(_name),
        parent_project: parent_project.clone(),
        created_at: Some(Utc::now().to_rfc3339()),
    };
    fs::write(
        project_dir.join("project.toml"),
        toml::to_string_pretty(&metadata)?,
    )?;

    let _prompt_hash = sha256_hex(&prompt_content);

    if let Some(starting_backend) = options.starting_backend {
        let mut project_config = ProjectConfig::default();
        project_config.workflow.starting_backend = Some(starting_backend);
        project_config.save(&project_dir.join("config.toml"))?;
    }

    maybe_create_project_branch(workspace, &id, parent_project.as_deref())?;

    // Auto-activate if no local active project is set.
    if workspace.active_project_id().is_none() {
        workspace.set_active_project_id(&id)?;
    }

    Ok(())
}

pub fn project_git_context(workspace: &Workspace, project_id: &str) -> Option<ProjectGitContext> {
    let repo_root = workspace.root.parent()?.to_path_buf();
    if !is_git_repo(&repo_root) {
        return None;
    }

    Some(ProjectGitContext {
        repo_root,
        branch: resolve_branch_name(&workspace.config.git.branch_format, project_id),
    })
}

pub fn reconstruct_project_state(workspace: &Workspace, project_id: &str) -> Result<ProjectState> {
    let project_dir = workspace.project_dir(project_id);
    let git = project_git_context(workspace, project_id);
    reconstruct_project_state_internal(
        &project_dir,
        project_id,
        git.as_ref().map(|ctx| ctx.repo_root.as_path()),
        git.as_ref().map(|ctx| ctx.branch.as_str()),
        Some(&workspace.config),
    )
}

pub fn reconstruct_project_state_from_project_dir(project_dir: &Path) -> Result<ProjectState> {
    let project_id = project_dir
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| {
            RalphError::Orchestration(format!(
                "cannot infer project id from path '{}'",
                project_dir.display()
            ))
        })?;

    let repo_root = find_repo_root(project_dir);

    // Try loading workspace config for branch_format and global completion
    // settings.  The project dir lives at `.ralph/projects/<id>/`, so the
    // workspace root is two levels up.
    let workspace = project_dir
        .parent()
        .and_then(|p| p.parent())
        .and_then(|ws_root| Workspace::load(ws_root.to_path_buf()).ok());

    let branch = workspace
        .as_ref()
        .map(|ws| resolve_branch_name(&ws.config.git.branch_format, project_id))
        .unwrap_or_else(|| format!("ralph/{project_id}"));

    let global_config = workspace.as_ref().map(|ws| &ws.config);

    reconstruct_project_state_internal(
        project_dir,
        project_id,
        repo_root.as_deref(),
        Some(branch.as_str()),
        global_config,
    )
}

pub fn parse_issue_number(project_id: &str) -> Option<u32> {
    project_id.strip_prefix("issue-")?.parse::<u32>().ok()
}

pub fn parse_github_repo_slug(repo_root: &Path) -> Option<(String, String)> {
    let output = std::process::Command::new("git")
        .args(["remote", "get-url", "origin"])
        .current_dir(repo_root)
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let mut remote = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    if remote.is_empty() {
        return None;
    }

    if let Some(rest) = remote.strip_prefix("git@github.com:") {
        remote = rest.to_owned();
    } else if let Some(rest) = remote.strip_prefix("ssh://git@github.com/") {
        remote = rest.to_owned();
    } else if let Some(rest) = remote.strip_prefix("https://github.com/") {
        remote = rest.to_owned();
    } else {
        return None;
    }

    let remote = remote.trim_end_matches('/').trim_end_matches(".git");
    let (owner, repo) = remote.split_once('/')?;
    if owner.is_empty() || repo.is_empty() {
        return None;
    }

    Some((owner.to_owned(), repo.to_owned()))
}

fn reconstruct_project_state_internal(
    project_dir: &Path,
    project_id: &str,
    repo_root: Option<&Path>,
    branch: Option<&str>,
    global_config: Option<&GlobalConfig>,
) -> Result<ProjectState> {
    let prompt_path = project_dir.join("prompt.md");
    let prompt_content = fs::read_to_string(&prompt_path).unwrap_or_default();
    let prompt_hash = sha256_hex(&prompt_content);

    let metadata = read_project_metadata(project_dir).unwrap_or_default();
    let project_name = metadata.name.unwrap_or_else(|| project_id.to_owned());
    let parent_project = metadata.parent_project;

    let mut state = ProjectState::new(project_id, &project_name, &prompt_hash, parent_project);
    if let Some(created_at) = metadata.created_at.and_then(|raw| parse_rfc3339_utc(&raw)) {
        state.created_at = created_at;
    }
    // Use the persisted prompt hash (from the last orchestrator run) as the
    // baseline so that prompt edits between runs are detectable by
    // `handle_prompt_change`.  Fall back to the current file hash for fresh
    // projects or when the sentinel file is absent.
    let persisted = project_dir.join(".last-prompt-hash");
    let baseline_prompt_hash = fs::read_to_string(&persisted)
        .ok()
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| prompt_hash.clone());
    state.prompt_hash = baseline_prompt_hash.clone();
    state.prompt_hash_at_loop_start = baseline_prompt_hash;

    let (checkpoint_loop, checkpoint_phase, checkpoint_commits) = match (repo_root, branch) {
        (Some(root), Some(branch_name)) if is_git_repo(root) => {
            let (loop_number, phase) = derive_position(root, branch_name)?;
            let commits = list_ralph_commits(root, branch_name)?;
            (loop_number, phase, commits)
        }
        _ => (1, Phase::Planning, Vec::new()),
    };

    let mut commit_by_loop: HashMap<u32, String> = HashMap::new();
    for commit in &checkpoint_commits {
        if let Some(hash) = &commit.commit_hash {
            commit_by_loop
                .entry(commit.loop_number)
                .or_insert_with(|| hash.clone());
        }
    }

    let mut loop_dirs = collect_loop_directories(project_dir)?;
    loop_dirs.sort_by_key(|(number, _, _)| *number);

    // Compute effective completion consensus parameters by merging global
    // workflow defaults with optional project overrides — matching the same
    // resolution that `resolve_effective_config` applies at runtime.  When no
    // global config is available (e.g. standalone project dir reconstruction)
    // fall back to GlobalConfig::default().
    let project_config = ProjectConfig::load(&project_dir.join("config.toml")).ok();
    let fallback_global = GlobalConfig::default();
    let effective_global = global_config.unwrap_or(&fallback_global);
    let (completion_min_completers, completion_consensus_threshold) =
        effective_completion_consensus(effective_global, project_config.as_ref());

    for (loop_number, loop_slug, loop_path) in loop_dirs {
        let artifacts = collect_loop_artifacts(project_dir, &loop_path)?;
        if loop_slug == "completion" {
            let completion = reconstruct_completion_attempt(
                loop_number,
                artifacts,
                completion_min_completers,
                completion_consensus_threshold,
            );
            state.completion_attempts.push(completion);
        } else {
            let feature = reconstruct_feature_loop(
                loop_number,
                &loop_slug,
                artifacts,
                commit_by_loop.get(&loop_number).cloned(),
            );
            state.loops.push(feature);
        }
    }

    state.prompt_review_completed =
        state_has_prompt_review(project_dir) || state.last_loop_number() > 0;

    // Position is always derived from checkpoint commits (no artifact-based
    // fallback).  When no checkpoint exists, derive_position defaults to
    // loop=1, phase=planning.  The loop 1→0 remap is removed.
    state.current_loop = checkpoint_loop;
    state.current_phase = checkpoint_phase.clone();

    // When a real checkpoint exists and the phase is not Planning, the
    // current loop is still actively mid-workflow — override any
    // artifact-based Completed status.  For example, review-approved.md may
    // exist while the checkpoint phase is Committing (after --until-review);
    // marking the loop as Completed would make has_in_progress_loop() false
    // and the orchestrator would skip finishing the commit phase on resume.
    //
    // Skip this override when checkpoint_phase is Planning, because the
    // "committing -> planning" transition proves the loop completed.  Also
    // skip in non-git contexts where checkpoint_commits is empty.
    if !checkpoint_commits.is_empty() && checkpoint_phase != Phase::Planning {
        if let Some(current) = state
            .loops
            .iter_mut()
            .find(|l| l.loop_number == checkpoint_loop)
        {
            if current.status == LoopStatus::Completed {
                current.status = LoopStatus::InProgress;
            }
        }
    }

    state.phase_iteration = infer_phase_iteration(&state);

    if state
        .completion_attempts
        .iter()
        .any(|attempt| attempt.verdict == Some(CompletionVerdict::Complete))
    {
        state.status = ProjectStatus::Completed;
    } else if state.last_loop_number() == 0 {
        state.status = ProjectStatus::Pending;
    } else {
        state.status = ProjectStatus::InProgress;
    }

    Ok(state)
}

fn maybe_create_project_branch(
    workspace: &Workspace,
    project_id: &str,
    parent_project: Option<&str>,
) -> Result<()> {
    if !workspace.config.git.auto_branch {
        return Ok(());
    }

    let Some(repo_root) = workspace.root.parent() else {
        return Ok(());
    };

    if !is_git_repo(repo_root) {
        return Ok(());
    }

    let branch_name = resolve_branch_name(&workspace.config.git.branch_format, project_id);
    if branch_exists(repo_root, &branch_name)? {
        return Err(RalphError::Validation(format!(
            "git branch '{}' already exists",
            branch_name
        )));
    }

    let from_ref = if let Some(parent_id) = parent_project {
        resolve_branch_name(&workspace.config.git.branch_format, parent_id)
    } else {
        let remote_ref = format!("origin/{}", workspace.config.git.base_branch);
        if remote_ref_exists(repo_root, &remote_ref)? {
            remote_ref
        } else {
            // Empty remote (e.g. freshly bootstrapped repo) — use local base.
            workspace.config.git.base_branch.clone()
        }
    };

    create_branch(repo_root, &branch_name, &from_ref)?;
    Ok(())
}

pub(crate) fn validate_project_id(id: &str) -> Result<()> {
    if id.is_empty() {
        return Err(RalphError::Validation(
            "project id cannot be empty".to_owned(),
        ));
    }

    if !id
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
    {
        return Err(RalphError::Validation(
            "project id may only contain [a-zA-Z0-9_-]".to_owned(),
        ));
    }

    Ok(())
}

fn collect_loop_directories(project_dir: &Path) -> Result<Vec<(u32, String, PathBuf)>> {
    let loops_dir = project_dir.join("loops");
    let entries = match fs::read_dir(&loops_dir) {
        Ok(entries) => entries,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(err) => return Err(err.into()),
    };

    let mut loop_dirs = Vec::new();

    for entry in entries {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }

        let name = entry.file_name().to_string_lossy().to_string();
        let Some((num, slug)) = name.split_once('-') else {
            continue;
        };
        let Ok(loop_number) = num.parse::<u32>() else {
            continue;
        };

        loop_dirs.push((loop_number, slug.to_owned(), entry.path()));
    }

    Ok(loop_dirs)
}

fn collect_loop_artifacts(project_dir: &Path, loop_dir: &Path) -> Result<Vec<ArtifactEntry>> {
    let entries = fs::read_dir(loop_dir)?;
    let mut out = Vec::new();

    for entry in entries {
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            continue;
        }

        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("md") {
            continue;
        }

        let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };

        let raw = fs::read_to_string(&path)?;
        let (frontmatter, body) = split_frontmatter(&raw);
        let modified = entry
            .metadata()
            .ok()
            .and_then(|meta| meta.modified().ok())
            .map(DateTime::<Utc>::from)
            .unwrap_or_else(Utc::now);
        let observed_at = frontmatter
            .get("created_at")
            .and_then(|created| parse_rfc3339_utc(created))
            .unwrap_or(modified);

        let base_name = normalize_artifact_basename(file_name);
        let rel_path = path
            .strip_prefix(project_dir)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/");

        out.push(ArtifactEntry {
            rel_path,
            file_name: file_name.to_owned(),
            base_name,
            frontmatter,
            body,
            observed_at,
        });
    }

    out.sort_by(|left, right| {
        left.observed_at
            .cmp(&right.observed_at)
            .then_with(|| left.file_name.cmp(&right.file_name))
    });

    Ok(out)
}

fn normalize_artifact_basename(file_name: &str) -> String {
    if let Some(prefix) = parse_artifact_filename_timestamp(file_name) {
        let expected = format!("{prefix}-");
        if let Some(stripped) = file_name.strip_prefix(&expected) {
            return stripped.to_owned();
        }
    }

    file_name.to_owned()
}

fn reconstruct_feature_loop(
    loop_number: u32,
    loop_slug: &str,
    artifacts: Vec<ArtifactEntry>,
    commit_hash: Option<String>,
) -> FeatureLoopState {
    let now = Utc::now();
    let started_at = artifacts
        .iter()
        .map(|artifact| artifact.observed_at)
        .min()
        .unwrap_or(now);

    let spec = latest_artifact(&artifacts, |artifact| artifact.base_name == "spec.md");
    let impl_notes = latest_artifact(&artifacts, |artifact| artifact.base_name == "impl-notes.md");
    let approval = latest_artifact(&artifacts, |artifact| {
        artifact.base_name == "review-approved.md"
    });

    let mut review_feedback: BTreeMap<u32, &ArtifactEntry> = BTreeMap::new();
    let mut impl_responses: BTreeMap<u32, &ArtifactEntry> = BTreeMap::new();
    let mut qa_reports: BTreeMap<u32, (&ArtifactEntry, bool)> = BTreeMap::new();
    let mut qa_responses: BTreeMap<u32, &ArtifactEntry> = BTreeMap::new();

    for artifact in &artifacts {
        if let Some(iteration) = parse_iteration(&artifact.base_name, "review-", "-feedback.md") {
            review_feedback.insert(iteration, artifact);
            continue;
        }

        if let Some(iteration) = parse_iteration(&artifact.base_name, "impl-response-", ".md") {
            impl_responses.insert(iteration, artifact);
            continue;
        }

        if let Some(iteration) = parse_iteration(&artifact.base_name, "qa-", "-pass.md") {
            qa_reports.insert(iteration, (artifact, true));
            continue;
        }

        if let Some(iteration) = parse_iteration(&artifact.base_name, "qa-", "-fail.md") {
            qa_reports.insert(iteration, (artifact, false));
            continue;
        }

        if let Some(iteration) = parse_iteration(&artifact.base_name, "impl-qa-response-", ".md") {
            qa_responses.insert(iteration, artifact);
        }
    }

    let mut reviews = Vec::new();
    for (iteration, feedback) in &review_feedback {
        if let Some(response) = impl_responses.get(iteration) {
            reviews.push(ReviewExchange {
                iteration: *iteration,
                feedback: feedback.rel_path.clone(),
                response: response.rel_path.clone(),
            });
        }
    }

    let mut qa_results = Vec::new();
    for (iteration, (report, passed)) in &qa_reports {
        qa_results.push(QaExchange {
            iteration: *iteration,
            passed: *passed,
            report: report.rel_path.clone(),
            implementer_response: qa_responses
                .get(iteration)
                .map(|entry| entry.rel_path.clone()),
        });
    }

    let pending_qa_feedback = qa_results
        .iter()
        .rev()
        .find(|result| !result.passed && result.implementer_response.is_none())
        .map(|result| result.report.clone());

    let spec_path = spec
        .map(|artifact| artifact.rel_path.clone())
        .unwrap_or_else(|| format!("loops/{loop_number:03}-{loop_slug}/spec.md"));

    let feature_name = spec
        .and_then(|artifact| first_feature_name(&artifact.body))
        .unwrap_or_else(|| loop_slug.replace('-', " "));

    let planner_backend = spec
        .and_then(|artifact| artifact.frontmatter.get("backend").cloned())
        .unwrap_or_else(|| "unknown".to_owned());
    let implementer_backend = impl_notes
        .and_then(|artifact| artifact.frontmatter.get("backend").cloned())
        .or_else(|| {
            artifacts
                .iter()
                .rev()
                .find(|artifact| {
                    artifact.base_name.starts_with("impl-response-")
                        || artifact.base_name.starts_with("impl-qa-response-")
                })
                .and_then(|artifact| artifact.frontmatter.get("backend").cloned())
        })
        .unwrap_or_else(|| "unknown".to_owned());
    let reviewer_backend = approval
        .and_then(|artifact| artifact.frontmatter.get("backend").cloned())
        .or_else(|| {
            artifacts
                .iter()
                .rev()
                .find(|artifact| artifact.base_name.starts_with("review-"))
                .and_then(|artifact| artifact.frontmatter.get("backend").cloned())
        })
        .unwrap_or_else(|| "unknown".to_owned());
    let qa_backend = artifacts
        .iter()
        .rev()
        .find(|artifact| artifact.base_name.starts_with("qa-"))
        .and_then(|artifact| artifact.frontmatter.get("backend").cloned())
        .unwrap_or_else(|| "unknown".to_owned());

    let completed_at = approval.map(|artifact| artifact.observed_at);
    let status = if completed_at.is_some() {
        LoopStatus::Completed
    } else {
        LoopStatus::InProgress
    };

    FeatureLoopState {
        loop_number,
        slug: loop_slug.to_owned(),
        feature_name,
        loop_type: LoopType::Feature,
        status,
        backends: FeatureLoopBackends {
            planner: planner_backend,
            implementer: implementer_backend,
            reviewer: reviewer_backend,
            qa: qa_backend,
        },
        artifacts: FeatureLoopArtifacts {
            spec: spec_path,
            impl_notes: impl_notes.map(|artifact| artifact.rel_path.clone()),
            reviews,
            approval: approval.map(|artifact| artifact.rel_path.clone()),
            qa_results,
            pending_qa_feedback,
        },
        commit: commit_hash,
        started_at,
        completed_at,
    }
}

fn reconstruct_completion_attempt(
    loop_number: u32,
    artifacts: Vec<ArtifactEntry>,
    min_completers: u32,
    consensus_threshold: f64,
) -> CompletionLoopState {
    let now = Utc::now();
    let started_at = artifacts
        .iter()
        .map(|artifact| artifact.observed_at)
        .min()
        .unwrap_or(now);

    let termination = latest_artifact(&artifacts, |artifact| {
        artifact.base_name == "termination-request.md"
    });

    // Collect per-backend verdict artifacts (new panel layout).
    // Deduplicate by backend: if retries produced multiple verdict files for
    // the same backend, keep only the latest one per backend to avoid
    // inflating the consensus vote count.
    let per_backend_verdicts: Vec<&ArtifactEntry> = {
        let mut by_backend: std::collections::BTreeMap<String, &ArtifactEntry> =
            std::collections::BTreeMap::new();
        for artifact in &artifacts {
            if artifact.base_name.starts_with("completer-verdict-")
                && artifact.base_name != "completer-verdict.md"
            {
                let backend = artifact
                    .frontmatter
                    .get("backend")
                    .cloned()
                    .unwrap_or_else(|| artifact.base_name.clone());
                match by_backend.get(&backend) {
                    Some(prev) if artifact.observed_at > prev.observed_at => {
                        by_backend.insert(backend, artifact);
                    }
                    None => {
                        by_backend.insert(backend, artifact);
                    }
                    _ => {}
                }
            }
        }
        by_backend.into_values().collect()
    };

    // Legacy single verdict artifact.
    let legacy_verdict = latest_artifact(&artifacts, |artifact| {
        artifact.base_name == "completer-verdict.md"
    });

    // Determine completers and effective verdict.
    let (completers, effective_verdict_artifact, panel_verdict) =
        if !per_backend_verdicts.is_empty() {
            // New per-backend verdict layout: extract completers from each verdict artifact.
            let mut completers = Vec::new();
            let mut complete_votes: u32 = 0;
            let mut any_verdict = false;
            let mut latest_verdict: Option<&ArtifactEntry> = None;
            for v in &per_backend_verdicts {
                let backend = v
                    .frontmatter
                    .get("backend")
                    .cloned()
                    .unwrap_or_else(|| "unknown".to_owned());
                completers.push(backend);
                if let Some(pv) = parse_completion_verdict(&v.body) {
                    any_verdict = true;
                    if pv == CompletionVerdict::Complete {
                        complete_votes += 1;
                    }
                }
                latest_verdict = Some(match latest_verdict {
                    Some(prev) if v.observed_at > prev.observed_at => v,
                    Some(prev) => prev,
                    None => v,
                });
            }
            // Apply the same consensus formula as the runtime orchestrator:
            // complete_votes >= min_completers AND ratio >= threshold.
            let total = per_backend_verdicts.len() as u32;
            let verdict = if any_verdict {
                let consensus_reached = complete_votes >= min_completers
                    && total > 0
                    && (complete_votes as f64 / total as f64) >= consensus_threshold;
                if consensus_reached {
                    Some(CompletionVerdict::Complete)
                } else {
                    Some(CompletionVerdict::Continue)
                }
            } else {
                None
            };
            (completers, latest_verdict, verdict)
        } else if let Some(single) = legacy_verdict {
            // Legacy single-verdict layout: map to single completer.
            let backend = single
                .frontmatter
                .get("backend")
                .cloned()
                .unwrap_or_else(|| "unknown".to_owned());
            let verdict = parse_completion_verdict(&single.body);
            (vec![backend], Some(single), verdict)
        } else {
            (Vec::new(), None, None)
        };

    let mut acceptance_results = Vec::new();
    for artifact in &artifacts {
        if artifact.base_name.starts_with("acceptance-pass")
            || artifact.base_name == "acceptance-pass.md"
        {
            acceptance_results.push(AcceptanceQaResult {
                backend: artifact
                    .frontmatter
                    .get("backend")
                    .cloned()
                    .unwrap_or_else(|| "unknown".to_owned()),
                passed: true,
                artifact: artifact.rel_path.clone(),
            });
            continue;
        }

        if artifact.base_name.starts_with("acceptance-fail")
            || artifact.base_name == "acceptance-fail.md"
        {
            acceptance_results.push(AcceptanceQaResult {
                backend: artifact
                    .frontmatter
                    .get("backend")
                    .cloned()
                    .unwrap_or_else(|| "unknown".to_owned()),
                passed: false,
                artifact: artifact.rel_path.clone(),
            });
        }
    }

    // Apply acceptance gate: if the panel said COMPLETE but any acceptance
    // result failed, the effective verdict is CONTINUE.
    let panel_verdict = match panel_verdict {
        Some(CompletionVerdict::Complete) if acceptance_results.iter().any(|r| !r.passed) => {
            Some(CompletionVerdict::Continue)
        }
        other => other,
    };

    let completed_at = effective_verdict_artifact.map(|artifact| artifact.observed_at);

    let planner_backend = termination
        .and_then(|artifact| artifact.frontmatter.get("backend").cloned())
        .unwrap_or_else(|| "unknown".to_owned());

    CompletionLoopState {
        loop_number,
        slug: "completion".to_owned(),
        loop_type: LoopType::Completion,
        status: if panel_verdict.is_some() {
            LoopStatus::Completed
        } else {
            LoopStatus::InProgress
        },
        backends: CompletionLoopBackends::new(planner_backend, completers),
        artifacts: CompletionLoopArtifacts {
            termination_request: termination
                .map(|artifact| artifact.rel_path.clone())
                .unwrap_or_else(|| {
                    format!("loops/{loop_number:03}-completion/termination-request.md")
                }),
            verdict: effective_verdict_artifact.map(|artifact| artifact.rel_path.clone()),
            acceptance_results,
            acceptance_result: None,
            acceptance_passed: None,
        },
        verdict: panel_verdict,
        started_at,
        completed_at,
    }
}

fn infer_phase_iteration(state: &ProjectState) -> u32 {
    if state.current_loop == 0 {
        return 1;
    }

    let Some(feature_loop) = state
        .loops
        .iter()
        .find(|loop_state| loop_state.loop_number == state.current_loop)
    else {
        return 1;
    };

    match state.current_phase {
        Phase::Planning | Phase::Committing | Phase::Completing => 1,
        Phase::Implementing => {
            if let Some(pending) = &feature_loop.artifacts.pending_qa_feedback {
                return parse_iteration_from_path(pending, "qa-")
                    .or_else(|| {
                        feature_loop
                            .artifacts
                            .qa_results
                            .last()
                            .map(|qa| qa.iteration)
                    })
                    .unwrap_or(1);
            }

            feature_loop
                .artifacts
                .reviews
                .last()
                .map(|review| review.iteration + 1)
                .unwrap_or(1)
        }
        Phase::QA => feature_loop
            .artifacts
            .qa_results
            .last()
            .map(|qa| qa.iteration + 1)
            .unwrap_or(1),
        Phase::Reviewing => feature_loop
            .artifacts
            .reviews
            .last()
            .map(|review| review.iteration + 1)
            .unwrap_or(1),
        Phase::FinalReview => 1,
    }
}

fn parse_iteration(base_name: &str, prefix: &str, suffix: &str) -> Option<u32> {
    let middle = base_name.strip_prefix(prefix)?.strip_suffix(suffix)?;
    middle.parse::<u32>().ok()
}

fn parse_iteration_from_path(path: &str, prefix: &str) -> Option<u32> {
    let file_name = Path::new(path).file_name()?.to_str()?;
    let base_name = normalize_artifact_basename(file_name);
    let rest = base_name.strip_prefix(prefix)?;
    rest.split('-').next()?.parse::<u32>().ok()
}

fn split_frontmatter(raw: &str) -> (BTreeMap<String, String>, String) {
    let trimmed = raw.trim();
    if !trimmed.starts_with("---") {
        return (BTreeMap::new(), trimmed.to_owned());
    }

    let mut lines = trimmed.lines();
    if lines.next() != Some("---") {
        return (BTreeMap::new(), trimmed.to_owned());
    }

    let mut frontmatter = BTreeMap::new();
    let mut in_frontmatter = true;
    let mut body_lines = Vec::new();

    for line in lines {
        if in_frontmatter {
            if line.trim() == "---" {
                in_frontmatter = false;
                continue;
            }

            if let Some((key, value)) = line.split_once(':') {
                frontmatter.insert(key.trim().to_owned(), value.trim().to_owned());
            }
            continue;
        }

        body_lines.push(line);
    }

    if in_frontmatter {
        (BTreeMap::new(), trimmed.to_owned())
    } else {
        (frontmatter, body_lines.join("\n").trim().to_owned())
    }
}

fn parse_rfc3339_utc(value: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|value| value.with_timezone(&Utc))
}

fn first_feature_name(body: &str) -> Option<String> {
    let heading = body
        .lines()
        .find(|line| line.trim_start().starts_with("# Feature:"))?;
    heading
        .trim()
        .strip_prefix("# Feature:")
        .map(|name| name.trim().to_owned())
        .filter(|name| !name.is_empty())
}

fn parse_completion_verdict(body: &str) -> Option<CompletionVerdict> {
    let heading = body
        .lines()
        .find(|line| line.trim_start().starts_with("# Verdict:"))?
        .trim();

    match heading {
        "# Verdict: COMPLETE" => Some(CompletionVerdict::Complete),
        "# Verdict: CONTINUE" => Some(CompletionVerdict::Continue),
        _ => None,
    }
}

fn latest_artifact<F>(artifacts: &[ArtifactEntry], mut predicate: F) -> Option<&ArtifactEntry>
where
    F: FnMut(&ArtifactEntry) -> bool,
{
    artifacts.iter().rev().find(|artifact| predicate(artifact))
}

fn state_has_prompt_review(project_dir: &Path) -> bool {
    // A completed prompt review always writes both the canonical review
    // artifact and the original prompt backup.  Requiring both prevents
    // validator-rejected runs (which may still write prompt-review.md) from
    // being reconstructed as completed.
    project_dir.join("prompt-review.md").exists() && project_dir.join("prompt-original.md").exists()
}

fn find_repo_root(start: &Path) -> Option<PathBuf> {
    let output = std::process::Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(start)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }

    let root = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    if root.is_empty() {
        None
    } else {
        Some(PathBuf::from(root))
    }
}

fn read_project_metadata(project_dir: &Path) -> Option<ProjectMetadata> {
    let path = project_dir.join("project.toml");
    let raw = fs::read_to_string(path).ok()?;
    toml::from_str(&raw).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    /// Helper: write a minimal verdict artifact in the completion loop directory.
    fn write_verdict_artifact(
        project_dir: &Path,
        loop_number: u32,
        file_name: &str,
        backend: &str,
        verdict: &str,
    ) {
        let loop_dir = project_dir
            .join("loops")
            .join(format!("{loop_number:03}-completion"));
        fs::create_dir_all(&loop_dir).unwrap();
        let body = format!(
            "---\nartifact: completer-verdict\nloop: {loop_number}\nproject: test\nbackend: {backend}\nrole: completer\ncreated_at: 2026-01-01T00:00:00Z\n---\n\n# Verdict: {verdict}\n"
        );
        fs::write(loop_dir.join(file_name), body).unwrap();
    }

    /// Helper: write a termination-request artifact in the completion loop directory.
    fn write_termination_artifact(project_dir: &Path, loop_number: u32) {
        let loop_dir = project_dir
            .join("loops")
            .join(format!("{loop_number:03}-completion"));
        fs::create_dir_all(&loop_dir).unwrap();
        let body = format!(
            "---\nartifact: termination-request\nloop: {loop_number}\nproject: test\nbackend: claude\nrole: planner\ncreated_at: 2026-01-01T00:00:00Z\n---\n\n# Project Completion Request\n"
        );
        fs::write(loop_dir.join("20260101000000-termination-request.md"), body).unwrap();
    }

    #[test]
    fn reconstruction_uses_global_completion_threshold_override() {
        // When the global config sets completion_consensus_threshold=0.5 and
        // the project config has no override, reconstruction should use 0.5
        // (not a hardcoded default of 1.0).  With threshold=0.5 and
        // min_completers=1, 1/2 COMPLETE votes should yield consensus.
        let temp = TempDir::new().unwrap();
        let project_dir = temp.path().join("projects").join("test-proj");
        fs::create_dir_all(&project_dir).unwrap();

        // Write prompt so reconstruction doesn't fail
        fs::write(project_dir.join("prompt.md"), "test prompt").unwrap();
        // Write project.toml metadata
        fs::write(project_dir.join("project.toml"), "name = \"test-proj\"\n").unwrap();
        // No project config.toml → no project-level completion overrides

        // Write per-backend verdict artifacts: one COMPLETE, one CONTINUE
        write_termination_artifact(&project_dir, 1);
        write_verdict_artifact(
            &project_dir,
            1,
            "20260101000001-completer-verdict-claude.md",
            "claude",
            "COMPLETE",
        );
        write_verdict_artifact(
            &project_dir,
            1,
            "20260101000002-completer-verdict-codex.md",
            "codex",
            "CONTINUE",
        );

        // Create a global config with threshold=0.5, min_completers=1
        let mut global = GlobalConfig::default();
        global.workflow.completion_consensus_threshold = 0.5;
        global.workflow.completion_min_completers = 1;

        let state = reconstruct_project_state_internal(
            &project_dir,
            "test-proj",
            None,
            None,
            Some(&global),
        )
        .expect("reconstruction should succeed");

        assert_eq!(state.completion_attempts.len(), 1);
        let attempt = &state.completion_attempts[0];
        // With threshold=0.5 and min=1: 1/2 COMPLETE >= 0.5 and 1 >= 1 → COMPLETE
        assert_eq!(
            attempt.verdict,
            Some(CompletionVerdict::Complete),
            "reconstruction with global threshold=0.5 should yield Complete for 1/2 votes"
        );
    }

    #[test]
    fn reconstruction_project_override_takes_precedence_over_global() {
        // When the global config sets threshold=0.5 but the project config
        // sets threshold=1.0, reconstruction should use 1.0 and yield
        // CONTINUE for 1/2 COMPLETE votes.
        let temp = TempDir::new().unwrap();
        let project_dir = temp.path().join("projects").join("test-proj2");
        fs::create_dir_all(&project_dir).unwrap();

        fs::write(project_dir.join("prompt.md"), "test prompt").unwrap();
        fs::write(project_dir.join("project.toml"), "name = \"test-proj2\"\n").unwrap();
        // Project config overrides threshold to 1.0
        fs::write(
            project_dir.join("config.toml"),
            "[workflow]\ncompletion_consensus_threshold = 1.0\ncompletion_min_completers = 2\n",
        )
        .unwrap();

        write_termination_artifact(&project_dir, 1);
        write_verdict_artifact(
            &project_dir,
            1,
            "20260101000001-completer-verdict-claude.md",
            "claude",
            "COMPLETE",
        );
        write_verdict_artifact(
            &project_dir,
            1,
            "20260101000002-completer-verdict-codex.md",
            "codex",
            "CONTINUE",
        );

        let mut global = GlobalConfig::default();
        global.workflow.completion_consensus_threshold = 0.5;
        global.workflow.completion_min_completers = 1;

        let state = reconstruct_project_state_internal(
            &project_dir,
            "test-proj2",
            None,
            None,
            Some(&global),
        )
        .expect("reconstruction should succeed");

        assert_eq!(state.completion_attempts.len(), 1);
        let attempt = &state.completion_attempts[0];
        // Project override: threshold=1.0 and min=2 → 1/2 COMPLETE < 1.0 and 1 < 2 → CONTINUE
        assert_eq!(
            attempt.verdict,
            Some(CompletionVerdict::Continue),
            "project override threshold=1.0 should yield Continue for 1/2 votes"
        );
    }
}
