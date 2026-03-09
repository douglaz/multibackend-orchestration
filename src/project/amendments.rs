use std::collections::HashMap;
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
    enqueue_amendment_with_timestamp_and_hook(
        project_dir,
        req,
        &now_timestamp_yyyymmddhhmmss(),
        |_| Ok(()),
    )
}

#[cfg(test)]
fn enqueue_amendment_with_timestamp(
    project_dir: &Path,
    req: &AmendmentRequest,
    timestamp: &str,
) -> Result<PathBuf> {
    enqueue_amendment_with_timestamp_and_hook(project_dir, req, timestamp, |_| Ok(()))
}

fn enqueue_amendment_with_timestamp_and_hook<F>(
    project_dir: &Path,
    req: &AmendmentRequest,
    timestamp: &str,
    mut before_publish: F,
) -> Result<PathBuf>
where
    F: FnMut(&Path) -> Result<()>,
{
    req.validate()?;

    let queue_dir = amendment_queue_dir(project_dir);
    fs::create_dir_all(&queue_dir)?;

    let sanitized_id = sanitize_id(&req.id);
    let payload = serde_json::to_vec(req)?;
    let mut suffix = 0usize;

    loop {
        let final_path = queue_path_with_suffix(&queue_dir, timestamp, &sanitized_id, suffix);
        // Treat an existing .inflight sibling as an occupied stem so we never
        // create a .json that shares a stem with an in-progress drain claim.
        let inflight_sibling = final_path.with_extension("inflight");
        if inflight_sibling.exists() {
            suffix += 1;
            continue;
        }
        let temp_path = write_payload_to_temp_file(&queue_dir, &payload)?;
        before_publish(&final_path)?;
        match claim_file_without_overwrite(&temp_path, &final_path)? {
            FileClaimOutcome::Claimed => return Ok(final_path),
            FileClaimOutcome::DestinationExists => {
                let _ = fs::remove_file(&temp_path);
                suffix += 1;
            }
            FileClaimOutcome::SourceMissing => {
                suffix += 1;
            }
        }
    }
}

pub fn drain_amendment_queue(project_dir: &Path) -> Result<Vec<AmendmentRequest>> {
    drain_amendment_queue_with_hook(project_dir, |_, _| Ok(()))
}

fn drain_amendment_queue_with_hook<F>(
    project_dir: &Path,
    mut before_json_claim: F,
) -> Result<Vec<AmendmentRequest>>
where
    F: FnMut(&Path, &Path) -> Result<()>,
{
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
        if is_queue_file_for_drain(&path) {
            queue_files.push(path);
        }
    }

    queue_files.sort_by_key(queue_sort_key);

    let mut drained = Vec::new();
    let mut completed_inflight_items: HashMap<String, AmendmentRequest> = HashMap::new();
    for queued_path in queue_files {
        if has_extension(&queued_path, "json") {
            if let Some(stem) = queue_stem(&queued_path) {
                if let Some(prev) = completed_inflight_items.get(&stem) {
                    // Same-stem .json found after its .inflight was already processed.
                    // Claim .json → .inflight before reading (same lifecycle as the
                    // main path), then compare content for dedup vs. distinct handling.
                    let claimed_path = queued_path.with_extension("inflight");
                    if let Err(err) = before_json_claim(&queued_path, &claimed_path) {
                        return Err(rollback_mid_drain(project_dir, drained, err));
                    }
                    let inflight_path =
                        match claim_file_without_overwrite(&queued_path, &claimed_path) {
                            Ok(FileClaimOutcome::Claimed) => claimed_path,
                            Ok(
                                FileClaimOutcome::DestinationExists
                                | FileClaimOutcome::SourceMissing,
                            ) => {
                                continue;
                            }
                            Err(io_err) => {
                                return Err(rollback_mid_drain(
                                    project_dir,
                                    drained,
                                    io_err.into(),
                                ));
                            }
                        };
                    match read_and_parse_inflight(&inflight_path) {
                        InflightReadOutcome::Parsed(ref req) if req == prev => {
                            if let Err(err) = remove_file_if_exists(&inflight_path) {
                                return Err(rollback_mid_drain(project_dir, drained, err));
                            }
                        }
                        InflightReadOutcome::Parsed(req) => {
                            if let Err(io_err) = fs::remove_file(&inflight_path) {
                                return Err(rollback_mid_drain(
                                    project_dir,
                                    drained,
                                    io_err.into(),
                                ));
                            }
                            drained.push(req);
                        }
                        InflightReadOutcome::Malformed(err) => {
                            warn!(
                                path = %inflight_path.display(),
                                error = %err,
                                "failed to parse same-stem amendment; quarantining file"
                            );
                            if let Err(q_err) =
                                quarantine_inflight_file(&queue_dir, &inflight_path)
                            {
                                warn!(
                                    path = %inflight_path.display(),
                                    error = %q_err,
                                    "failed to quarantine malformed same-stem amendment"
                                );
                            }
                        }
                        InflightReadOutcome::ReadFailed(io_err) => {
                            return Err(rollback_mid_drain(
                                project_dir,
                                drained,
                                io_err.into(),
                            ));
                        }
                    }
                    continue;
                }
            }
        }

        let inflight_path = if has_extension(&queued_path, "json") {
            let claimed_path = queued_path.with_extension("inflight");
            if let Err(err) = before_json_claim(&queued_path, &claimed_path) {
                return Err(rollback_mid_drain(project_dir, drained, err));
            }
            match claim_file_without_overwrite(&queued_path, &claimed_path) {
                Ok(FileClaimOutcome::Claimed) => claimed_path,
                Ok(FileClaimOutcome::DestinationExists | FileClaimOutcome::SourceMissing) => {
                    continue
                }
                Err(io_err) => {
                    return Err(rollback_mid_drain(project_dir, drained, io_err.into()));
                }
            }
        } else {
            queued_path
        };

        let parsed = match read_and_parse_inflight(&inflight_path) {
            InflightReadOutcome::Parsed(req) => req,
            InflightReadOutcome::Malformed(err) => {
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
            InflightReadOutcome::ReadFailed(io_err) => {
                return Err(rollback_mid_drain(project_dir, drained, io_err.into()));
            }
        };

        if let Err(io_err) = fs::remove_file(&inflight_path) {
            return Err(rollback_mid_drain(project_dir, drained, io_err.into()));
        }
        if let Some(stem) = queue_stem(&inflight_path) {
            completed_inflight_items.insert(stem, parsed.clone());
        }
        drained.push(parsed);
    }

    Ok(drained)
}

/// Best-effort rollback of already-drained amendments after a mid-drain failure.
/// Re-enqueues all items in `drained`, then returns an appropriate error.
/// If rollback is partial, the returned error includes the unrestored amendment IDs.
fn rollback_mid_drain(
    project_dir: &Path,
    drained: Vec<AmendmentRequest>,
    original_error: RalphError,
) -> RalphError {
    if drained.is_empty() {
        return original_error;
    }
    let failed_ids = re_enqueue_amendments(project_dir, &drained);
    if failed_ids.is_empty() {
        warn!(
            count = drained.len(),
            "re-enqueued drained amendments after mid-drain failure"
        );
        original_error
    } else {
        RalphError::Orchestration(format!(
            "{original_error}; mid-drain rollback partially failed, could not restore IDs: [{}]",
            failed_ids.join(", ")
        ))
    }
}

/// Re-enqueue previously drained amendments back to the queue.
/// Returns a list of amendment IDs that could not be re-enqueued.
/// Successfully re-enqueued items remain queued even if later items fail.
pub fn re_enqueue_amendments(
    project_dir: &Path,
    amendments: &[AmendmentRequest],
) -> Vec<String> {
    let mut failed_ids = Vec::new();
    for req in amendments {
        if let Err(err) = enqueue_amendment(project_dir, req) {
            warn!(
                id = %req.id,
                error = %err,
                "failed to re-enqueue amendment during rollback"
            );
            failed_ids.push(req.id.clone());
        }
    }
    failed_ids
}

/// Attempt to re-enqueue drained amendments after a phase failure.
/// Returns the original error if all amendments are restored.
/// Returns a combined error (original + unrestored IDs) on partial failure.
pub fn rollback_drained_amendments(
    project_dir: &Path,
    drained: &[AmendmentRequest],
    phase_error: RalphError,
) -> RalphError {
    if drained.is_empty() {
        return phase_error;
    }
    let failed_ids = re_enqueue_amendments(project_dir, drained);
    if failed_ids.is_empty() {
        warn!(
            count = drained.len(),
            "re-enqueued drained amendments after phase failure"
        );
        phase_error
    } else {
        RalphError::Orchestration(format!(
            "{phase_error}; amendment rollback partially failed, could not restore IDs: [{}]",
            failed_ids.join(", ")
        ))
    }
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
        if is_queue_file_for_drain(&path) {
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

fn queue_path_with_suffix(
    queue_dir: &Path,
    timestamp: &str,
    sanitized_id: &str,
    suffix: usize,
) -> PathBuf {
    let filename = if suffix == 0 {
        format!("{timestamp}-{sanitized_id}.json")
    } else {
        format!("{timestamp}-{sanitized_id}-{suffix}.json")
    };
    queue_dir.join(filename)
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FileClaimOutcome {
    Claimed,
    DestinationExists,
    SourceMissing,
}

fn claim_file_without_overwrite(
    source: &Path,
    destination: &Path,
) -> std::io::Result<FileClaimOutcome> {
    // Plain rename is not safe for collision detection on Unix because it overwrites
    // existing destinations. hard_link gives us a no-overwrite claim primitive.
    match fs::hard_link(source, destination) {
        Ok(()) => match fs::remove_file(source) {
            Ok(()) => Ok(FileClaimOutcome::Claimed),
            Err(err) if err.kind() == ErrorKind::NotFound => Ok(FileClaimOutcome::Claimed),
            Err(err) => Err(err),
        },
        Err(err) if err.kind() == ErrorKind::AlreadyExists => {
            Ok(FileClaimOutcome::DestinationExists)
        }
        Err(err) if err.kind() == ErrorKind::NotFound => Ok(FileClaimOutcome::SourceMissing),
        Err(err) => Err(err),
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

fn is_temp_queue_file(path: &Path) -> bool {
    has_extension(path, "json")
        && path
            .file_name()
            .and_then(OsStr::to_str)
            .is_some_and(|name| name.starts_with(".tmp-"))
}

fn is_queue_file_for_drain(path: &Path) -> bool {
    has_extension(path, "inflight") || (has_extension(path, "json") && !is_temp_queue_file(path))
}

/// Outcome of reading and parsing an inflight amendment file.
/// Separates I/O read failures (fatal) from content failures (quarantinable).
enum InflightReadOutcome {
    /// Successfully read and validated the amendment.
    Parsed(AmendmentRequest),
    /// File was readable but content was invalid (bad JSON or validation failure).
    Malformed(RalphError),
    /// File could not be read due to an I/O error (e.g. PermissionDenied).
    ReadFailed(std::io::Error),
}

fn read_and_parse_inflight(path: &Path) -> InflightReadOutcome {
    let bytes = match fs::read(path) {
        Ok(b) => b,
        Err(io_err) => return InflightReadOutcome::ReadFailed(io_err),
    };
    let req: AmendmentRequest = match serde_json::from_slice(&bytes) {
        Ok(r) => r,
        Err(err) => return InflightReadOutcome::Malformed(err.into()),
    };
    match req.validate() {
        Ok(()) => InflightReadOutcome::Parsed(req),
        Err(err) => InflightReadOutcome::Malformed(err),
    }
}

fn remove_file_if_exists(path: &Path) -> Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err.into()),
    }
}

fn queue_stem(path: &Path) -> Option<String> {
    path.file_stem()
        .and_then(OsStr::to_str)
        .map(ToOwned::to_owned)
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
        match claim_file_without_overwrite(inflight_path, &target)? {
            FileClaimOutcome::Claimed | FileClaimOutcome::SourceMissing => return Ok(()),
            FileClaimOutcome::DestinationExists => {
                suffix += 1;
            }
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

        let path = enqueue_amendment_with_timestamp(project_dir, &req, timestamp)
            .expect("enqueue with suffix");
        let file_name = path
            .file_name()
            .and_then(OsStr::to_str)
            .expect("file name should be utf-8");
        assert_eq!(file_name, "20260309030001-EXT-1-1.json");
    }

    #[test]
    fn enqueue_avoids_overwriting_file_that_appears_before_publish() {
        let temp = tempdir().expect("tempdir");
        let project_dir = temp.path();
        let queue_dir = amendment_queue_dir(project_dir);
        fs::create_dir_all(&queue_dir).expect("create queue dir");

        let timestamp = "20260309030006";
        let first_candidate = queue_dir.join("20260309030006-EXT-1.json");
        let req = sample_request("EXT-1", "new body");
        let mut injected = false;

        let path =
            enqueue_amendment_with_timestamp_and_hook(project_dir, &req, timestamp, |candidate| {
                if !injected {
                    assert_eq!(candidate, first_candidate.as_path());
                    fs::write(candidate, r#"{"winner":"other-writer"}"#)?;
                    injected = true;
                }
                Ok(())
            })
            .expect("enqueue with concurrent destination");

        assert_eq!(
            path.file_name().and_then(OsStr::to_str),
            Some("20260309030006-EXT-1-1.json")
        );

        let original = fs::read_to_string(&first_candidate).expect("read preserved destination");
        assert_eq!(original, r#"{"winner":"other-writer"}"#);

        let created_payload = fs::read_to_string(&path).expect("read new queued amendment");
        let created_request: AmendmentRequest =
            serde_json::from_str(&created_payload).expect("deserialize queued amendment");
        assert_eq!(created_request.id, "EXT-1");
        assert_eq!(created_request.body, "new body");
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
        assert_eq!(
            pending_amendment_count(project_dir).expect("pending count"),
            0
        );

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
        assert_eq!(
            pending_amendment_count(project_dir).expect("pending count"),
            0
        );
    }

    #[test]
    fn drain_processes_same_stem_json_and_inflight_only_once() {
        let temp = tempdir().expect("tempdir");
        let project_dir = temp.path();
        let queue_dir = amendment_queue_dir(project_dir);
        fs::create_dir_all(&queue_dir).expect("create queue");

        let req = sample_request("dup", "claim interrupted");
        write_request_file(&queue_dir.join("20260309030008-dup.json"), &req);
        write_request_file(&queue_dir.join("20260309030008-dup.inflight"), &req);

        let drained = drain_amendment_queue(project_dir).expect("drain queue");
        let ids: Vec<_> = drained.iter().map(|item| item.id.as_str()).collect();
        assert_eq!(ids, vec!["dup"]);
        assert_eq!(
            pending_amendment_count(project_dir).expect("pending count"),
            0
        );
    }

    #[test]
    fn drain_processes_same_stem_json_and_inflight_with_different_content() {
        let temp = tempdir().expect("tempdir");
        let project_dir = temp.path();
        let queue_dir = amendment_queue_dir(project_dir);
        fs::create_dir_all(&queue_dir).expect("create queue");

        // Same stem but different payloads: both must be processed.
        let req_inflight = sample_request("dup", "inflight payload");
        let req_json = sample_request("dup-new", "json payload");
        write_request_file(&queue_dir.join("20260309030008-dup.inflight"), &req_inflight);
        write_request_file(&queue_dir.join("20260309030008-dup.json"), &req_json);

        let drained = drain_amendment_queue(project_dir).expect("drain queue");
        let ids: Vec<&str> = drained.iter().map(|item| item.id.as_str()).collect();
        assert_eq!(ids.len(), 2, "both distinct payloads must be drained");
        assert!(ids.contains(&"dup"), "inflight payload must be present");
        assert!(ids.contains(&"dup-new"), "json payload must be present");
        assert_eq!(
            pending_amendment_count(project_dir).expect("pending count"),
            0
        );
    }

    #[test]
    fn drain_same_stem_distinct_content_claims_json_to_inflight_before_parse() {
        let temp = tempdir().expect("tempdir");
        let project_dir = temp.path();
        let queue_dir = amendment_queue_dir(project_dir);
        fs::create_dir_all(&queue_dir).expect("create queue");

        let req_inflight = sample_request("dup", "inflight payload");
        let req_json = sample_request("dup-new", "json payload");
        write_request_file(&queue_dir.join("20260309030008-dup.inflight"), &req_inflight);
        write_request_file(&queue_dir.join("20260309030008-dup.json"), &req_json);

        // Track that the before_json_claim hook fires for the same-stem .json
        // and that the claimed path is a .inflight extension.
        let mut same_stem_claim_observed = false;
        let json_path = queue_dir.join("20260309030008-dup.json");
        let expected_inflight = queue_dir.join("20260309030008-dup.inflight");

        let drained = drain_amendment_queue_with_hook(project_dir, |queued, claimed| {
            if queued == json_path.as_path() {
                assert_eq!(
                    claimed,
                    expected_inflight.as_path(),
                    "same-stem .json must be claimed to .inflight"
                );
                same_stem_claim_observed = true;
            }
            Ok(())
        })
        .expect("drain queue");

        assert!(
            same_stem_claim_observed,
            "before_json_claim hook must fire for same-stem .json"
        );
        let ids: Vec<&str> = drained.iter().map(|item| item.id.as_str()).collect();
        assert_eq!(ids.len(), 2, "both distinct payloads must be drained");
        assert!(ids.contains(&"dup"), "inflight payload must be present");
        assert!(ids.contains(&"dup-new"), "json payload must be present");
    }

    #[test]
    fn drain_same_stem_json_skipped_on_rename_race() {
        let temp = tempdir().expect("tempdir");
        let project_dir = temp.path();
        let queue_dir = amendment_queue_dir(project_dir);
        fs::create_dir_all(&queue_dir).expect("create queue");

        let req_inflight = sample_request("dup", "inflight payload");
        let req_json = sample_request("dup-new", "json payload");
        write_request_file(&queue_dir.join("20260309030008-dup.inflight"), &req_inflight);
        write_request_file(&queue_dir.join("20260309030008-dup.json"), &req_json);

        // Simulate race: remove the .json before the claim rename happens.
        let json_path = queue_dir.join("20260309030008-dup.json");
        let drained = drain_amendment_queue_with_hook(project_dir, |queued, _claimed| {
            if queued == json_path.as_path() {
                // Another process already claimed this file.
                fs::remove_file(queued)?;
            }
            Ok(())
        })
        .expect("drain queue");

        // Only the inflight item should be drained; the raced .json is skipped.
        let ids: Vec<&str> = drained.iter().map(|item| item.id.as_str()).collect();
        assert_eq!(ids, vec!["dup"], "raced .json must be skipped");
    }

    #[test]
    fn enqueue_skips_stem_occupied_by_inflight() {
        let temp = tempdir().expect("tempdir");
        let project_dir = temp.path();
        let queue_dir = amendment_queue_dir(project_dir);
        fs::create_dir_all(&queue_dir).expect("create queue dir");

        // Pre-create an .inflight file with the stem that enqueue would choose.
        let inflight_path = queue_dir.join("20260309030010-EXT-1.inflight");
        fs::write(&inflight_path, "{}").expect("write inflight");

        let req = sample_request("EXT-1", "Body");
        let path = enqueue_amendment_with_timestamp(project_dir, &req, "20260309030010")
            .expect("enqueue should succeed with suffix");
        let file_name = path
            .file_name()
            .and_then(OsStr::to_str)
            .expect("file name should be utf-8");
        assert_eq!(
            file_name, "20260309030010-EXT-1-1.json",
            "enqueue must skip stem occupied by .inflight"
        );
        assert!(
            inflight_path.exists(),
            ".inflight file must not be disturbed"
        );
    }

    #[test]
    fn invalid_utf8_file_is_quarantined_not_fatal() {
        let temp = tempdir().expect("tempdir");
        let project_dir = temp.path();
        let queue_dir = amendment_queue_dir(project_dir);
        fs::create_dir_all(&queue_dir).expect("create queue");

        // Write bytes that are not valid UTF-8.
        let bad_bytes: &[u8] = &[0xFF, 0xFE, 0x80, 0x81];
        fs::write(queue_dir.join("20260309030001-bad-utf8.json"), bad_bytes)
            .expect("write invalid utf-8");
        write_request_file(
            &queue_dir.join("20260309030002-good.json"),
            &sample_request("good", "valid payload"),
        );

        let drained = drain_amendment_queue(project_dir).expect("drain should succeed");
        assert_eq!(drained.len(), 1, "valid item must still be drained");
        assert_eq!(drained[0].id, "good");

        let quarantine_dir = queue_dir.join(QUARANTINE_DIR_NAME);
        assert!(
            quarantine_dir.exists(),
            "quarantine directory must be created for invalid utf-8"
        );
        let quarantine_entries: Vec<_> = fs::read_dir(&quarantine_dir)
            .expect("read quarantine")
            .filter_map(|entry| entry.ok())
            .collect();
        assert_eq!(
            quarantine_entries.len(),
            1,
            "invalid utf-8 file must be quarantined"
        );
    }

    #[test]
    fn drain_does_not_overwrite_inflight_created_before_json_claim() {
        let temp = tempdir().expect("tempdir");
        let project_dir = temp.path();
        let queue_dir = amendment_queue_dir(project_dir);
        fs::create_dir_all(&queue_dir).expect("create queue");

        let json_path = queue_dir.join("20260309030007-item.json");
        let inflight_path = queue_dir.join("20260309030007-item.inflight");
        write_request_file(&json_path, &sample_request("json-item", "json payload"));

        let mut injected = false;
        let drained = drain_amendment_queue_with_hook(project_dir, |queued, claimed| {
            if !injected {
                assert_eq!(queued, json_path.as_path());
                assert_eq!(claimed, inflight_path.as_path());
                write_request_file(
                    claimed,
                    &sample_request("inflight-item", "inflight payload"),
                );
                injected = true;
            }
            Ok(())
        })
        .expect("drain queue");

        assert!(
            drained.is_empty(),
            "json file should not be claimed when inflight appears first"
        );

        let existing_inflight_payload =
            fs::read_to_string(&inflight_path).expect("read preserved inflight file");
        let existing_inflight: AmendmentRequest = serde_json::from_str(&existing_inflight_payload)
            .expect("deserialize preserved inflight");
        assert_eq!(existing_inflight.id, "inflight-item");

        let preserved_json_payload =
            fs::read_to_string(&json_path).expect("read preserved json file");
        let preserved_json: AmendmentRequest =
            serde_json::from_str(&preserved_json_payload).expect("deserialize preserved json");
        assert_eq!(preserved_json.id, "json-item");
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
        assert_eq!(
            pending_amendment_count(project_dir).expect("pending count"),
            0
        );

        let quarantine_dir = queue_dir.join(QUARANTINE_DIR_NAME);
        assert!(
            quarantine_dir.exists(),
            "quarantine directory should be created"
        );
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
    fn temp_staging_files_are_ignored_by_drain_and_pending_count() {
        let temp = tempdir().expect("tempdir");
        let project_dir = temp.path();
        let queue_dir = amendment_queue_dir(project_dir);
        fs::create_dir_all(&queue_dir).expect("create queue");

        let tmp_path = queue_dir.join(".tmp-1234.json");
        fs::write(&tmp_path, "{ not valid json").expect("write temp staging file");

        assert_eq!(
            pending_amendment_count(project_dir).expect("pending count should ignore temp file"),
            0
        );
        let drained = drain_amendment_queue(project_dir).expect("drain queue");
        assert!(drained.is_empty(), "temp staging files must not be drained");
        assert!(tmp_path.exists(), "drain should leave temp staging files untouched");
    }

    #[test]
    fn drain_skips_temp_staging_and_processes_published_files() {
        let temp = tempdir().expect("tempdir");
        let project_dir = temp.path();
        let queue_dir = amendment_queue_dir(project_dir);
        fs::create_dir_all(&queue_dir).expect("create queue");

        let tmp_path = queue_dir.join(".tmp-5678.json");
        fs::write(&tmp_path, "{ not valid json").expect("write temp staging file");

        let published_path = queue_dir.join("20260309030009-real.json");
        write_request_file(&published_path, &sample_request("real", "published payload"));

        assert_eq!(
            pending_amendment_count(project_dir)
                .expect("pending count should include only published queue files"),
            1
        );
        let drained = drain_amendment_queue(project_dir).expect("drain queue");
        let ids: Vec<_> = drained.iter().map(|item| item.id.as_str()).collect();
        assert_eq!(ids, vec!["real"]);
        assert!(
            !published_path.exists(),
            "published queue file should be removed after successful drain"
        );
        assert!(
            tmp_path.exists(),
            "temp staging file should be ignored and left untouched"
        );
        assert_eq!(
            pending_amendment_count(project_dir).expect("pending count after drain"),
            0
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

    #[test]
    fn re_enqueue_restores_items_to_queue() {
        let temp = tempdir().expect("tempdir");
        let project_dir = temp.path();

        let reqs = vec![
            sample_request("EXT-1", "first body"),
            sample_request("EXT-2", "second body"),
        ];

        let failed = re_enqueue_amendments(project_dir, &reqs);
        assert!(failed.is_empty(), "all items should be re-enqueued");

        let drained = drain_amendment_queue(project_dir).expect("drain");
        assert_eq!(drained.len(), 2);
        let ids: Vec<_> = drained.iter().map(|r| r.id.as_str()).collect();
        assert!(ids.contains(&"EXT-1"));
        assert!(ids.contains(&"EXT-2"));
    }

    #[test]
    fn re_enqueue_preserves_original_fields() {
        let temp = tempdir().expect("tempdir");
        let project_dir = temp.path();

        let req = AmendmentRequest {
            id: "EXT-ORIG".to_owned(),
            body: "original body".to_owned(),
            priority: AmendmentPriority::P1,
            source: AmendmentSource::FinalReview,
            source_detail: Some("claude(opus)".to_owned()),
            created_at: Utc
                .with_ymd_and_hms(2026, 3, 9, 12, 0, 0)
                .single()
                .expect("valid datetime"),
        };

        let failed = re_enqueue_amendments(project_dir, &[req.clone()]);
        assert!(failed.is_empty());

        let drained = drain_amendment_queue(project_dir).expect("drain");
        assert_eq!(drained.len(), 1);
        assert_eq!(drained[0].id, req.id);
        assert_eq!(drained[0].body, req.body);
        assert_eq!(drained[0].priority, req.priority);
        assert_eq!(drained[0].source, req.source);
        assert_eq!(drained[0].source_detail, req.source_detail);
        assert_eq!(drained[0].created_at, req.created_at);
    }

    #[test]
    fn drain_rolls_back_already_drained_items_on_mid_drain_fatal_error() {
        let temp = tempdir().expect("tempdir");
        let project_dir = temp.path();
        let queue_dir = amendment_queue_dir(project_dir);
        fs::create_dir_all(&queue_dir).expect("create queue");

        // Enqueue 3 items in deterministic lexicographic order.
        let req_a = sample_request("a", "body a");
        let req_b = sample_request("b", "body b");
        let req_c = sample_request("c", "body c");
        write_request_file(&queue_dir.join("20260309030001-a.json"), &req_a);
        write_request_file(&queue_dir.join("20260309030002-b.json"), &req_b);
        write_request_file(&queue_dir.join("20260309030003-c.json"), &req_c);

        // The before_json_claim hook fires once per .json file before the
        // claim rename.  Items a and b are fully drained (parsed + deleted)
        // before the hook fires for item c.  Injecting a fatal error here
        // exercises the mid-drain rollback path.
        let mut call_count = 0u32;
        let result = drain_amendment_queue_with_hook(project_dir, |_, _| {
            call_count += 1;
            if call_count == 3 {
                Err(RalphError::Io(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    "injected mid-drain failure",
                )))
            } else {
                Ok(())
            }
        });

        // Drain must return an error.
        assert!(result.is_err(), "drain should fail on injected error");

        // All 3 amendments must be recoverable: a and b were rolled back,
        // c was never claimed from its .json file.
        let pending = pending_amendment_count(project_dir).expect("pending count");
        assert_eq!(
            pending, 3,
            "all amendments must be preserved after mid-drain rollback"
        );

        // A fresh drain must succeed and return all 3 items.
        let drained = drain_amendment_queue(project_dir).expect("retry drain");
        assert_eq!(
            drained.len(),
            3,
            "all amendments should be drainable after rollback"
        );
        let ids: Vec<&str> = drained.iter().map(|r| r.id.as_str()).collect();
        assert!(ids.contains(&"a"));
        assert!(ids.contains(&"b"));
        assert!(ids.contains(&"c"));
    }

    #[test]
    fn drain_rollback_preserves_original_amendment_fields() {
        let temp = tempdir().expect("tempdir");
        let project_dir = temp.path();
        let queue_dir = amendment_queue_dir(project_dir);
        fs::create_dir_all(&queue_dir).expect("create queue");

        let original = AmendmentRequest {
            id: "EXT-ORIG".to_owned(),
            body: "original body".to_owned(),
            priority: AmendmentPriority::P1,
            source: AmendmentSource::FinalReview,
            source_detail: Some("claude(opus)".to_owned()),
            created_at: Utc
                .with_ymd_and_hms(2026, 3, 9, 12, 0, 0)
                .single()
                .expect("valid datetime"),
        };
        write_request_file(&queue_dir.join("20260309030001-EXT-ORIG.json"), &original);
        write_request_file(
            &queue_dir.join("20260309030002-trigger.json"),
            &sample_request("trigger", "trigger body"),
        );

        // Fail on the 2nd item so the 1st is rolled back.
        let mut call_count = 0u32;
        let result = drain_amendment_queue_with_hook(project_dir, |_, _| {
            call_count += 1;
            if call_count == 2 {
                Err(RalphError::Io(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    "injected failure",
                )))
            } else {
                Ok(())
            }
        });
        assert!(result.is_err());

        // Drain again and verify the rolled-back item retains all original fields.
        let drained = drain_amendment_queue(project_dir).expect("drain after rollback");
        let restored = drained
            .iter()
            .find(|r| r.id == "EXT-ORIG")
            .expect("rolled-back item must be present");
        assert_eq!(restored.body, original.body);
        assert_eq!(restored.priority, AmendmentPriority::P1);
        assert_eq!(restored.source, AmendmentSource::FinalReview);
        assert_eq!(restored.source_detail, Some("claude(opus)".to_owned()));
        assert_eq!(restored.created_at, original.created_at);
    }

    #[test]
    fn drain_malformed_files_do_not_trigger_rollback() {
        let temp = tempdir().expect("tempdir");
        let project_dir = temp.path();
        let queue_dir = amendment_queue_dir(project_dir);
        fs::create_dir_all(&queue_dir).expect("create queue");

        // A valid item first, then a malformed item, then another valid item.
        write_request_file(
            &queue_dir.join("20260309030001-good1.json"),
            &sample_request("good1", "body 1"),
        );
        fs::write(
            queue_dir.join("20260309030002-bad.json"),
            "{ not valid json",
        )
        .expect("write malformed");
        write_request_file(
            &queue_dir.join("20260309030003-good2.json"),
            &sample_request("good2", "body 2"),
        );

        // Drain should succeed: malformed item is quarantined, valid items are drained.
        let drained = drain_amendment_queue(project_dir).expect("drain should succeed");
        assert_eq!(drained.len(), 2);
        let ids: Vec<&str> = drained.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids, vec!["good1", "good2"]);

        // No items re-enqueued; malformed file went to quarantine.
        assert_eq!(
            pending_amendment_count(project_dir).expect("pending count"),
            0
        );
    }

    #[cfg(unix)]
    #[test]
    fn drain_io_error_on_inflight_read_is_fatal_with_rollback() {
        use std::os::unix::fs::PermissionsExt;

        let temp = tempdir().expect("tempdir");
        let project_dir = temp.path();
        let queue_dir = amendment_queue_dir(project_dir);
        fs::create_dir_all(&queue_dir).expect("create queue");

        // First item: valid, will be drained before the fatal error.
        write_request_file(
            &queue_dir.join("20260309030001-good.json"),
            &sample_request("good", "body good"),
        );

        // Second item: pre-existing .inflight with no read permission.
        let unreadable = queue_dir.join("20260309030002-unreadable.inflight");
        write_request_file(&unreadable, &sample_request("unreadable", "body"));
        fs::set_permissions(&unreadable, fs::Permissions::from_mode(0o000))
            .expect("remove read permissions");

        // Skip if running as root (root bypasses file permissions).
        if fs::read_to_string(&unreadable).is_ok() {
            let _ = fs::set_permissions(&unreadable, fs::Permissions::from_mode(0o644));
            return;
        }

        let result = drain_amendment_queue(project_dir);

        // Restore permissions before assertions so tempdir cleanup always works.
        let _ = fs::set_permissions(&unreadable, fs::Permissions::from_mode(0o644));

        assert!(result.is_err(), "read I/O error must be fatal");

        // The first item was drained then rolled back; the second is still inflight.
        let pending = pending_amendment_count(project_dir).expect("pending count");
        assert_eq!(
            pending, 2,
            "rolled-back item and unreadable inflight should both be pending"
        );
    }

    #[test]
    fn drain_validation_failure_is_quarantined_not_fatal() {
        let temp = tempdir().expect("tempdir");
        let project_dir = temp.path();
        let queue_dir = amendment_queue_dir(project_dir);
        fs::create_dir_all(&queue_dir).expect("create queue");

        // Valid JSON that fails validation (empty id).
        let invalid_json =
            r#"{"id":"","body":"x","source":"cli","created_at":"2026-03-09T03:00:00Z"}"#;
        fs::write(queue_dir.join("20260309030001-bad.json"), invalid_json)
            .expect("write invalid amendment");
        write_request_file(
            &queue_dir.join("20260309030002-good.json"),
            &sample_request("good", "body good"),
        );

        let drained = drain_amendment_queue(project_dir).expect("drain should succeed");
        assert_eq!(drained.len(), 1);
        assert_eq!(drained[0].id, "good");

        let quarantine_dir = queue_dir.join(QUARANTINE_DIR_NAME);
        assert!(
            quarantine_dir.exists(),
            "quarantine directory should exist for validation failure"
        );
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
