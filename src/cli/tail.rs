use std::cmp::Ordering;
use std::collections::HashSet;
use std::fs;
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;

use chrono::{DateTime, NaiveDateTime, SecondsFormat, Utc};
use serde::Serialize;

use crate::cli::TailArgs;
use crate::workspace::Workspace;
use crate::{error::RalphError, Result};

#[derive(Debug, Clone)]
struct ArtifactEvent {
    project: String,
    path: String,
    filename_timestamp: Option<NaiveDateTime>,
    created_at: Option<DateTime<Utc>>,
}

impl ArtifactEvent {
    fn filename_timestamp_string(&self) -> Option<String> {
        self.filename_timestamp
            .map(|value| value.format("%Y%m%d%H%M%S").to_string())
    }

    fn created_at_string(&self) -> Option<String> {
        self.created_at
            .map(|value| value.to_rfc3339_opts(SecondsFormat::Secs, true))
    }
}

#[derive(Debug, Serialize)]
struct JsonArtifactEvent<'a> {
    project: &'a str,
    path: &'a str,
    filename_timestamp: Option<String>,
    created_at: Option<String>,
}

pub fn execute(args: TailArgs) -> Result<()> {
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

    if workspace.index.get_project(&project_id).is_none() {
        return Err(RalphError::ProjectNotFound(project_id));
    }

    let project_dir = workspace.project_dir(&project_id);
    let initial_events = discover_artifact_events(&project_dir, &project_id)?;
    let mut seen_paths = initial_events
        .iter()
        .map(|event| event.path.clone())
        .collect::<HashSet<_>>();

    for event in select_initial_events(&initial_events, args.last) {
        emit_event(&event, args.json)?;
    }

    if !args.follow {
        return Ok(());
    }

    let poll_interval = Duration::from_millis(args.poll_interval_ms.max(1));
    loop {
        thread::sleep(poll_interval);
        let events = discover_artifact_events(&project_dir, &project_id)?;
        for event in events {
            if seen_paths.insert(event.path.clone()) {
                emit_event(&event, args.json)?;
            }
        }
    }
}

fn emit_event(event: &ArtifactEvent, as_json: bool) -> Result<()> {
    if as_json {
        let payload = JsonArtifactEvent {
            project: &event.project,
            path: &event.path,
            filename_timestamp: event.filename_timestamp_string(),
            created_at: event.created_at_string(),
        };
        println!("{}", serde_json::to_string(&payload)?);
    } else if let Some(created_at) = event.created_at_string() {
        let timestamp = event
            .filename_timestamp_string()
            .unwrap_or_else(|| "unknown-timestamp".to_owned());
        println!("{timestamp} {} created_at={created_at}", event.path);
    } else {
        let timestamp = event
            .filename_timestamp_string()
            .unwrap_or_else(|| "unknown-timestamp".to_owned());
        println!("{timestamp} {}", event.path);
    }

    std::io::stdout().flush()?;
    Ok(())
}

fn discover_artifact_events(project_dir: &Path, project_id: &str) -> Result<Vec<ArtifactEvent>> {
    let loops_dir = project_dir.join("loops");
    let markdown_files = collect_markdown_files(&loops_dir)?;

    let mut events = Vec::new();
    for file_path in markdown_files {
        let raw = match fs::read_to_string(&file_path) {
            Ok(raw) => raw,
            Err(err) if err.kind() == ErrorKind::NotFound => {
                continue;
            }
            Err(err) => return Err(err.into()),
        };

        events.push(ArtifactEvent {
            project: project_id.to_owned(),
            path: project_relative_path(project_dir, &file_path),
            filename_timestamp: parse_filename_timestamp(&file_path),
            created_at: parse_created_at_frontmatter(&raw),
        });
    }

    sort_artifact_events(&mut events);
    Ok(events)
}

fn collect_markdown_files(root: &Path) -> Result<Vec<PathBuf>> {
    if !root.is_dir() {
        return Ok(Vec::new());
    }

    let mut markdown_files = Vec::new();
    let mut stack = vec![root.to_path_buf()];

    while let Some(dir) = stack.pop() {
        let entries = match fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(err) if err.kind() == ErrorKind::NotFound => {
                continue;
            }
            Err(err) => return Err(err.into()),
        };

        for entry_result in entries {
            let entry = match entry_result {
                Ok(entry) => entry,
                Err(err) if err.kind() == ErrorKind::NotFound => {
                    continue;
                }
                Err(err) => return Err(err.into()),
            };

            let file_type = match entry.file_type() {
                Ok(file_type) => file_type,
                Err(err) if err.kind() == ErrorKind::NotFound => {
                    continue;
                }
                Err(err) => return Err(err.into()),
            };

            if file_type.is_dir() {
                stack.push(entry.path());
            } else if file_type.is_file()
                && entry.path().extension().and_then(|ext| ext.to_str()) == Some("md")
            {
                markdown_files.push(entry.path());
            }
        }
    }

    Ok(markdown_files)
}

fn parse_filename_timestamp(path: &Path) -> Option<NaiveDateTime> {
    let file_name = path.file_name()?.to_str()?;
    let (prefix, _) = file_name.split_once('-')?;
    if prefix.len() != 14 || !prefix.chars().all(|ch| ch.is_ascii_digit()) {
        return None;
    }

    NaiveDateTime::parse_from_str(prefix, "%Y%m%d%H%M%S").ok()
}

fn parse_created_at_frontmatter(raw: &str) -> Option<DateTime<Utc>> {
    let mut lines = raw.lines();
    if lines.next()?.trim() != "---" {
        return None;
    }

    for line in lines {
        let trimmed = line.trim();
        if trimmed == "---" {
            break;
        }

        if let Some(value) = trimmed.strip_prefix("created_at:") {
            let value = value.trim().trim_matches('"').trim_matches('\'');
            return DateTime::parse_from_rfc3339(value)
                .ok()
                .map(|value| value.with_timezone(&Utc));
        }
    }

    None
}

fn project_relative_path(project_dir: &Path, artifact_path: &Path) -> String {
    artifact_path
        .strip_prefix(project_dir)
        .unwrap_or(artifact_path)
        .to_string_lossy()
        .replace('\\', "/")
}

fn sort_artifact_events(events: &mut [ArtifactEvent]) {
    events.sort_by(compare_events);
}

fn compare_events(left: &ArtifactEvent, right: &ArtifactEvent) -> Ordering {
    compare_optional(&left.filename_timestamp, &right.filename_timestamp)
        .then_with(|| compare_optional(&left.created_at, &right.created_at))
        .then_with(|| left.path.cmp(&right.path))
}

fn compare_optional<T: Ord>(left: &Option<T>, right: &Option<T>) -> Ordering {
    match (left, right) {
        (Some(left), Some(right)) => left.cmp(right),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    }
}

fn select_initial_events(events: &[ArtifactEvent], last: Option<usize>) -> Vec<ArtifactEvent> {
    let start = match last {
        Some(last) if last < events.len() => events.len() - last,
        _ => 0,
    };
    events[start..].to_vec()
}

#[cfg(test)]
mod tests {
    use std::fs;

    use chrono::{DateTime, NaiveDateTime, Utc};
    use tempfile::TempDir;

    use super::{
        collect_markdown_files, compare_events, discover_artifact_events,
        parse_created_at_frontmatter, parse_filename_timestamp, select_initial_events,
        sort_artifact_events, ArtifactEvent,
    };

    #[test]
    fn parse_filename_timestamp_accepts_valid_prefix() {
        let path = std::path::Path::new("loops/001-x/20260206210337-spec.md");
        let parsed = parse_filename_timestamp(path).expect("timestamp should parse");
        assert_eq!(parsed.format("%Y%m%d%H%M%S").to_string(), "20260206210337");
    }

    #[test]
    fn parse_filename_timestamp_rejects_invalid_prefix() {
        let path = std::path::Path::new("loops/001-x/spec.md");
        assert!(parse_filename_timestamp(path).is_none());
    }

    #[test]
    fn parse_created_at_frontmatter_reads_rfc3339() {
        let raw = "---\nartifact: spec\ncreated_at: 2026-02-06T21:03:37Z\n---\n\n# Feature";
        let parsed = parse_created_at_frontmatter(raw).expect("created_at should parse");
        assert_eq!(
            parsed.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
            "2026-02-06T21:03:37Z"
        );
    }

    #[test]
    fn parse_created_at_frontmatter_returns_none_without_frontmatter() {
        let raw = "# Feature: something";
        assert!(parse_created_at_frontmatter(raw).is_none());
    }

    #[test]
    fn compare_events_uses_timestamp_then_created_at_then_path() {
        let ts = Some(NaiveDateTime::parse_from_str("20260206210000", "%Y%m%d%H%M%S").unwrap());
        let created_1 = Some(
            DateTime::parse_from_rfc3339("2026-02-06T21:00:01Z")
                .unwrap()
                .with_timezone(&Utc),
        );
        let created_2 = Some(
            DateTime::parse_from_rfc3339("2026-02-06T21:00:02Z")
                .unwrap()
                .with_timezone(&Utc),
        );

        let a = ArtifactEvent {
            project: "01-plan".to_owned(),
            path: "loops/001-a/20260206210000-spec.md".to_owned(),
            filename_timestamp: ts,
            created_at: created_2,
        };
        let b = ArtifactEvent {
            project: "01-plan".to_owned(),
            path: "loops/001-b/20260206210000-spec.md".to_owned(),
            filename_timestamp: ts,
            created_at: created_1,
        };
        let c = ArtifactEvent {
            project: "01-plan".to_owned(),
            path: "loops/001-c/20260206210001-spec.md".to_owned(),
            filename_timestamp: Some(
                NaiveDateTime::parse_from_str("20260206210001", "%Y%m%d%H%M%S").unwrap(),
            ),
            created_at: created_1,
        };

        assert_eq!(compare_events(&a, &b), std::cmp::Ordering::Greater);
        assert_eq!(compare_events(&b, &c), std::cmp::Ordering::Less);
    }

    #[test]
    fn sort_artifact_events_orders_deterministically() {
        let ts = Some(NaiveDateTime::parse_from_str("20260206210000", "%Y%m%d%H%M%S").unwrap());
        let created = Some(
            DateTime::parse_from_rfc3339("2026-02-06T21:03:37Z")
                .unwrap()
                .with_timezone(&Utc),
        );

        let mut events = vec![
            ArtifactEvent {
                project: "01-plan".to_owned(),
                path: "loops/001-z/20260206210000-spec.md".to_owned(),
                filename_timestamp: ts,
                created_at: created,
            },
            ArtifactEvent {
                project: "01-plan".to_owned(),
                path: "loops/001-a/20260206210000-spec.md".to_owned(),
                filename_timestamp: ts,
                created_at: created,
            },
        ];

        sort_artifact_events(&mut events);
        assert_eq!(events[0].path, "loops/001-a/20260206210000-spec.md");
        assert_eq!(events[1].path, "loops/001-z/20260206210000-spec.md");
    }

    #[test]
    fn collect_markdown_files_returns_empty_when_missing() {
        let temp = TempDir::new().unwrap();
        let missing = temp.path().join("loops");
        let files = collect_markdown_files(&missing).unwrap();
        assert!(files.is_empty());
    }

    #[test]
    fn discover_artifact_events_reads_and_sorts_markdown_files() {
        let temp = TempDir::new().unwrap();
        let project_dir = temp.path().join("01-plan");
        let loop_dir = project_dir.join("loops/001-tail");
        fs::create_dir_all(&loop_dir).unwrap();

        fs::write(
            loop_dir.join("20260206210002-impl-notes.md"),
            "---\ncreated_at: 2026-02-06T21:00:03Z\n---\n\n# Implementation Notes",
        )
        .unwrap();
        fs::write(
            loop_dir.join("20260206210001-spec.md"),
            "---\ncreated_at: 2026-02-06T21:00:01Z\n---\n\n# Feature",
        )
        .unwrap();
        fs::write(loop_dir.join("ignore.txt"), "not markdown").unwrap();

        let events = discover_artifact_events(&project_dir, "01-plan").unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].path, "loops/001-tail/20260206210001-spec.md");
        assert_eq!(
            events[1].path,
            "loops/001-tail/20260206210002-impl-notes.md"
        );
    }

    #[test]
    fn select_initial_events_respects_last() {
        let events = vec![
            ArtifactEvent {
                project: "01-plan".to_owned(),
                path: "loops/001-a/1.md".to_owned(),
                filename_timestamp: None,
                created_at: None,
            },
            ArtifactEvent {
                project: "01-plan".to_owned(),
                path: "loops/001-a/2.md".to_owned(),
                filename_timestamp: None,
                created_at: None,
            },
            ArtifactEvent {
                project: "01-plan".to_owned(),
                path: "loops/001-a/3.md".to_owned(),
                filename_timestamp: None,
                created_at: None,
            },
        ];

        assert_eq!(select_initial_events(&events, Some(2)).len(), 2);
        assert_eq!(select_initial_events(&events, Some(0)).len(), 0);
        assert_eq!(select_initial_events(&events, None).len(), 3);
    }
}
