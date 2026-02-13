use std::collections::BTreeMap;
use std::fs;
use std::io::ErrorKind;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use chrono::{DateTime, NaiveDateTime, SecondsFormat, Utc};
use serde_json::{json, Value};

use crate::project::artifacts::parse_artifact_filename_timestamp;
use crate::project::lifecycle::load_project_state;
use crate::project::state::{CompletionVerdict, LoopStatus};

const EVENT_ORDER_ARTIFACT: u8 = 0;
const EVENT_ORDER_STATE: u8 = 1;
const EVENT_ORDER_GIT: u8 = 2;

struct RawEvent {
    sort_epoch_ms: i64,
    sort_tiebreaker: u8,
    signature: String,
    value: Value,
}

/// Collect tail events for a project directory, returning them as sorted JSON
/// values matching the CLI tail JSON event shape (artifact / state / git).
///
/// If `last` is `Some(n)`, only the final `n` events are returned.
pub fn collect_tail_events(
    project_dir: &Path,
    project_id: &str,
    last: Option<usize>,
) -> crate::Result<Vec<Value>> {
    let mut events = collect_artifact_events(project_dir, project_id)?;
    events.extend(collect_state_events(project_dir, project_id));

    events.sort_by(|a, b| {
        a.sort_epoch_ms
            .cmp(&b.sort_epoch_ms)
            .then_with(|| a.sort_tiebreaker.cmp(&b.sort_tiebreaker))
            .then_with(|| a.signature.cmp(&b.signature))
    });

    let start_idx = last
        .map(|count| events.len().saturating_sub(count))
        .unwrap_or(0);

    Ok(events[start_idx..].iter().map(|e| e.value.clone()).collect())
}

fn collect_artifact_events(project_dir: &Path, project_id: &str) -> crate::Result<Vec<RawEvent>> {
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

            let mut value = json!({
                "project_id": project_id,
                "event_type": "artifact",
                "timestamp": timestamp,
                "path": rel_path,
                "body": body,
            });
            let obj = value.as_object_mut().unwrap();
            if let Some(v) = &filename_timestamp {
                obj.insert("filename_timestamp".to_owned(), json!(v));
            }
            if let Some(v) = &created_at {
                obj.insert("created_at".to_owned(), json!(v));
            }
            if let Some(v) = &artifact {
                obj.insert("artifact".to_owned(), json!(v));
            }
            if let Some(v) = loop_number {
                obj.insert("loop_number".to_owned(), json!(v));
            }
            if let Some(v) = iteration {
                obj.insert("iteration".to_owned(), json!(v));
            }
            if let Some(v) = &role {
                obj.insert("role".to_owned(), json!(v));
            }
            if let Some(v) = &backend {
                obj.insert("backend".to_owned(), json!(v));
            }
            if let Some(v) = &heading {
                obj.insert("heading".to_owned(), json!(v));
            }

            events.push(RawEvent {
                sort_epoch_ms,
                sort_tiebreaker: EVENT_ORDER_ARTIFACT,
                signature: format!("artifact::{rel_path}::{}:{modified_ns}", metadata.len()),
                value,
            });
        }
    }

    Ok(events)
}

fn collect_state_events(project_dir: &Path, project_id: &str) -> Vec<RawEvent> {
    let state_path = project_dir.join("state.json");
    if !state_path.exists() {
        return Vec::new();
    }

    let state = match load_project_state(project_dir) {
        Ok(state) => state,
        Err(_) => return Vec::new(),
    };

    let mut events = Vec::new();

    for feature_loop in &state.loops {
        let (started_ts, started_epoch_ms) = to_timestamp(feature_loop.started_at);
        events.push(RawEvent {
            sort_epoch_ms: started_epoch_ms,
            sort_tiebreaker: EVENT_ORDER_STATE,
            signature: format!(
                "state::feature_started::{}::{}",
                feature_loop.loop_number,
                feature_loop
                    .started_at
                    .to_rfc3339_opts(SecondsFormat::Secs, true)
            ),
            value: json!({
                "project_id": project_id,
                "event_type": "state",
                "timestamp": started_ts,
                "description": format!(
                    "loop {} ({}) started",
                    feature_loop.loop_number, feature_loop.feature_name
                ),
                "loop_number": feature_loop.loop_number,
            }),
        });

        if feature_loop.status == LoopStatus::Completed {
            if let Some(completed_at) = feature_loop.completed_at {
                let (completed_ts, completed_epoch_ms) = to_timestamp(completed_at);
                events.push(RawEvent {
                    sort_epoch_ms: completed_epoch_ms,
                    sort_tiebreaker: EVENT_ORDER_STATE,
                    signature: format!(
                        "state::feature_completed::{}::{}",
                        feature_loop.loop_number,
                        completed_at.to_rfc3339_opts(SecondsFormat::Secs, true)
                    ),
                    value: json!({
                        "project_id": project_id,
                        "event_type": "state",
                        "timestamp": completed_ts,
                        "description": format!(
                            "loop {} ({}) completed",
                            feature_loop.loop_number, feature_loop.feature_name
                        ),
                        "loop_number": feature_loop.loop_number,
                    }),
                });
            }
        }

        if let Some(commit_hash) = feature_loop.commit.as_deref() {
            let commit_time = feature_loop.completed_at.unwrap_or(feature_loop.started_at);
            let (commit_ts, commit_epoch_ms) = to_timestamp(commit_time);
            events.push(RawEvent {
                sort_epoch_ms: commit_epoch_ms,
                sort_tiebreaker: EVENT_ORDER_GIT,
                signature: format!("git::commit::{}::{commit_hash}", feature_loop.loop_number),
                value: json!({
                    "project_id": project_id,
                    "event_type": "git",
                    "timestamp": commit_ts,
                    "loop_number": feature_loop.loop_number,
                    "commit_hash": commit_hash,
                    "tag": format!("{project_id}/{}", feature_loop.loop_number),
                }),
            });
        }
    }

    for completion in &state.completion_attempts {
        let (started_ts, started_epoch_ms) = to_timestamp(completion.started_at);
        events.push(RawEvent {
            sort_epoch_ms: started_epoch_ms,
            sort_tiebreaker: EVENT_ORDER_STATE,
            signature: format!(
                "state::completion_started::{}::{}",
                completion.loop_number,
                completion
                    .started_at
                    .to_rfc3339_opts(SecondsFormat::Secs, true)
            ),
            value: json!({
                "project_id": project_id,
                "event_type": "state",
                "timestamp": started_ts,
                "description": format!("loop {} completion check started", completion.loop_number),
                "loop_number": completion.loop_number,
            }),
        });

        if completion.status == LoopStatus::Completed {
            if let (Some(completed_at), Some(verdict)) =
                (completion.completed_at, &completion.verdict)
            {
                let (completed_ts, completed_epoch_ms) = to_timestamp(completed_at);
                let verdict_label = match verdict {
                    CompletionVerdict::Continue => "CONTINUE",
                    CompletionVerdict::Complete => "COMPLETE",
                };
                events.push(RawEvent {
                    sort_epoch_ms: completed_epoch_ms,
                    sort_tiebreaker: EVENT_ORDER_STATE,
                    signature: format!(
                        "state::completion_verdict::{}::{}::{verdict_label}",
                        completion.loop_number,
                        completed_at.to_rfc3339_opts(SecondsFormat::Secs, true),
                    ),
                    value: json!({
                        "project_id": project_id,
                        "event_type": "state",
                        "timestamp": completed_ts,
                        "description": format!(
                            "loop {} completion verdict: {verdict_label}",
                            completion.loop_number,
                        ),
                        "loop_number": completion.loop_number,
                    }),
                });
            }
        }
    }

    events
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
    use chrono::DateTime;
    use serde_json::Value;
    use tempfile::tempdir;

    use super::collect_tail_events;
    use crate::project::state::{
        CompletionLoopArtifacts, CompletionLoopBackends, CompletionLoopState, CompletionVerdict,
        FeatureLoopArtifacts, FeatureLoopBackends, FeatureLoopState, LoopStatus, LoopType, Phase,
        ProjectState, ProjectStatus,
    };

    fn parse_utc(value: &str) -> DateTime<chrono::Utc> {
        DateTime::parse_from_rfc3339(value)
            .unwrap()
            .with_timezone(&chrono::Utc)
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
                impl_notes: None,
                reviews: Vec::new(),
                approval: None,
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
    fn collects_state_and_git_events_as_json() {
        let temp = tempdir().unwrap();
        let state = demo_project_state();
        state.save(&temp.path().join("state.json")).unwrap();

        let events = collect_tail_events(temp.path(), "demo", None).unwrap();
        assert!(!events.is_empty());

        let state_events: Vec<&Value> = events
            .iter()
            .filter(|e| e["event_type"] == "state")
            .collect();
        assert!(state_events.len() >= 3);

        let git_events: Vec<&Value> = events
            .iter()
            .filter(|e| e["event_type"] == "git")
            .collect();
        assert_eq!(git_events.len(), 1);
        assert_eq!(git_events[0]["commit_hash"], "abc123");
        assert_eq!(git_events[0]["loop_number"], 1);
    }

    #[test]
    fn last_truncates_events() {
        let temp = tempdir().unwrap();
        let state = demo_project_state();
        state.save(&temp.path().join("state.json")).unwrap();

        let all = collect_tail_events(temp.path(), "demo", None).unwrap();
        let last2 = collect_tail_events(temp.path(), "demo", Some(2)).unwrap();

        assert_eq!(last2.len(), 2);
        assert_eq!(last2[0], all[all.len() - 2]);
        assert_eq!(last2[1], all[all.len() - 1]);
    }

    #[test]
    fn returns_empty_when_no_state() {
        let temp = tempdir().unwrap();
        let events = collect_tail_events(temp.path(), "demo", None).unwrap();
        assert!(events.is_empty());
    }

    #[test]
    fn deterministic_sort_ordering() {
        let temp = tempdir().unwrap();
        let state = demo_project_state();
        state.save(&temp.path().join("state.json")).unwrap();

        let events1 = collect_tail_events(temp.path(), "demo", None).unwrap();
        let events2 = collect_tail_events(temp.path(), "demo", None).unwrap();

        assert_eq!(events1, events2);
    }
}
