use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use chrono::Utc;
use tracing::warn;

/// Derives a deterministic, collision-safe log file path under `.ralph/tmp/logs`.
///
/// Filenames encode project, loop, and role to prevent collisions:
/// - With loop: `{log_dir}/{project_id}-{loop_number:03}-{role}.log`
/// - Without loop: `{log_dir}/{project_id}-{role}.log`
pub fn log_path_for_role(
    log_dir: &Path,
    project_id: &str,
    loop_number: Option<u32>,
    role: &str,
) -> PathBuf {
    let filename = match loop_number {
        Some(n) => format!("{project_id}-{n:03}-{role}.log"),
        None => format!("{project_id}-{role}.log"),
    };
    log_dir.join(filename)
}

pub fn ensure_log_parent(path: &Path) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)?;
        }
    }
    Ok(())
}

pub fn sanitize_for_filename(label: &str) -> String {
    let mut sanitized = String::with_capacity(label.len());
    let mut last_was_underscore = false;

    for ch in label.chars() {
        if ch.is_ascii_alphanumeric() || ch == '-' {
            sanitized.push(ch);
            last_was_underscore = false;
        } else if !last_was_underscore {
            sanitized.push('_');
            last_was_underscore = true;
        }
    }

    sanitized.trim_matches('_').to_owned()
}

/// Formats a separator line for a backend attempt in the log file.
pub fn format_attempt_separator(
    attempt: u32,
    backend_label: &str,
    is_fallback: bool,
    timestamp: &str,
) -> String {
    let sanitized = sanitize_for_filename(backend_label);
    let fallback_flag = if is_fallback {
        "fallback=true"
    } else {
        "fallback=false"
    };
    format!("\n--- attempt={attempt} backend={sanitized} {fallback_flag} ts={timestamp} ---\n")
}

/// Formats a timeout footer line appended when backend execution times out.
pub fn format_timeout_footer(timestamp: &str) -> String {
    format!("\n--- timeout ts={timestamp} ---\n")
}

/// Best-effort append-mode log writer.
///
/// Opens the file in create+append mode. On any I/O failure (open, write, flush),
/// logs a `tracing::warn!` and disables further writes for this writer instance.
/// Failures never propagate to callers and do not affect backend/orchestrator
/// result semantics.
pub struct LogWriter {
    file: Option<File>,
    path: PathBuf,
    attempt: u32,
}

impl LogWriter {
    /// Open a log file for the given role in create+append mode.
    /// Returns a writer that is always usable — if the open fails, the writer
    /// is in a disabled state and all subsequent writes are silently skipped
    /// after a single warning.
    ///
    /// `log_dir` should point to `.ralph/tmp/logs`.
    pub fn open(
        log_dir: &Path,
        project_id: &str,
        loop_number: Option<u32>,
        role: &str,
    ) -> Self {
        let path = log_path_for_role(log_dir, project_id, loop_number, role);
        let file = match Self::try_open(&path) {
            Ok(f) => Some(f),
            Err(e) => {
                warn!(
                    path = %path.display(),
                    error = %e,
                    "failed to open log file; logging disabled for this run"
                );
                None
            }
        };
        Self {
            file,
            path,
            attempt: 0,
        }
    }

    fn try_open(path: &Path) -> io::Result<File> {
        ensure_log_parent(path)?;
        OpenOptions::new().create(true).append(true).open(path)
    }

    /// Write an attempt separator before a backend execution.
    /// Increments the internal attempt counter.
    pub fn write_attempt_separator(&mut self, backend_label: &str, is_fallback: bool) {
        self.attempt += 1;
        let timestamp = Utc::now().to_rfc3339();
        let separator =
            format_attempt_separator(self.attempt, backend_label, is_fallback, &timestamp);
        self.write_bytes(separator.as_bytes());
    }

    /// Append a timeout footer line with the provided timestamp.
    pub fn write_timeout_footer(&mut self, timestamp: &str) {
        let footer = format_timeout_footer(timestamp);
        self.write_bytes(footer.as_bytes());
    }

    /// Append raw bytes to the log file.
    pub fn write_bytes(&mut self, data: &[u8]) {
        let Some(file) = self.file.as_mut() else {
            return;
        };
        if let Err(e) = file.write_all(data).and_then(|_| file.flush()) {
            warn!(
                path = %self.path.display(),
                error = %e,
                "log write/flush failed; disabling further writes"
            );
            self.file = None;
        }
    }

    /// Append a string to the log file.
    pub fn write_str(&mut self, s: &str) {
        self.write_bytes(s.as_bytes());
    }

    /// Returns the current attempt number.
    pub fn attempt(&self) -> u32 {
        self.attempt
    }

    /// Returns the log file path.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Returns true if the writer is still active (not disabled by error).
    pub fn is_active(&self) -> bool {
        self.file.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use tempfile::tempdir;

    #[test]
    fn derives_loop_scoped_log_path() {
        let log_dir = Path::new("/tmp/logs");
        let path = log_path_for_role(log_dir, "issue-42", Some(7), "implementer");
        assert_eq!(
            path,
            Path::new("/tmp/logs/issue-42-007-implementer.log")
        );
    }

    #[test]
    fn derives_prompt_reviewer_log_path_when_loop_is_none() {
        let log_dir = Path::new("/tmp/logs");
        let path = log_path_for_role(log_dir, "issue-42", None, "prompt-reviewer");
        assert_eq!(
            path,
            Path::new("/tmp/logs/issue-42-prompt-reviewer.log")
        );
    }

    #[test]
    fn formats_loop_number_edges_with_three_digits() {
        let log_dir = Path::new("/tmp/logs");
        let loop_zero = log_path_for_role(log_dir, "issue-1", Some(0), "planner");
        let loop_max = log_path_for_role(log_dir, "issue-1", Some(999), "planner");

        assert_eq!(
            loop_zero,
            Path::new("/tmp/logs/issue-1-000-planner.log")
        );
        assert_eq!(
            loop_max,
            Path::new("/tmp/logs/issue-1-999-planner.log")
        );
    }

    #[test]
    fn collision_safe_across_projects() {
        let log_dir = Path::new("/tmp/logs");
        let path_a = log_path_for_role(log_dir, "issue-1", Some(1), "planner");
        let path_b = log_path_for_role(log_dir, "issue-2", Some(1), "planner");
        assert_ne!(path_a, path_b, "different projects should produce different paths");
    }

    #[test]
    fn creates_missing_parent_directories() {
        let temp = tempdir().expect("tempdir");
        let log_path = temp.path().join("tmp/logs/issue-1-004-reviewer.log");

        ensure_log_parent(&log_path).expect("parent directories should be created");

        assert!(
            log_path
                .parent()
                .expect("parent")
                .try_exists()
                .expect("exists check"),
            "parent path should exist after ensure_log_parent"
        );
    }

    #[test]
    fn sanitizes_unsafe_filename_characters() {
        assert_eq!(sanitize_for_filename("../"), "");
        assert_eq!(sanitize_for_filename("; rm -rf"), "rm_-rf");
        assert_eq!(
            sanitize_for_filename("backend label with spaces"),
            "backend_label_with_spaces"
        );
        assert_eq!(sanitize_for_filename("日本語"), "");
    }

    #[test]
    fn sanitization_collapses_and_trims_underscores() {
        assert_eq!(sanitize_for_filename("___alpha___beta___"), "alpha_beta");
        assert_eq!(sanitize_for_filename("a////b"), "a_b");
    }

    #[test]
    fn sanitization_handles_empty_input() {
        assert_eq!(sanitize_for_filename(""), "");
    }

    #[test]
    fn separator_format_contains_required_fields() {
        let sep = format_attempt_separator(1, "claude(opus)", false, "2026-01-01T00:00:00Z");
        assert!(sep.contains("attempt=1"));
        assert!(sep.contains("backend=claude_opus"));
        assert!(sep.contains("fallback=false"));
        assert!(sep.contains("ts=2026-01-01T00:00:00Z"));
        assert!(sep.starts_with('\n'));
        assert!(sep.ends_with('\n'));
    }

    #[test]
    fn separator_format_fallback_flag() {
        let sep = format_attempt_separator(2, "codex(gpt-5)", true, "2026-01-01T00:00:00Z");
        assert!(sep.contains("attempt=2"));
        assert!(sep.contains("fallback=true"));
    }

    #[test]
    fn timeout_footer_format_contains_timestamp() {
        let footer = format_timeout_footer("2026-01-01T00:00:00Z");
        assert_eq!(footer, "\n--- timeout ts=2026-01-01T00:00:00Z ---\n");
    }

    #[test]
    fn log_writer_opens_and_appends() {
        let temp = tempdir().expect("tempdir");
        let log_dir = temp.path();

        let mut writer = LogWriter::open(log_dir, "issue-1", Some(1), "planner");
        assert!(writer.is_active());

        writer.write_attempt_separator("claude(opus)", false);
        writer.write_str("hello world");
        writer.write_attempt_separator("codex(gpt-5)", true);
        writer.write_str("retry output");

        let content = fs::read_to_string(writer.path()).expect("read log");
        assert!(content.contains("attempt=1"));
        assert!(content.contains("hello world"));
        assert!(content.contains("attempt=2"));
        assert!(content.contains("fallback=true"));
        assert!(content.contains("retry output"));
    }

    #[test]
    fn log_writer_preserves_cr_and_partial_line_bytes() {
        let temp = tempdir().expect("tempdir");
        let mut writer = LogWriter::open(temp.path(), "issue-9", Some(9), "planner");

        writer.write_bytes(b"progress 10%\r");
        writer.write_bytes(b"progress 20%\r");
        writer.write_bytes(b"partial-line");

        let bytes = fs::read(writer.path()).expect("read log bytes");
        assert_eq!(bytes, b"progress 10%\rprogress 20%\rpartial-line");
    }

    #[test]
    fn log_writer_timeout_footer_appends() {
        let temp = tempdir().expect("tempdir");
        let mut writer = LogWriter::open(temp.path(), "issue-3", Some(3), "implementer");

        writer.write_bytes(b"partial output");
        writer.write_timeout_footer("2026-01-01T00:00:00Z");

        let content = fs::read_to_string(writer.path()).expect("read log");
        assert!(content.contains("partial output"));
        assert!(content.contains("--- timeout ts=2026-01-01T00:00:00Z ---"));
    }

    #[test]
    fn log_writer_appends_across_instances() {
        let temp = tempdir().expect("tempdir");
        let log_dir = temp.path();

        {
            let mut w = LogWriter::open(log_dir, "issue-1", Some(1), "implementer");
            w.write_attempt_separator("claude", false);
            w.write_str("first run\n");
        }
        {
            let mut w = LogWriter::open(log_dir, "issue-1", Some(1), "implementer");
            w.write_attempt_separator("codex", false);
            w.write_str("second run\n");
        }

        let content =
            fs::read_to_string(log_path_for_role(log_dir, "issue-1", Some(1), "implementer"))
                .expect("read log");
        assert!(content.contains("first run"));
        assert!(content.contains("second run"));
    }

    #[test]
    fn log_writer_attempt_counter_increments() {
        let temp = tempdir().expect("tempdir");
        let mut writer = LogWriter::open(temp.path(), "issue-1", None, "prompt-reviewer");

        assert_eq!(writer.attempt(), 0);
        writer.write_attempt_separator("backend-a", false);
        assert_eq!(writer.attempt(), 1);
        writer.write_attempt_separator("backend-b", true);
        assert_eq!(writer.attempt(), 2);
    }

    #[test]
    fn log_writer_disabled_on_bad_path_continues_silently() {
        // Open a writer against a path that can't be opened (device file as dir).
        // /dev/null is not a directory, so creating a file under it will fail.
        let writer = LogWriter::open(Path::new("/dev/null/nonexistent"), "issue-1", Some(1), "planner");
        assert!(!writer.is_active());
    }

    #[test]
    fn log_writer_prompt_reviewer_uses_flat_path() {
        let temp = tempdir().expect("tempdir");
        let writer = LogWriter::open(temp.path(), "issue-42", None, "prompt-reviewer");
        assert_eq!(
            writer.path(),
            temp.path().join("issue-42-prompt-reviewer.log")
        );
    }

    /// Simulate the attempt numbering pattern from `execute_with_timeout_retries`:
    /// each call to `write_attempt_separator` in the timeout-retry loop increments
    /// the attempt counter and writes a separator. After 3 timeout retries the
    /// attempt count should be 3 and the log should contain separators for all 3.
    #[test]
    fn timeout_retry_path_attempt_numbering() {
        let temp = tempdir().expect("tempdir");
        let mut writer = LogWriter::open(temp.path(), "issue-1", Some(1), "planner");

        // Simulate 3 timeout retries as in execute_with_timeout_retries:
        // for attempt in 1..=3 { is_fallback = writer.attempt() > 0; write_separator; }
        for _ in 1..=3_u8 {
            let is_fallback = writer.attempt() > 0;
            writer.write_attempt_separator("claude(opus)", is_fallback);
            // Simulate backend returning a timeout (no output written)
        }

        assert_eq!(writer.attempt(), 3);

        let content = fs::read_to_string(writer.path()).expect("read log");
        assert!(content.contains("attempt=1"));
        assert!(content.contains("attempt=2"));
        assert!(content.contains("attempt=3"));

        // First attempt: fallback=false (attempt was 0 before)
        // Subsequent attempts: fallback=true (attempt > 0)
        let lines: Vec<&str> = content.lines().collect();
        let sep1 = lines.iter().find(|l| l.contains("attempt=1")).unwrap();
        let sep2 = lines.iter().find(|l| l.contains("attempt=2")).unwrap();
        let sep3 = lines.iter().find(|l| l.contains("attempt=3")).unwrap();
        assert!(
            sep1.contains("fallback=false"),
            "first attempt should be fallback=false"
        );
        assert!(
            sep2.contains("fallback=true"),
            "second attempt should be fallback=true"
        );
        assert!(
            sep3.contains("fallback=true"),
            "third attempt should be fallback=true"
        );
    }

    /// Simulate the attempt numbering pattern from `execute_with_parse_retries`:
    /// 1. First `execute_with_timeout_retries` call (attempt=1, success)
    /// 2. Parse fails → reformatter call via `execute_with_timeout_retries` (attempt=2)
    /// 3. Reformatter fails → format-reminder call (attempt=3)
    ///    All go through the same LogWriter, so attempt numbers are continuous.
    #[test]
    fn parse_retry_path_attempt_numbering() {
        let temp = tempdir().expect("tempdir");
        let mut writer = LogWriter::open(temp.path(), "issue-1", Some(1), "planner");

        // Step 1: First backend call succeeds (execute_with_timeout_retries, 1 attempt)
        let is_fallback = writer.attempt() > 0;
        writer.write_attempt_separator("claude(opus)", is_fallback);
        writer.write_str("unparseable output from first attempt");

        // Step 2: Parse fails → reformatter backend (execute_with_timeout_retries, 1 attempt)
        let is_fallback = writer.attempt() > 0;
        writer.write_attempt_separator("codex(gpt-5)", is_fallback);
        writer.write_str("reformatter output attempt");

        // Step 3: Reformatter parse also fails → format-reminder with original backend
        let is_fallback = writer.attempt() > 0;
        writer.write_attempt_separator("claude(opus)", is_fallback);
        writer.write_str("format-reminder output");

        assert_eq!(writer.attempt(), 3);

        let content = fs::read_to_string(writer.path()).expect("read log");

        // All 3 attempts in the same file
        assert!(content.contains("attempt=1"));
        assert!(content.contains("attempt=2"));
        assert!(content.contains("attempt=3"));

        // First attempt is not fallback, subsequent are
        let lines: Vec<&str> = content.lines().collect();
        let sep1 = lines.iter().find(|l| l.contains("attempt=1")).unwrap();
        let sep2 = lines.iter().find(|l| l.contains("attempt=2")).unwrap();
        let sep3 = lines.iter().find(|l| l.contains("attempt=3")).unwrap();
        assert!(sep1.contains("fallback=false"));
        assert!(sep2.contains("fallback=true"));
        assert!(sep3.contains("fallback=true"));

        // Verify different backend labels are attributed correctly
        assert!(sep1.contains("backend=claude_opus"));
        assert!(sep2.contains("backend=codex_gpt-5"));
        assert!(sep3.contains("backend=claude_opus"));

        // All output content is appended
        assert!(content.contains("unparseable output from first attempt"));
        assert!(content.contains("reformatter output attempt"));
        assert!(content.contains("format-reminder output"));
    }

    /// Simulate mixed timeout + parse retry: timeout on first attempt in
    /// `execute_with_timeout_retries`, success on second, then parse failure
    /// leading to a reformatter call. Total: 3 attempts across both retry paths.
    #[test]
    fn mixed_timeout_and_parse_retry_numbering() {
        let temp = tempdir().expect("tempdir");
        let mut writer = LogWriter::open(temp.path(), "issue-2", Some(2), "implementer");

        // execute_with_timeout_retries: attempt 1 times out
        let is_fallback = writer.attempt() > 0;
        writer.write_attempt_separator("claude(opus)", is_fallback);
        // (no output — timed out)

        // execute_with_timeout_retries: attempt 2 succeeds
        let is_fallback = writer.attempt() > 0;
        writer.write_attempt_separator("claude(opus)", is_fallback);
        writer.write_str("output from timeout retry");

        // Parse fails on the output → reformatter call (new execute_with_timeout_retries)
        let is_fallback = writer.attempt() > 0;
        writer.write_attempt_separator("codex(gpt-5)", is_fallback);
        writer.write_str("reformatter fixed output");

        assert_eq!(writer.attempt(), 3);

        let content = fs::read_to_string(writer.path()).expect("read log");
        assert!(content.contains("attempt=1"));
        assert!(content.contains("attempt=2"));
        assert!(content.contains("attempt=3"));

        // First is not fallback, rest are
        let lines: Vec<&str> = content.lines().collect();
        let sep1 = lines.iter().find(|l| l.contains("attempt=1")).unwrap();
        assert!(sep1.contains("fallback=false"));
        let sep2 = lines.iter().find(|l| l.contains("attempt=2")).unwrap();
        assert!(sep2.contains("fallback=true"));
        let sep3 = lines.iter().find(|l| l.contains("attempt=3")).unwrap();
        assert!(sep3.contains("fallback=true"));
    }

    /// Verify that `fallback` semantics are: `is_fallback = writer.attempt() > 0`.
    /// The first call to `write_attempt_separator` always has `attempt() == 0`
    /// before incrementing, so `is_fallback` is false. All subsequent calls
    /// have `attempt() > 0`, so `is_fallback` is true.
    #[test]
    fn fallback_flag_semantics_locked_down() {
        let temp = tempdir().expect("tempdir");
        let mut writer = LogWriter::open(temp.path(), "issue-1", Some(1), "reviewer");

        // Before any writes, attempt is 0
        assert_eq!(writer.attempt(), 0);

        // Attempt 1: is_fallback = (0 > 0) = false
        let fb1 = writer.attempt() > 0;
        assert!(!fb1, "first attempt should not be fallback");
        writer.write_attempt_separator("backend-a", fb1);
        assert_eq!(writer.attempt(), 1);

        // Attempt 2: is_fallback = (1 > 0) = true
        let fb2 = writer.attempt() > 0;
        assert!(fb2, "second attempt should be fallback");
        writer.write_attempt_separator("backend-b", fb2);
        assert_eq!(writer.attempt(), 2);

        // Attempt 3: is_fallback = (2 > 0) = true
        let fb3 = writer.attempt() > 0;
        assert!(fb3, "third attempt should be fallback");
        writer.write_attempt_separator("backend-a", fb3);
        assert_eq!(writer.attempt(), 3);

        let content = fs::read_to_string(writer.path()).expect("read log");
        // Exactly one fallback=false (first attempt)
        assert_eq!(
            content.matches("fallback=false").count(),
            1,
            "exactly one attempt should have fallback=false"
        );
        // Remaining attempts are fallback=true
        assert_eq!(
            content.matches("fallback=true").count(),
            2,
            "subsequent attempts should have fallback=true"
        );
    }
}
