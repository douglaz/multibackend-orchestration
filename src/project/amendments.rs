use std::ffi::OsStr;
use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tracing::warn;

use crate::error::RalphError;
use crate::util::time::now_timestamp_yyyymmddhhmmss;
use crate::Result;

const QUEUE_DIR_NAME: &str = "amendment-queue";
const QUARANTINE_DIR_NAME: &str = ".quarantine";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AmendmentRequest {
    pub id: String,
    pub body: String,
    #[serde(default)]
    pub priority: AmendmentPriority,
    pub source: AmendmentSource,
    #[serde(default)]
    pub source_detail: Option<String>,
    pub created_at: DateTime<Utc>,
}

impl AmendmentRequest {
    fn validate(&self) -> Result<()> {
        if self.id.trim().is_empty() {
            return Err(RalphError::Validation(
                "amendment id cannot be empty".to_owned(),
            ));
        }
        if self.body.trim().is_empty() {
            return Err(RalphError::Validation(
                "amendment body cannot be empty".to_owned(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Default)]
pub enum AmendmentPriority {
    P0,
    P1,
    #[default]
    P2,
    P3,
}

impl AmendmentPriority {
    fn as_str(&self) -> &'static str {
        match self {
            Self::P0 => "P0",
            Self::P1 => "P1",
            Self::P2 => "P2",
            Self::P3 => "P3",
        }
    }
}

impl fmt::Display for AmendmentPriority {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum AmendmentSource {
    Cli,
    FinalReview,
    File,
}

impl AmendmentSource {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Cli => "cli",
            Self::FinalReview => "final-review",
            Self::File => "file",
        }
    }
}

impl fmt::Display for AmendmentSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

pub fn sanitize_id(id: &str) -> String {
    id.chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-') {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

pub fn enqueue_amendment(project_dir: &Path, req: &AmendmentRequest) -> Result<PathBuf> {
    enqueue_amendment_with_timestamp(project_dir, req, &now_timestamp_yyyymmddhhmmss())
}

fn enqueue_amendment_with_timestamp(
    project_dir: &Path,
    req: &AmendmentRequest,
    timestamp: &str,
) -> Result<PathBuf> {
    req.validate()?;

    let queue_dir = amendment_queue_dir(project_dir);
    fs::create_dir_all(&queue_dir)?;

    let sanitized_id = sanitize_id(&req.id);
    let mut final_path = unique_queue_path(&queue_dir, timestamp, &sanitized_id);
    let payload = serde_json::to_vec(req)?;

    loop {
        let temp_path = write_payload_to_temp_file(&queue_dir, &payload)?;
        match fs::rename(&temp_path, &final_path) {
            Ok(()) => return Ok(final_path),
            Err(err) if err.kind() == ErrorKind::AlreadyExists => {
                let _ = fs::remove_file(&temp_path);
                final_path = unique_queue_path(&queue_dir, timestamp, &sanitized_id);
            }
            Err(err) => {
                let _ = fs::remove_file(&temp_path);
                return Err(err.into());
            }
        }
    }
}

pub fn drain_amendment_queue(project_dir: &Path) -> Result<Vec<AmendmentRequest>> {
    let queue_dir = amendment_queue_dir(project_dir);
    if !queue_dir.exists() {
        return Ok(Vec::new());
    }

    let mut queue_files = Vec::new();
    for entry in fs::read_dir(&queue_dir)? {
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            continue;
        }
        let path = entry.path();
        if has_extension(&path, "json") || has_extension(&path, "inflight") {
            queue_files.push(path);
        }
    }

    queue_files.sort_by_key(queue_sort_key);

    let mut drained = Vec::new();
    for queued_path in queue_files {
        let inflight_path = if has_extension(&queued_path, "json") {
            let claimed_path = queued_path.with_extension("inflight");
            match fs::rename(&queued_path, &claimed_path) {
                Ok(()) => claimed_path,
                Err(err) if err.kind() == ErrorKind::NotFound => continue,
                Err(err) => return Err(err.into()),
            }
        } else {
            queued_path
        };

        let parsed = match parse_inflight_request(&inflight_path) {
            Ok(req) => req,
            Err(err) => {
                warn!(
                    path = %inflight_path.display(),
                    error = %err,
                    "failed to parse amendment request; quarantining file"
                );
                if let Err(quarantine_err) = quarantine_inflight_file(&queue_dir, &inflight_path) {
                    warn!(
                        path = %inflight_path.display(),
                        error = %quarantine_err,
                        "failed to quarantine malformed amendment request"
                    );
                }
                continue;
            }
        };

        fs::remove_file(&inflight_path)?;
        drained.push(parsed);
    }

    Ok(drained)
}

pub fn pending_amendment_count(project_dir: &Path) -> Result<usize> {
    let queue_dir = amendment_queue_dir(project_dir);
    if !queue_dir.exists() {
        return Ok(0);
    }

    let mut count = 0usize;
    for entry in fs::read_dir(queue_dir)? {
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            continue;
        }
        let path = entry.path();
        if has_extension(&path, "json") || has_extension(&path, "inflight") {
            count += 1;
        }
    }

    Ok(count)
}

pub fn format_external_amendments_for_prompt(amendments: &[AmendmentRequest]) -> String {
    if amendments.is_empty() {
        return String::new();
    }

    let mut out = String::new();
    for (idx, amendment) in amendments.iter().enumerate() {
        if idx > 0 {
            out.push_str("\n\n");
        }
        out.push_str(&format!("### Amendment {}\n", idx + 1));
        out.push_str(&format!("- id: {}\n", amendment.id));
        out.push_str(&format!("- priority: {}\n", amendment.priority));
        match amendment.source_detail.as_deref() {
            Some(detail) if !detail.trim().is_empty() => {
                out.push_str(&format!("- source: {} ({detail})\n", amendment.source));
            }
            _ => {
                out.push_str(&format!("- source: {}\n", amendment.source));
            }
        }
        out.push_str("- body:\n");
        for line in amendment.body.lines() {
            out.push_str("  ");
            out.push_str(line);
            out.push('\n');
        }
        if amendment.body.is_empty() {
            out.push_str("  \n");
        }
    }

    out
}

fn amendment_queue_dir(project_dir: &Path) -> PathBuf {
    project_dir.join(QUEUE_DIR_NAME)
}

fn unique_queue_path(queue_dir: &Path, timestamp: &str, sanitized_id: &str) -> PathBuf {
    let mut suffix = 0usize;
    loop {
        let filename = if suffix == 0 {
            format!("{timestamp}-{sanitized_id}.json")
        } else {
            format!("{timestamp}-{sanitized_id}-{suffix}.json")
        };
        let candidate = queue_dir.join(filename);
        if !candidate.exists() {
            return candidate;
        }
        suffix += 1;
    }
}

fn write_payload_to_temp_file(queue_dir: &Path, payload: &[u8]) -> Result<PathBuf> {
    loop {
        let temp_name = format!(".tmp-{}.json", uuid_like());
        let temp_path = queue_dir.join(temp_name);
        match OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temp_path)
        {
            Ok(mut file) => {
                file.write_all(payload)?;
                file.sync_all()?;
                return Ok(temp_path);
            }
            Err(err) if err.kind() == ErrorKind::AlreadyExists => continue,
            Err(err) => return Err(err.into()),
        }
    }
}

fn queue_sort_key(path: &PathBuf) -> String {
    path.file_name()
        .map_or_else(String::new, |name| name.to_string_lossy().to_string())
}

fn has_extension(path: &Path, ext: &str) -> bool {
    path.extension()
        .and_then(OsStr::to_str)
        .is_some_and(|candidate| candidate.eq_ignore_ascii_case(ext))
}

fn parse_inflight_request(path: &Path) -> Result<AmendmentRequest> {
    let content = fs::read_to_string(path)?;
    let req: AmendmentRequest = serde_json::from_str(&content)?;
    req.validate()?;
    Ok(req)
}

fn quarantine_inflight_file(queue_dir: &Path, inflight_path: &Path) -> Result<()> {
    let quarantine_dir = queue_dir.join(QUARANTINE_DIR_NAME);
    fs::create_dir_all(&quarantine_dir)?;

    let stem = inflight_path
        .file_stem()
        .and_then(OsStr::to_str)
        .unwrap_or("amendment");
    let extension = inflight_path.extension().and_then(OsStr::to_str);
    let timestamp = now_timestamp_yyyymmddhhmmss();

    let mut suffix = 0usize;
    loop {
        let mut filename = if suffix == 0 {
            format!("{stem}-{timestamp}")
        } else {
            format!("{stem}-{timestamp}-{suffix}")
        };
        if let Some(ext) = extension {
            filename.push('.');
            filename.push_str(ext);
        }

        let target = quarantine_dir.join(filename);
        match fs::rename(inflight_path, &target) {
            Ok(()) => return Ok(()),
            Err(err) if err.kind() == ErrorKind::AlreadyExists => {
                suffix += 1;
            }
            Err(err) if err.kind() == ErrorKind::NotFound => return Ok(()),
            Err(err) => return Err(err.into()),
        }
    }
}

fn uuid_like() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);

    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let count = u128::from(COUNTER.fetch_add(1, Ordering::Relaxed));
    let pid = u128::from(std::process::id());
    let mut value = nanos ^ (count << 16) ^ (pid << 80);
    if value == 0 {
        value = 1;
    }

    let hex = format!("{value:032x}");
    format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use tempfile::tempdir;

    #[test]
    fn sanitize_id_replaces_unsupported_characters() {
        assert_eq!(sanitize_id("EXT-1"), "EXT-1");
        assert_eq!(sanitize_id("a b/c@d"), "a_b_c_d");
        assert_eq!(sanitize_id("日本語"), "___");
        assert_eq!(sanitize_id("a.b_c-d"), "a.b_c-d");
    }

    #[test]
    fn enqueue_uses_timestamped_sanitized_name_and_atomic_handoff() {
        let temp = tempdir().expect("tempdir");
        let project_dir = temp.path();
        let timestamp = "20260309030000";
        let req = sample_request("EXT/123", "Add retry guard");

        let path = enqueue_amendment_with_timestamp(project_dir, &req, timestamp).expect("enqueue");
        let file_name = path
            .file_name()
            .and_then(OsStr::to_str)
            .expect("file name should be utf-8");
        assert_eq!(file_name, "20260309030000-EXT_123.json");
        assert!(path.exists(), "final queue file should exist");

        let queue_dir = amendment_queue_dir(project_dir);
        let tmp_count = fs::read_dir(queue_dir)
            .expect("read queue")
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.file_name().to_string_lossy().to_string())
            .filter(|name| name.starts_with(".tmp-") && name.ends_with(".json"))
            .count();
        assert_eq!(tmp_count, 0, "temporary files should be renamed away");
    }

    #[test]
    fn enqueue_appends_numeric_suffix_when_target_exists() {
        let temp = tempdir().expect("tempdir");
        let project_dir = temp.path();
        let timestamp = "20260309030001";
        let req = sample_request("EXT-1", "Body");

        let queue_dir = amendment_queue_dir(project_dir);
        fs::create_dir_all(&queue_dir).expect("create queue dir");
        fs::write(queue_dir.join("20260309030001-EXT-1.json"), "{}").expect("write existing file");

        let path =
            enqueue_amendment_with_timestamp(project_dir, &req, timestamp).expect("enqueue with suffix");
        let file_name = path
            .file_name()
            .and_then(OsStr::to_str)
            .expect("file name should be utf-8");
        assert_eq!(file_name, "20260309030001-EXT-1-1.json");
    }

    #[test]
    fn drain_processes_queue_files_in_deterministic_lexicographic_order() {
        let temp = tempdir().expect("tempdir");
        let project_dir = temp.path();
        let queue_dir = amendment_queue_dir(project_dir);
        fs::create_dir_all(&queue_dir).expect("create queue");

        write_request_file(
            &queue_dir.join("20260309030002-z.json"),
            &sample_request("z", "z body"),
        );
        write_request_file(
            &queue_dir.join("20260309030001-b.json"),
            &sample_request("b", "b body"),
        );
        write_request_file(
            &queue_dir.join("20260309030001-a.inflight"),
            &sample_request("a", "a body"),
        );

        let drained = drain_amendment_queue(project_dir).expect("drain queue");
        let ids: Vec<_> = drained.iter().map(|req| req.id.clone()).collect();
        assert_eq!(ids, vec!["a", "b", "z"]);
    }

    #[test]
    fn drain_removes_processed_files_after_successful_parse() {
        let temp = tempdir().expect("tempdir");
        let project_dir = temp.path();
        let queue_dir = amendment_queue_dir(project_dir);
        fs::create_dir_all(&queue_dir).expect("create queue");
        write_request_file(
            &queue_dir.join("20260309030003-item.json"),
            &sample_request("item", "payload"),
        );

        let drained = drain_amendment_queue(project_dir).expect("drain queue");
        assert_eq!(drained.len(), 1);
        assert_eq!(pending_amendment_count(project_dir).expect("pending count"), 0);

        let leftover = fs::read_dir(&queue_dir)
            .expect("read queue")
            .filter_map(|entry| entry.ok())
            .filter(|entry| {
                let path = entry.path();
                has_extension(&path, "json") || has_extension(&path, "inflight")
            })
            .count();
        assert_eq!(leftover, 0);
    }

    #[test]
    fn drain_recovers_and_processes_existing_inflight_files() {
        let temp = tempdir().expect("tempdir");
        let project_dir = temp.path();
        let queue_dir = amendment_queue_dir(project_dir);
        fs::create_dir_all(&queue_dir).expect("create queue");
        write_request_file(
            &queue_dir.join("20260309030004-recovered.inflight"),
            &sample_request("recovered", "resume after crash"),
        );

        let drained = drain_amendment_queue(project_dir).expect("drain queue");
        assert_eq!(drained.len(), 1);
        assert_eq!(drained[0].id, "recovered");
        assert_eq!(pending_amendment_count(project_dir).expect("pending count"), 0);
    }

    #[test]
    fn malformed_json_is_quarantined_and_drain_continues() {
        let temp = tempdir().expect("tempdir");
        let project_dir = temp.path();
        let queue_dir = amendment_queue_dir(project_dir);
        fs::create_dir_all(&queue_dir).expect("create queue");
        fs::write(
            queue_dir.join("20260309030005-bad.json"),
            "{ not valid json",
        )
        .expect("write malformed");

        let drained = drain_amendment_queue(project_dir).expect("drain queue");
        assert!(drained.is_empty(), "malformed input should be skipped");
        assert_eq!(pending_amendment_count(project_dir).expect("pending count"), 0);

        let quarantine_dir = queue_dir.join(QUARANTINE_DIR_NAME);
        assert!(quarantine_dir.exists(), "quarantine directory should be created");
        let quarantine_entries: Vec<_> = fs::read_dir(&quarantine_dir)
            .expect("read quarantine")
            .filter_map(|entry| entry.ok())
            .collect();
        assert_eq!(quarantine_entries.len(), 1);

        let quarantined_name = quarantine_entries[0]
            .file_name()
            .to_string_lossy()
            .to_string();
        assert!(
            quarantined_name.starts_with("20260309030005-bad-"),
            "unexpected quarantine file name: {quarantined_name}"
        );
        assert!(
            quarantined_name.ends_with(".inflight"),
            "quarantine file should keep inflight extension: {quarantined_name}"
        );
    }

    #[test]
    fn missing_or_empty_queue_returns_empty_results_and_zero_count() {
        let temp = tempdir().expect("tempdir");
        let project_dir = temp.path();

        let drained = drain_amendment_queue(project_dir).expect("drain missing queue");
        assert!(drained.is_empty());
        assert_eq!(
            pending_amendment_count(project_dir).expect("count missing queue"),
            0
        );

        fs::create_dir_all(amendment_queue_dir(project_dir)).expect("create empty queue");
        let drained = drain_amendment_queue(project_dir).expect("drain empty queue");
        assert!(drained.is_empty());
        assert_eq!(
            pending_amendment_count(project_dir).expect("count empty queue"),
            0
        );
    }

    #[test]
    fn missing_priority_deserializes_to_p2_by_default() {
        let raw = r#"{
            "id":"EXT-99",
            "body":"Ship fix",
            "source":"cli",
            "created_at":"2026-03-09T02:52:14Z"
        }"#;
        let req: AmendmentRequest = serde_json::from_str(raw).expect("deserialize amendment");
        assert_eq!(req.priority, AmendmentPriority::P2);
    }

    #[test]
    fn amendment_request_serialization_roundtrip() {
        let req = AmendmentRequest {
            id: "EXT-123".to_owned(),
            body: "Line one\nLine two".to_owned(),
            priority: AmendmentPriority::P1,
            source: AmendmentSource::FinalReview,
            source_detail: Some("claude(opus)".to_owned()),
            created_at: Utc
                .with_ymd_and_hms(2026, 3, 9, 2, 52, 14)
                .single()
                .expect("valid datetime"),
        };

        let encoded = serde_json::to_string(&req).expect("serialize");
        let decoded: AmendmentRequest = serde_json::from_str(&encoded).expect("deserialize");
        assert_eq!(decoded, req);
    }

    #[test]
    fn formatter_returns_empty_for_no_amendments() {
        let out = format_external_amendments_for_prompt(&[]);
        assert!(out.is_empty());
    }

    #[test]
    fn formatter_lists_required_fields() {
        let req = AmendmentRequest {
            id: "EXT-777".to_owned(),
            body: "Please add retry logic.".to_owned(),
            priority: AmendmentPriority::P0,
            source: AmendmentSource::Cli,
            source_detail: Some("manual".to_owned()),
            created_at: Utc
                .with_ymd_and_hms(2026, 3, 9, 2, 52, 14)
                .single()
                .expect("valid datetime"),
        };

        let out = format_external_amendments_for_prompt(&[req]);
        assert!(out.contains("id: EXT-777"));
        assert!(out.contains("priority: P0"));
        assert!(out.contains("source: cli (manual)"));
        assert!(out.contains("Please add retry logic."));
    }

    fn sample_request(id: &str, body: &str) -> AmendmentRequest {
        AmendmentRequest {
            id: id.to_owned(),
            body: body.to_owned(),
            priority: AmendmentPriority::P2,
            source: AmendmentSource::Cli,
            source_detail: None,
            created_at: Utc
                .with_ymd_and_hms(2026, 3, 9, 2, 52, 14)
                .single()
                .expect("valid datetime"),
        }
    }

    fn write_request_file(path: &Path, req: &AmendmentRequest) {
        let payload = serde_json::to_string(req).expect("serialize");
        fs::write(path, payload).expect("write request file");
    }
}
