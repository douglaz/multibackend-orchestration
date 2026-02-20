use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::io::ErrorKind;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use chrono::{DateTime, NaiveDateTime, SecondsFormat, Utc};
use serde::Serialize;
use tokio::time::{sleep, Duration};

use crate::git::ralph_commit::{derive_position, parse_last_ralph_commit};
use crate::cli::TailArgs;
use crate::project::artifacts::parse_artifact_filename_timestamp;
use crate::project::lifecycle::{project_git_context, reconstruct_project_state};
use crate::project::state::{CompletionVerdict, LoopStatus, Phase, ProjectStatus};
use crate::workspace::Workspace;
use crate::{error::RalphError, Result};

const EVENT_ORDER_ARTIFACT: u8 = 0;
const EVENT_ORDER_STATE: u8 = 1;
const EVENT_ORDER_GIT: u8 = 2;

#[derive(Debug, Clone)]
enum TailEventKind {
    Artifact {
        rel_path: String,
        filename_timestamp: Option<String>,
        created_at: Option<String>,
        artifact: Option<String>,
        loop_number: Option<u32>,
        iteration: Option<u32>,
        role: Option<String>,
        backend: Option<String>,
        heading: Option<String>,
        body: String,
    },
    StateTransition {
        description: String,
        loop_number: Option<u32>,
        phase: Option<String>,
    },
    GitCommit {
        loop_number: u32,
        commit_hash: String,
        tag: Option<String>,
    },
}

impl TailEventKind {
    fn event_type(&self) -> &'static str {
        match self {
            Self::Artifact { .. } => "artifact",
            Self::StateTransition { .. } => "state",
            Self::GitCommit { .. } => "git",
        }
    }
}

#[derive(Debug, Clone)]
struct TailEvent {
    timestamp: String,
    sort_epoch_ms: i64,
    sort_tiebreaker: u8,
    signature: String,
    kind: TailEventKind,
}

impl TailEvent {
    fn event_key(&self) -> String {
        self.signature.clone()
    }
}

#[derive(Debug, Clone)]
struct PhaseSnapshot {
    current_phase: Phase,
    current_loop: u32,
    timestamp: String,
    sort_epoch_ms: i64,
    checkpoint_hash: Option<String>,
}

#[derive(Debug, Serialize)]
struct TailEventOutput<'a> {
    project_id: &'a str,
    event_type: &'a str,
    timestamp: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    path: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    filename_timestamp: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    created_at: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    artifact: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    loop_number: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    iteration: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    role: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    backend: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    heading: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    body: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    phase: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    commit_hash: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tag: Option<&'a str>,
}

pub async fn execute(args: TailArgs) -> Result<()> {
    if args.tmux {
        return execute_tmux_attach().await;
    }

    let workspace = Workspace::discover()?;
    let project_id = workspace.resolve_project_id(args.project.as_deref())?;

    let project_dir = workspace.project_dir(&project_id);
    if !project_dir.exists() {
        return Err(RalphError::ProjectNotFound(project_id));
    }

    let git_context = project_git_context(&workspace, &project_id);

    // Warn if showing a completed project (may be stale)
    if let Ok(state) = reconstruct_project_state(&workspace, &project_id) {
        if state.status == ProjectStatus::Completed {
            eprintln!(
                "warning: project '{}' is completed. Output may be stale.",
                project_id
            );
        }
    }

    let mut events = collect_all_events(&workspace, &project_id)?;
    sort_events(&mut events);

    let mut seen: HashSet<String> = events.iter().map(TailEvent::event_key).collect();
    print_events(&project_id, &events, args.last, args.json)?;

    if !args.follow {
        return Ok(());
    }

    let mut phase_snapshot = collect_phase_snapshot(git_context.as_ref());

    loop {
        sleep(Duration::from_millis(args.poll_interval_ms)).await;

        let mut rescanned = collect_all_events(&workspace, &project_id)?;
        let current_phase_snapshot = collect_phase_snapshot(git_context.as_ref());
        if let (Some(previous), Some(current)) = (&phase_snapshot, &current_phase_snapshot) {
            if let Some(phase_event) = create_phase_change_event(previous, current) {
                rescanned.push(phase_event);
            }
        }
        phase_snapshot = current_phase_snapshot;

        let mut new_events = Vec::new();
        for event in rescanned {
            if seen.insert(event.event_key()) {
                new_events.push(event);
            }
        }
        sort_events(&mut new_events);

        if !new_events.is_empty() {
            print_events(&project_id, &new_events, None, args.json)?;
        }
    }
}

async fn execute_tmux_attach() -> Result<()> {
    let workspace = Workspace::discover()?;
    let session_name = &workspace.config.workspace.tmux_session;
    tmux_attach(session_name).await
}

/// Attach to a tmux session by name. Validates that tmux is available and
/// the session exists before attempting to attach.
pub async fn tmux_attach(session_name: &str) -> Result<()> {
    // Check if tmux is available
    crate::backend::tmux::check_tmux_available().map_err(|_| {
        RalphError::Validation(
            "tmux is not installed or not on PATH; cannot attach to tmux session".to_owned(),
        )
    })?;

    // Check if the session exists
    let session_exists = {
        let output = tokio::process::Command::new("tmux")
            .args(["has-session", "-t", session_name])
            .output()
            .await
            .map_err(|err| RalphError::Validation(format!("failed to run tmux: {err}")))?;
        output.status.success()
    };

    if !session_exists {
        return Err(RalphError::Validation(format!(
            "tmux session '{session_name}' does not exist. \
             Start a run with `ralph run --tmux` to create it."
        )));
    }

    // Run tmux attach as a child process (exec would be ideal but isn't
    // portable / testable; a blocking child process serves the same purpose).
    let status = std::process::Command::new("tmux")
        .args(["attach", "-t", session_name])
        .status()
        .map_err(|err| {
            RalphError::Validation(format!(
                "failed to attach to tmux session '{session_name}': {err}"
            ))
        })?;

    if !status.success() {
        return Err(RalphError::Validation(format!(
            "tmux attach to session '{session_name}' exited with non-zero status"
        )));
    }

    Ok(())
}

fn collect_all_events(workspace: &Workspace, project_id: &str) -> Result<Vec<TailEvent>> {
    let project_dir = workspace.project_dir(project_id);
    let mut events = collect_artifact_events(&project_dir)?;
    events.extend(collect_state_events(workspace, project_id));
    Ok(events)
}

fn print_events(
    project_id: &str,
    events: &[TailEvent],
    last: Option<usize>,
    as_json: bool,
) -> Result<()> {
    let start_idx = last
        .map(|count| events.len().saturating_sub(count))
        .unwrap_or(0);
    let selected = &events[start_idx..];

    for event in selected {
        if as_json {
            let out = event_output(project_id, event);
            println!("{}", serde_json::to_string(&out)?);
        } else {
            print_human_event(event);
        }
    }

    Ok(())
}

fn print_human_event(event: &TailEvent) {
    match &event.kind {
        TailEventKind::Artifact {
            rel_path,
            artifact,
            loop_number,
            iteration,
            role,
            backend,
            body,
            ..
        } => {
            println!("--- [{}] {rel_path} ---", event.timestamp);
            let mut meta = Vec::new();
            push_meta_str(&mut meta, "artifact", artifact.as_deref());
            push_meta_u32(&mut meta, "loop", *loop_number);
            push_meta_u32(&mut meta, "iteration", *iteration);
            push_meta_str(&mut meta, "role", role.as_deref());
            push_meta_str(&mut meta, "backend", backend.as_deref());
            if !meta.is_empty() {
                println!("{}", meta.join("  "));
            }
            if !body.is_empty() {
                println!();
                println!("{body}");
            }
            println!();
        }
        TailEventKind::StateTransition {
            description,
            loop_number,
            phase,
        } => {
            println!("--- [{}] state ---", event.timestamp);
            println!("{description}");
            let mut meta = Vec::new();
            push_meta_u32(&mut meta, "loop", *loop_number);
            push_meta_str(&mut meta, "phase", phase.as_deref());
            if !meta.is_empty() {
                println!("{}", meta.join("  "));
            }
            println!();
        }
        TailEventKind::GitCommit {
            loop_number,
            commit_hash,
            tag,
        } => {
            println!("--- [{}] git ---", event.timestamp);
            if let Some(tag) = tag.as_deref() {
                println!("loop {loop_number} committed: {commit_hash} (tag: {tag})");
            } else {
                println!("loop {loop_number} committed: {commit_hash}");
            }
            println!();
        }
    }
}

fn event_output<'a>(project_id: &'a str, event: &'a TailEvent) -> TailEventOutput<'a> {
    match &event.kind {
        TailEventKind::Artifact {
            rel_path,
            filename_timestamp,
            created_at,
            artifact,
            loop_number,
            iteration,
            role,
            backend,
            heading,
            body,
        } => TailEventOutput {
            project_id,
            event_type: event.kind.event_type(),
            timestamp: &event.timestamp,
            path: Some(rel_path),
            filename_timestamp: filename_timestamp.as_deref(),
            created_at: created_at.as_deref(),
            artifact: artifact.as_deref(),
            loop_number: *loop_number,
            iteration: *iteration,
            role: role.as_deref(),
            backend: backend.as_deref(),
            heading: heading.as_deref(),
            body: Some(body),
            description: None,
            phase: None,
            commit_hash: None,
            tag: None,
        },
        TailEventKind::StateTransition {
            description,
            loop_number,
            phase,
        } => TailEventOutput {
            project_id,
            event_type: event.kind.event_type(),
            timestamp: &event.timestamp,
            path: None,
            filename_timestamp: None,
            created_at: None,
            artifact: None,
            loop_number: *loop_number,
            iteration: None,
            role: None,
            backend: None,
            heading: None,
            body: None,
            description: Some(description),
            phase: phase.as_deref(),
            commit_hash: None,
            tag: None,
        },
        TailEventKind::GitCommit {
            loop_number,
            commit_hash,
            tag,
        } => TailEventOutput {
            project_id,
            event_type: event.kind.event_type(),
            timestamp: &event.timestamp,
            path: None,
            filename_timestamp: None,
            created_at: None,
            artifact: None,
            loop_number: Some(*loop_number),
            iteration: None,
            role: None,
            backend: None,
            heading: None,
            body: None,
            description: None,
            phase: None,
            commit_hash: Some(commit_hash),
            tag: tag.as_deref(),
        },
    }
}

fn push_meta_str(parts: &mut Vec<String>, key: &str, value: Option<&str>) {
    if let Some(value) = value {
        parts.push(format!("{key}={value}"));
    }
}

fn push_meta_u32(parts: &mut Vec<String>, key: &str, value: Option<u32>) {
    if let Some(value) = value {
        parts.push(format!("{key}={value}"));
    }
}

fn collect_artifact_events(project_dir: &Path) -> Result<Vec<TailEvent>> {
    let loops_dir = project_dir.join("loops");
    let loops = match fs::read_dir(&loops_dir) {
        Ok(entries) => entries,
        Err(err) if err.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
        Err(err) => return Err(err.into()),
    };

    let mut events = Vec::new();

    for loop_entry in loops {
        let loop_entry = loop_entry?;
        if !loop_entry.file_type()?.is_dir() {
            continue;
        }

        let artifacts = match fs::read_dir(loop_entry.path()) {
            Ok(entries) => entries,
            Err(err) if err.kind() == ErrorKind::NotFound => continue,
            Err(err) => return Err(err.into()),
        };

        for artifact_entry in artifacts {
            let artifact_entry = artifact_entry?;
            if !artifact_entry.file_type()?.is_file() {
                continue;
            }

            let artifact_path = artifact_entry.path();
            if artifact_path.extension().and_then(|ext| ext.to_str()) != Some("md") {
                continue;
            }

            let metadata = match artifact_entry.metadata() {
                Ok(meta) => meta,
                Err(err) if err.kind() == ErrorKind::NotFound => continue,
                Err(err) => return Err(err.into()),
            };

            let content = match fs::read_to_string(&artifact_path) {
                Ok(raw) => raw,
                Err(err) if err.kind() == ErrorKind::NotFound => continue,
                Err(err) => return Err(err.into()),
            };

            let file_name = artifact_path
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or_default()
                .to_owned();
            let filename_timestamp = parse_artifact_filename_timestamp(&file_name);
            let rel_path = artifact_path
                .strip_prefix(project_dir)
                .unwrap_or(&artifact_path)
                .to_string_lossy()
                .replace('\\', "/");
            let modified = metadata.modified().ok();
            let modified_ns = modified
                .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
                .map(|d| d.as_nanos())
                .unwrap_or(0);

            let (frontmatter, body) = split_frontmatter(&content);
            let created_at = frontmatter.get("created_at").cloned();
            let artifact = frontmatter.get("artifact").cloned();
            let role = frontmatter.get("role").cloned();
            let backend = frontmatter.get("backend").cloned();
            let loop_number = frontmatter.get("loop").and_then(|v| v.parse::<u32>().ok());
            let iteration = frontmatter
                .get("iteration")
                .and_then(|v| v.parse::<u32>().ok());
            let heading = first_h1(&body);
            let (timestamp, sort_epoch_ms) = normalize_event_time(
                created_at.as_deref(),
                filename_timestamp.as_deref(),
                modified,
            );

            events.push(TailEvent {
                timestamp,
                sort_epoch_ms,
                sort_tiebreaker: EVENT_ORDER_ARTIFACT,
                signature: format!("artifact::{rel_path}::{}:{modified_ns}", metadata.len()),
                kind: TailEventKind::Artifact {
                    rel_path,
                    filename_timestamp,
                    created_at,
                    artifact,
                    loop_number,
                    iteration,
                    role,
                    backend,
                    heading,
                    body,
                },
            });
        }
    }

    Ok(events)
}

fn collect_state_events(workspace: &Workspace, project_id: &str) -> Vec<TailEvent> {
    let state = match reconstruct_project_state(workspace, project_id) {
        Ok(state) => state,
        Err(err) => {
            eprintln!("warning: failed to derive state for tail: {err}");
            return Vec::new();
        }
    };

    let mut events = Vec::new();

    for feature_loop in &state.loops {
        let (started_ts, started_epoch_ms) = to_timestamp(feature_loop.started_at);
        events.push(TailEvent {
            timestamp: started_ts,
            sort_epoch_ms: started_epoch_ms,
            sort_tiebreaker: EVENT_ORDER_STATE,
            signature: format!(
                "state::feature_started::{}::{}",
                feature_loop.loop_number,
                feature_loop
                    .started_at
                    .to_rfc3339_opts(SecondsFormat::Secs, true)
            ),
            kind: TailEventKind::StateTransition {
                description: format!(
                    "loop {} ({}) started",
                    feature_loop.loop_number, feature_loop.feature_name
                ),
                loop_number: Some(feature_loop.loop_number),
                phase: None,
            },
        });

        if feature_loop.status == LoopStatus::Completed {
            if let Some(completed_at) = feature_loop.completed_at {
                let (completed_ts, completed_epoch_ms) = to_timestamp(completed_at);
                events.push(TailEvent {
                    timestamp: completed_ts,
                    sort_epoch_ms: completed_epoch_ms,
                    sort_tiebreaker: EVENT_ORDER_STATE,
                    signature: format!(
                        "state::feature_completed::{}::{}",
                        feature_loop.loop_number,
                        completed_at.to_rfc3339_opts(SecondsFormat::Secs, true)
                    ),
                    kind: TailEventKind::StateTransition {
                        description: format!(
                            "loop {} ({}) completed",
                            feature_loop.loop_number, feature_loop.feature_name
                        ),
                        loop_number: Some(feature_loop.loop_number),
                        phase: None,
                    },
                });
            }
        }

        if let Some(commit_hash) = feature_loop.commit.as_deref() {
            let commit_time = feature_loop.completed_at.unwrap_or(feature_loop.started_at);
            let (commit_ts, commit_epoch_ms) = to_timestamp(commit_time);
            events.push(TailEvent {
                timestamp: commit_ts,
                sort_epoch_ms: commit_epoch_ms,
                sort_tiebreaker: EVENT_ORDER_GIT,
                signature: format!("git::commit::{}::{commit_hash}", feature_loop.loop_number),
                kind: TailEventKind::GitCommit {
                    loop_number: feature_loop.loop_number,
                    commit_hash: commit_hash.to_owned(),
                    tag: Some(format!("{project_id}/{}", feature_loop.loop_number)),
                },
            });
        }
    }

    for completion in &state.completion_attempts {
        let (started_ts, started_epoch_ms) = to_timestamp(completion.started_at);
        events.push(TailEvent {
            timestamp: started_ts,
            sort_epoch_ms: started_epoch_ms,
            sort_tiebreaker: EVENT_ORDER_STATE,
            signature: format!(
                "state::completion_started::{}::{}",
                completion.loop_number,
                completion
                    .started_at
                    .to_rfc3339_opts(SecondsFormat::Secs, true)
            ),
            kind: TailEventKind::StateTransition {
                description: format!("loop {} completion check started", completion.loop_number),
                loop_number: Some(completion.loop_number),
                phase: None,
            },
        });

        if completion.status == LoopStatus::Completed {
            if let (Some(completed_at), Some(verdict)) =
                (completion.completed_at, &completion.verdict)
            {
                let (completed_ts, completed_epoch_ms) = to_timestamp(completed_at);
                events.push(TailEvent {
                    timestamp: completed_ts,
                    sort_epoch_ms: completed_epoch_ms,
                    sort_tiebreaker: EVENT_ORDER_STATE,
                    signature: format!(
                        "state::completion_verdict::{}::{}::{}",
                        completion.loop_number,
                        completed_at.to_rfc3339_opts(SecondsFormat::Secs, true),
                        completion_verdict_label(verdict)
                    ),
                    kind: TailEventKind::StateTransition {
                        description: format!(
                            "loop {} completion verdict: {}",
                            completion.loop_number,
                            completion_verdict_label(verdict)
                        ),
                        loop_number: Some(completion.loop_number),
                        phase: None,
                    },
                });
            }
        }
    }

    events
}

fn collect_phase_snapshot(
    git_context: Option<&crate::project::lifecycle::ProjectGitContext>,
) -> Option<PhaseSnapshot> {
    let ctx = git_context?;
    let (current_loop, current_phase) = derive_position(&ctx.repo_root, &ctx.branch).ok()?;
    let checkpoint_hash = parse_last_ralph_commit(&ctx.repo_root, &ctx.branch)
        .ok()
        .flatten()
        .and_then(|commit| commit.commit_hash);
    let now = Utc::now();
    let (timestamp, sort_epoch_ms) = to_timestamp(now);

    Some(PhaseSnapshot {
        current_phase,
        current_loop,
        timestamp,
        sort_epoch_ms,
        checkpoint_hash,
    })
}

fn create_phase_change_event(
    previous: &PhaseSnapshot,
    current: &PhaseSnapshot,
) -> Option<TailEvent> {
    if previous.current_phase == current.current_phase
        && previous.current_loop == current.current_loop
    {
        return None;
    }

    let description = if previous.current_loop == current.current_loop {
        format!(
            "phase changed: {} -> {} (loop {})",
            phase_label(&previous.current_phase),
            phase_label(&current.current_phase),
            current.current_loop
        )
    } else {
        format!(
            "phase changed: {} -> {} (loop {} -> {})",
            phase_label(&previous.current_phase),
            phase_label(&current.current_phase),
            previous.current_loop,
            current.current_loop
        )
    };

    Some(TailEvent {
        timestamp: current.timestamp.clone(),
        sort_epoch_ms: current.sort_epoch_ms,
        sort_tiebreaker: EVENT_ORDER_STATE,
        signature: format!(
            "state::phase::{}->{}::{}::{}",
            phase_label(&previous.current_phase),
            phase_label(&current.current_phase),
            current.current_loop,
            current
                .checkpoint_hash
                .as_deref()
                .unwrap_or("no-checkpoint-hash")
        ),
        kind: TailEventKind::StateTransition {
            description,
            loop_number: Some(current.current_loop),
            phase: Some(phase_label(&current.current_phase).to_owned()),
        },
    })
}

fn phase_label(phase: &Phase) -> &'static str {
    match phase {
        Phase::Planning => "planning",
        Phase::Implementing => "implementing",
        Phase::QA => "qa",
        Phase::Reviewing => "reviewing",
        Phase::Committing => "committing",
        Phase::Completing => "completing",
        Phase::FinalReview => "final_review",
    }
}

fn completion_verdict_label(verdict: &CompletionVerdict) -> &'static str {
    match verdict {
        CompletionVerdict::Continue => "CONTINUE",
        CompletionVerdict::Complete => "COMPLETE",
    }
}

fn sort_events(events: &mut [TailEvent]) {
    events.sort_by(|left, right| {
        left.sort_epoch_ms
            .cmp(&right.sort_epoch_ms)
            .then_with(|| left.sort_tiebreaker.cmp(&right.sort_tiebreaker))
            .then_with(|| left.signature.cmp(&right.signature))
    });
}

fn normalize_event_time(
    created_at: Option<&str>,
    filename_timestamp: Option<&str>,
    file_mtime: Option<SystemTime>,
) -> (String, i64) {
    if let Some(created_at) = created_at {
        if let Some(parsed) = parse_rfc3339_utc(created_at) {
            return to_timestamp(parsed);
        }
    }

    if let Some(filename_timestamp) = filename_timestamp {
        if let Some(parsed) = filename_ts_to_datetime(filename_timestamp) {
            return to_timestamp(parsed);
        }
    }

    if let Some(file_mtime) = file_mtime {
        let parsed = DateTime::<Utc>::from(file_mtime);
        return to_timestamp(parsed);
    }

    ("unknown-time".to_owned(), 0)
}

fn parse_rfc3339_utc(value: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|dt| dt.with_timezone(&Utc))
}

fn filename_ts_to_datetime(value: &str) -> Option<DateTime<Utc>> {
    let parsed = NaiveDateTime::parse_from_str(value, "%Y%m%d%H%M%S").ok()?;
    Some(DateTime::from_naive_utc_and_offset(parsed, Utc))
}

fn to_timestamp(dt: DateTime<Utc>) -> (String, i64) {
    (
        dt.to_rfc3339_opts(SecondsFormat::Secs, true),
        dt.timestamp_millis(),
    )
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

fn first_h1(body: &str) -> Option<String> {
    body.lines()
        .find(|line| line.trim_start().starts_with("# "))
        .map(|line| line.trim().to_owned())
}

#[cfg(test)]
mod tests {
    use std::fs;

    use chrono::{DateTime, Utc};
    use tempfile::tempdir;

    use super::{
        collect_state_events, completion_verdict_label, create_phase_change_event, event_output,
        filename_ts_to_datetime, first_h1, normalize_event_time, phase_label, sort_events,
        split_frontmatter, PhaseSnapshot, TailEvent, TailEventKind, EVENT_ORDER_ARTIFACT,
        EVENT_ORDER_GIT, EVENT_ORDER_STATE,
    };
    use crate::project::state::{
        CompletionLoopArtifacts, CompletionLoopBackends, CompletionLoopState, CompletionVerdict,
        FeatureLoopArtifacts, FeatureLoopBackends, FeatureLoopState, LoopStatus, LoopType, Phase,
        ProjectState, ProjectStatus,
    };

    fn parse_utc(value: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(value)
            .unwrap()
            .with_timezone(&Utc)
    }

    fn artifact_event(signature: &str, epoch_ms: i64) -> TailEvent {
        TailEvent {
            timestamp: "2026-02-06T21:00:00Z".to_owned(),
            sort_epoch_ms: epoch_ms,
            sort_tiebreaker: EVENT_ORDER_ARTIFACT,
            signature: signature.to_owned(),
            kind: TailEventKind::Artifact {
                rel_path: "loops/001-demo/20260206210000-spec.md".to_owned(),
                filename_timestamp: Some("20260206210000".to_owned()),
                created_at: Some("2026-02-06T21:00:00Z".to_owned()),
                artifact: Some("spec".to_owned()),
                loop_number: Some(1),
                iteration: None,
                role: Some("planner".to_owned()),
                backend: Some("codex".to_owned()),
                heading: Some("# Demo".to_owned()),
                body: "# Demo".to_owned(),
            },
        }
    }

    fn state_event(signature: &str, epoch_ms: i64) -> TailEvent {
        TailEvent {
            timestamp: "2026-02-06T21:00:00Z".to_owned(),
            sort_epoch_ms: epoch_ms,
            sort_tiebreaker: EVENT_ORDER_STATE,
            signature: signature.to_owned(),
            kind: TailEventKind::StateTransition {
                description: "loop 1 completed".to_owned(),
                loop_number: Some(1),
                phase: None,
            },
        }
    }

    fn git_event(signature: &str, epoch_ms: i64) -> TailEvent {
        TailEvent {
            timestamp: "2026-02-06T21:00:00Z".to_owned(),
            sort_epoch_ms: epoch_ms,
            sort_tiebreaker: EVENT_ORDER_GIT,
            signature: signature.to_owned(),
            kind: TailEventKind::GitCommit {
                loop_number: 1,
                commit_hash: "abc123".to_owned(),
                tag: Some("demo/1".to_owned()),
            },
        }
    }

    fn demo_project_state() -> ProjectState {
        let mut state = ProjectState::new("demo", "Demo Project", "hash", None);
        state.current_loop = 2;
        state.current_phase = Phase::Completing;
        state.phase_iteration = 1;
        state.status = ProjectStatus::InProgress;

        state.loops.push(FeatureLoopState {
            loop_number: 1,
            slug: "feature-a".to_owned(),
            feature_name: "Feature A".to_owned(),
            loop_type: LoopType::Feature,
            status: LoopStatus::Completed,
            backends: FeatureLoopBackends {
                planner: "planner-a".to_owned(),
                implementer: "impl-a".to_owned(),
                reviewer: "reviewer-a".to_owned(),
                qa: "qa-a".to_owned(),
            },
            artifacts: FeatureLoopArtifacts {
                spec: "loops/001-feature-a/spec.md".to_owned(),
                impl_notes: Some("loops/001-feature-a/impl-notes.md".to_owned()),
                reviews: Vec::new(),
                approval: Some("loops/001-feature-a/review-approved.md".to_owned()),
                qa_results: Vec::new(),
                pending_qa_feedback: None,
            },
            commit: Some("abc123".to_owned()),
            started_at: parse_utc("2026-02-06T21:00:00Z"),
            completed_at: Some(parse_utc("2026-02-06T21:05:00Z")),
        });

        state.completion_attempts.push(CompletionLoopState {
            loop_number: 2,
            slug: "completion".to_owned(),
            loop_type: LoopType::Completion,
            status: LoopStatus::Completed,
            backends: CompletionLoopBackends {
                planner: "planner-a".to_owned(),
                completer: "completer-a".to_owned(),
            },
            artifacts: CompletionLoopArtifacts {
                termination_request: "loops/002-completion/termination-request.md".to_owned(),
                verdict: Some("loops/002-completion/completer-verdict.md".to_owned()),
                acceptance_results: Vec::new(),
                acceptance_result: None,
                acceptance_passed: None,
            },
            verdict: Some(CompletionVerdict::Continue),
            started_at: parse_utc("2026-02-06T21:06:00Z"),
            completed_at: Some(parse_utc("2026-02-06T21:07:00Z")),
        });

        state
    }

    #[test]
    fn splits_frontmatter_and_extracts_heading() {
        let raw = r#"---
artifact: spec
created_at: 2026-02-06T20:00:00Z
---

# Feature: Demo
## Detail
"#;
        let (fm, body) = split_frontmatter(raw);
        assert_eq!(fm.get("artifact").map(String::as_str), Some("spec"));
        assert_eq!(
            fm.get("created_at").map(String::as_str),
            Some("2026-02-06T20:00:00Z")
        );
        assert_eq!(first_h1(&body).as_deref(), Some("# Feature: Demo"));
        assert!(body.contains("## Detail"));
    }

    #[test]
    fn parses_filename_timestamp_as_utc() {
        let parsed = filename_ts_to_datetime("20260206212953").unwrap();
        assert_eq!(
            parsed.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
            "2026-02-06T21:29:53Z"
        );
    }

    #[test]
    fn normalizes_timestamps_preferring_created_at() {
        let (timestamp, epoch_ms) =
            normalize_event_time(Some("2026-02-06T21:10:11Z"), Some("20260206220000"), None);
        assert_eq!(timestamp, "2026-02-06T21:10:11Z");
        assert_eq!(
            epoch_ms,
            parse_utc("2026-02-06T21:10:11Z").timestamp_millis()
        );
    }

    #[test]
    fn sorts_by_epoch_then_type_then_signature() {
        let mut events = vec![
            git_event("git::same-time", 1000),
            state_event("state::same-time", 1000),
            artifact_event("artifact::same-time", 1000),
            artifact_event("artifact::older", 999),
        ];

        sort_events(&mut events);

        let signatures: Vec<&str> = events
            .iter()
            .map(|event| event.signature.as_str())
            .collect();
        assert_eq!(
            signatures,
            vec![
                "artifact::older",
                "artifact::same-time",
                "state::same-time",
                "git::same-time"
            ]
        );
    }

    #[test]
    fn emits_phase_change_event_with_new_phase_details() {
        let previous = PhaseSnapshot {
            current_phase: Phase::Planning,
            current_loop: 1,
            timestamp: "2026-02-06T21:00:00Z".to_owned(),
            sort_epoch_ms: 1,
            checkpoint_hash: Some("abc".to_owned()),
        };
        let current = PhaseSnapshot {
            current_phase: Phase::Implementing,
            current_loop: 1,
            timestamp: "2026-02-06T21:01:00Z".to_owned(),
            sort_epoch_ms: 2,
            checkpoint_hash: Some("def".to_owned()),
        };

        let event = create_phase_change_event(&previous, &current).unwrap();
        match event.kind {
            TailEventKind::StateTransition {
                description,
                loop_number,
                phase,
            } => {
                assert_eq!(
                    description,
                    "phase changed: planning -> implementing (loop 1)"
                );
                assert_eq!(loop_number, Some(1));
                assert_eq!(phase.as_deref(), Some("implementing"));
            }
            TailEventKind::Artifact { .. } | TailEventKind::GitCommit { .. } => {
                panic!("expected state transition")
            }
        }
    }

    #[test]
    fn serializes_json_with_event_specific_fields() {
        let artifact = artifact_event("artifact::demo", 100);
        let artifact_json = serde_json::to_value(event_output("demo", &artifact)).unwrap();
        assert_eq!(artifact_json.get("event_type").unwrap(), "artifact");
        assert_eq!(artifact_json.get("body").unwrap(), "# Demo");
        assert!(artifact_json.get("description").is_none());

        let state = state_event("state::demo", 100);
        let state_json = serde_json::to_value(event_output("demo", &state)).unwrap();
        assert_eq!(state_json.get("event_type").unwrap(), "state");
        assert_eq!(state_json.get("description").unwrap(), "loop 1 completed");
        assert!(state_json.get("commit_hash").is_none());

        let git = git_event("git::demo", 100);
        let git_json = serde_json::to_value(event_output("demo", &git)).unwrap();
        assert_eq!(git_json.get("event_type").unwrap(), "git");
        assert_eq!(git_json.get("commit_hash").unwrap(), "abc123");
        assert_eq!(git_json.get("tag").unwrap(), "demo/1");
    }

    #[test]
    fn labels_use_expected_wire_format() {
        assert_eq!(phase_label(&Phase::Reviewing), "reviewing");
        assert_eq!(phase_label(&Phase::FinalReview), "final_review");
        assert_eq!(
            completion_verdict_label(&CompletionVerdict::Complete),
            "COMPLETE"
        );
    }
}
