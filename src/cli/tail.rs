use std::cmp::Ordering;
use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::io::ErrorKind;
use std::path::Path;
use std::time::UNIX_EPOCH;

use serde::Serialize;
use tokio::time::{sleep, Duration};

use crate::cli::TailArgs;
use crate::project::artifacts::parse_artifact_filename_timestamp;
use crate::workspace::Workspace;
use crate::{error::RalphError, Result};

#[derive(Debug, Clone)]
struct TailEvent {
    rel_path: String,
    filename_timestamp: Option<String>,
    created_at: Option<String>,
    artifact: Option<String>,
    loop_number: Option<u32>,
    iteration: Option<u32>,
    role: Option<String>,
    backend: Option<String>,
    heading: Option<String>,
    signature: String,
}

impl TailEvent {
    fn event_key(&self) -> String {
        format!("{}::{}", self.rel_path, self.signature)
    }
}

#[derive(Debug, Serialize)]
struct TailEventOutput<'a> {
    project_id: &'a str,
    path: &'a str,
    filename_timestamp: Option<&'a str>,
    created_at: Option<&'a str>,
    artifact: Option<&'a str>,
    loop_number: Option<u32>,
    iteration: Option<u32>,
    role: Option<&'a str>,
    backend: Option<&'a str>,
    heading: Option<&'a str>,
}

pub async fn execute(args: TailArgs) -> Result<()> {
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

    let project_dir = workspace.project_dir(&project_id);
    if !project_dir.exists() {
        return Err(RalphError::ProjectNotFound(project_id));
    }

    let mut events = collect_artifact_events(&project_dir)?;
    sort_events(&mut events);

    let mut seen: HashSet<String> = events.iter().map(TailEvent::event_key).collect();
    print_events(&project_id, &events, args.last, args.json)?;

    if !args.follow {
        return Ok(());
    }

    loop {
        sleep(Duration::from_millis(args.poll_interval_ms)).await;

        let mut rescanned = collect_artifact_events(&project_dir)?;
        sort_events(&mut rescanned);

        let mut new_events = Vec::new();
        for event in rescanned {
            if seen.insert(event.event_key()) {
                new_events.push(event);
            }
        }

        if !new_events.is_empty() {
            print_events(&project_id, &new_events, None, args.json)?;
        }
    }
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
            let out = TailEventOutput {
                project_id,
                path: &event.rel_path,
                filename_timestamp: event.filename_timestamp.as_deref(),
                created_at: event.created_at.as_deref(),
                artifact: event.artifact.as_deref(),
                loop_number: event.loop_number,
                iteration: event.iteration,
                role: event.role.as_deref(),
                backend: event.backend.as_deref(),
                heading: event.heading.as_deref(),
            };
            println!("{}", serde_json::to_string(&out)?);
        } else {
            let when = event
                .created_at
                .as_deref()
                .or(event.filename_timestamp.as_deref())
                .unwrap_or("unknown-time");
            let artifact = event.artifact.as_deref().unwrap_or("unknown");
            let role = event.role.as_deref().unwrap_or("unknown");
            if let Some(heading) = event.heading.as_deref() {
                println!(
                    "[{when}] {} | artifact={artifact} role={role} | {heading}",
                    event.rel_path
                );
            } else {
                println!(
                    "[{when}] {} | artifact={artifact} role={role}",
                    event.rel_path
                );
            }
        }
    }

    Ok(())
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
            let modified_ns = metadata
                .modified()
                .ok()
                .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
                .map(|d| d.as_nanos())
                .unwrap_or(0);
            let signature = format!("{}:{modified_ns}", metadata.len());

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

            events.push(TailEvent {
                rel_path,
                filename_timestamp,
                created_at,
                artifact,
                loop_number,
                iteration,
                role,
                backend,
                heading,
                signature,
            });
        }
    }

    Ok(events)
}

fn sort_events(events: &mut [TailEvent]) {
    events.sort_by(|left, right| {
        compare_timestamps(
            left.filename_timestamp.as_deref(),
            right.filename_timestamp.as_deref(),
        )
        .then_with(|| compare_optional(left.created_at.as_deref(), right.created_at.as_deref()))
        .then_with(|| left.rel_path.cmp(&right.rel_path))
    });
}

fn compare_timestamps(left: Option<&str>, right: Option<&str>) -> Ordering {
    match (left, right) {
        (Some(l), Some(r)) => l.cmp(r),
        _ => Ordering::Equal,
    }
}

fn compare_optional(left: Option<&str>, right: Option<&str>) -> Ordering {
    match (left, right) {
        (Some(l), Some(r)) => l.cmp(r),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    }
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
    use super::{first_h1, sort_events, split_frontmatter, TailEvent};

    fn event(path: &str, filename_ts: Option<&str>, created_at: Option<&str>) -> TailEvent {
        TailEvent {
            rel_path: path.to_owned(),
            filename_timestamp: filename_ts.map(str::to_owned),
            created_at: created_at.map(str::to_owned),
            artifact: None,
            loop_number: None,
            iteration: None,
            role: None,
            backend: None,
            heading: None,
            signature: "s".to_owned(),
        }
    }

    #[test]
    fn splits_frontmatter_and_extracts_heading() {
        let raw = r#"---
artifact: spec
created_at: 2026-02-06T20:00:00Z
---

# Feature: Demo
"#;
        let (fm, body) = split_frontmatter(raw);
        assert_eq!(fm.get("artifact").map(String::as_str), Some("spec"));
        assert_eq!(
            fm.get("created_at").map(String::as_str),
            Some("2026-02-06T20:00:00Z")
        );
        assert_eq!(first_h1(&body).as_deref(), Some("# Feature: Demo"));
    }

    #[test]
    fn sorts_by_filename_timestamp_then_created_at_then_path() {
        let mut events = vec![
            event(
                "loops/001-demo/20260203060110-spec.md",
                Some("20260203060110"),
                Some("2026-02-03T06:01:10Z"),
            ),
            event(
                "loops/001-demo/20260203055920-spec.md",
                Some("20260203055920"),
                Some("2026-02-03T05:59:20Z"),
            ),
            event(
                "loops/001-demo/legacy.md",
                None,
                Some("2026-02-03T05:58:00Z"),
            ),
        ];

        sort_events(&mut events);

        assert_eq!(events[0].rel_path, "loops/001-demo/legacy.md");
        assert_eq!(events[1].rel_path, "loops/001-demo/20260203055920-spec.md");
        assert_eq!(events[2].rel_path, "loops/001-demo/20260203060110-spec.md");
    }
}
