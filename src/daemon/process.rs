use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;
use std::time::{Duration, Instant};

use chrono::Utc;
use nix::errno::Errno;
use nix::sys::signal::{kill, killpg, Signal};
use nix::unistd::Pid;
use tokio::process::Command;

use crate::error::RalphError;
use crate::Result;

/// Environment variables that must be stripped from daemon child processes.
/// `CLAUDECODE` — prevents Claude Code backends from detecting a nested session
/// and refusing to launch.
const SANITIZED_ENV_VARS: &[&str] = &["CLAUDECODE"];

/// Remove environment variables that interfere with child process execution.
fn sanitize_command_env(cmd: &mut Command) {
    for var in SANITIZED_ENV_VARS {
        cmd.env_remove(var);
    }
}

/// Result of spawning a child process.
pub struct SpawnedChild {
    pub pid: u32,
    pub pgid: u32,
    pub child: tokio::process::Child,
}

/// Spawn `ralph auto` in a new session/process group.
///
/// Uses an in-process `libc::setsid()` call via `CommandExt::pre_exec` so the
/// child gets its own session and process group without depending on an
/// external `setsid` binary being available on PATH.
///
/// Child stdout and stderr are redirected to `log_file` so output is preserved
/// after worktree cleanup.
pub async fn spawn_ralph_auto(
    ralph_bin: &Path,
    worktree_path: &Path,
    idea: &str,
    log_file: &Path,
    project_id: Option<&str>,
    pr_url: Option<&str>,
) -> Result<SpawnedChild> {
    let mut cmd =
        build_ralph_auto_command(ralph_bin, worktree_path, idea, log_file, project_id, pr_url)?;

    // SAFETY: setsid() is async-signal-safe and is the standard way to
    // create a new session/process group in the child before exec.
    unsafe {
        cmd.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }

    let child = cmd.spawn().map_err(|err| {
        RalphError::Orchestration(format!(
            "failed to spawn ralph auto in {}: {err}",
            worktree_path.display()
        ))
    })?;

    let pid = child.id().ok_or_else(|| {
        RalphError::Orchestration(format!(
            "spawned ralph auto without PID in {}",
            worktree_path.display()
        ))
    })?;
    // After setsid(), the child's PID is also its PGID (it's the session
    // leader).
    let pgid = pid;

    Ok(SpawnedChild { pid, pgid, child })
}

/// Spawn `ralph run --project <id>` in a new session/process group.
///
/// Child stdout and stderr are redirected to `log_file` so output is preserved
/// after worktree cleanup.
pub async fn spawn_ralph_run(
    ralph_bin: &Path,
    worktree_path: &Path,
    project_id: &str,
    log_file: &Path,
    pr_url: Option<&str>,
) -> Result<SpawnedChild> {
    let mut cmd = build_ralph_run_command(ralph_bin, worktree_path, project_id, log_file, pr_url)?;

    // SAFETY: setsid() is async-signal-safe and is the standard way to
    // create a new session/process group in the child before exec.
    unsafe {
        cmd.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }

    let child = cmd.spawn().map_err(|err| {
        RalphError::Orchestration(format!(
            "failed to spawn ralph run --project {} in {}: {err}",
            project_id,
            worktree_path.display()
        ))
    })?;

    let pid = child.id().ok_or_else(|| {
        RalphError::Orchestration(format!(
            "spawned ralph run --project {} without PID in {}",
            project_id,
            worktree_path.display()
        ))
    })?;
    let pgid = pid;

    Ok(SpawnedChild { pid, pgid, child })
}

fn build_ralph_auto_command(
    ralph_bin: &Path,
    worktree_path: &Path,
    idea: &str,
    log_file: &Path,
    project_id: Option<&str>,
    pr_url: Option<&str>,
) -> Result<Command> {
    let file = open_log_file_append(log_file)?;
    let file_clone = file.try_clone().map_err(|err| {
        RalphError::Orchestration(format!("failed to clone log file handle: {err}"))
    })?;

    let mut cmd = Command::new(ralph_bin);
    sanitize_command_env(&mut cmd);
    cmd.args(["auto", "--idea", idea]);
    cmd.arg("--workspace-root");
    cmd.arg(worktree_path);
    if let Some(project_id) = project_id {
        cmd.args(["--project-id", project_id]);
    }
    if let Some(url) = pr_url {
        cmd.args(["--pr-url", url]);
    }
    cmd.current_dir(worktree_path)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::from(file))
        .stderr(std::process::Stdio::from(file_clone));
    Ok(cmd)
}

fn build_ralph_run_command(
    ralph_bin: &Path,
    worktree_path: &Path,
    project_id: &str,
    log_file: &Path,
    pr_url: Option<&str>,
) -> Result<Command> {
    let file = open_log_file_append(log_file)?;
    let file_clone = file.try_clone().map_err(|err| {
        RalphError::Orchestration(format!("failed to clone log file handle: {err}"))
    })?;

    let mut cmd = Command::new(ralph_bin);
    sanitize_command_env(&mut cmd);
    cmd.args(["run", "--project", project_id, "--until-complete"]);
    cmd.arg("--workspace-root");
    cmd.arg(worktree_path);
    if let Some(url) = pr_url {
        cmd.args(["--pr-url", url]);
    }
    cmd.current_dir(worktree_path)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::from(file))
        .stderr(std::process::Stdio::from(file_clone));
    Ok(cmd)
}

fn open_log_file_append(log_file: &Path) -> Result<std::fs::File> {
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .read(true)
        .append(true)
        .open(log_file)
        .map_err(|err| {
            RalphError::Orchestration(format!(
                "failed to open log file {} for append: {err}",
                log_file.display()
            ))
        })?;

    let (has_content, force_conservative_separator) =
        has_content_for_separator(log_file, file.metadata().map(|meta| meta.len()));

    if has_content {
        let timestamp = Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
        let separator = if force_conservative_separator {
            format_retrigger_separator(&timestamp, None)
        } else {
            let ends_with_newline = file.seek(SeekFrom::End(-1)).and_then(|_| {
                let mut last = [0_u8; 1];
                file.read_exact(&mut last).map(|_| last[0] == b'\n')
            });
            match ends_with_newline {
                Ok(ends_with_newline) => {
                    format_retrigger_separator(&timestamp, Some(ends_with_newline))
                }
                Err(err) => {
                    eprintln!(
                        "warning: failed to inspect trailing newline for log file {}: {err}",
                        log_file.display()
                    );
                    format_retrigger_separator(&timestamp, None)
                }
            }
        };
        if let Err(err) = file.write_all(separator.as_bytes()) {
            eprintln!(
                "warning: failed to write retrigger separator to {}: {err}",
                log_file.display()
            );
        }
    }

    Ok(file)
}

fn has_content_for_separator(log_file: &Path, metadata_len: std::io::Result<u64>) -> (bool, bool) {
    match metadata_len {
        Ok(len) => (len > 0, false),
        Err(err) => {
            eprintln!(
                "warning: failed to inspect log file {} metadata: {err}",
                log_file.display()
            );
            // Conservative fallback: assume existing content and avoid relying on
            // trailing-newline inspection.
            (true, true)
        }
    }
}

fn format_retrigger_separator(timestamp: &str, ends_with_newline: Option<bool>) -> String {
    match ends_with_newline {
        Some(true) => format!("\n--- retrigger at {timestamp} ---\n\n"),
        Some(false) | None => format!("\n\n--- retrigger at {timestamp} ---\n\n"),
    }
}

/// Spawn `ralph quick-dev-auto --idea ...` in a new session/process group.
///
/// Child stdout and stderr are redirected to `log_file` so output is preserved
/// after worktree cleanup.
pub async fn spawn_ralph_quick_dev_auto(
    ralph_bin: &Path,
    worktree_path: &Path,
    idea: &str,
    log_file: &Path,
    project_id: Option<&str>,
    pr_url: Option<&str>,
) -> Result<SpawnedChild> {
    let mut cmd = build_ralph_quick_dev_auto_command(
        ralph_bin,
        worktree_path,
        idea,
        log_file,
        project_id,
        pr_url,
    )?;

    // SAFETY: setsid() is async-signal-safe and is the standard way to
    // create a new session/process group in the child before exec.
    unsafe {
        cmd.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }

    let child = cmd.spawn().map_err(|err| {
        RalphError::Orchestration(format!(
            "failed to spawn ralph quick-dev-auto in {}: {err}",
            worktree_path.display()
        ))
    })?;

    let pid = child.id().ok_or_else(|| {
        RalphError::Orchestration(format!(
            "spawned ralph quick-dev-auto without PID in {}",
            worktree_path.display()
        ))
    })?;
    let pgid = pid;

    Ok(SpawnedChild { pid, pgid, child })
}

/// Spawn `ralph quick-dev-run --project <id>` in a new session/process group.
///
/// Child stdout and stderr are redirected to `log_file` so output is preserved
/// after worktree cleanup.
pub async fn spawn_ralph_quick_dev_run(
    ralph_bin: &Path,
    worktree_path: &Path,
    project_id: &str,
    log_file: &Path,
    pr_url: Option<&str>,
) -> Result<SpawnedChild> {
    let mut cmd =
        build_ralph_quick_dev_run_command(ralph_bin, worktree_path, project_id, log_file, pr_url)?;

    // SAFETY: setsid() is async-signal-safe and is the standard way to
    // create a new session/process group in the child before exec.
    unsafe {
        cmd.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }

    let child = cmd.spawn().map_err(|err| {
        RalphError::Orchestration(format!(
            "failed to spawn ralph quick-dev-run --project {} in {}: {err}",
            project_id,
            worktree_path.display()
        ))
    })?;

    let pid = child.id().ok_or_else(|| {
        RalphError::Orchestration(format!(
            "spawned ralph quick-dev-run --project {} without PID in {}",
            project_id,
            worktree_path.display()
        ))
    })?;
    let pgid = pid;

    Ok(SpawnedChild { pid, pgid, child })
}

fn build_ralph_quick_dev_auto_command(
    ralph_bin: &Path,
    worktree_path: &Path,
    idea: &str,
    log_file: &Path,
    project_id: Option<&str>,
    pr_url: Option<&str>,
) -> Result<Command> {
    let file = open_log_file_append(log_file)?;
    let file_clone = file.try_clone().map_err(|err| {
        RalphError::Orchestration(format!("failed to clone log file handle: {err}"))
    })?;

    let mut cmd = Command::new(ralph_bin);
    sanitize_command_env(&mut cmd);
    cmd.args(["quick-dev-auto", "--idea", idea]);
    cmd.arg("--workspace-root");
    cmd.arg(worktree_path);
    if let Some(project_id) = project_id {
        cmd.args(["--project-id", project_id]);
    }
    if let Some(url) = pr_url {
        cmd.args(["--pr-url", url]);
    }
    cmd.current_dir(worktree_path)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::from(file))
        .stderr(std::process::Stdio::from(file_clone));
    Ok(cmd)
}

fn build_ralph_quick_dev_run_command(
    ralph_bin: &Path,
    worktree_path: &Path,
    project_id: &str,
    log_file: &Path,
    pr_url: Option<&str>,
) -> Result<Command> {
    let file = open_log_file_append(log_file)?;
    let file_clone = file.try_clone().map_err(|err| {
        RalphError::Orchestration(format!("failed to clone log file handle: {err}"))
    })?;

    let mut cmd = Command::new(ralph_bin);
    sanitize_command_env(&mut cmd);
    cmd.args(["quick-dev-run", "--project", project_id]);
    cmd.arg("--workspace-root");
    cmd.arg(worktree_path);
    if let Some(url) = pr_url {
        cmd.args(["--pr-url", url]);
    }
    cmd.current_dir(worktree_path)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::from(file))
        .stderr(std::process::Stdio::from(file_clone));
    Ok(cmd)
}

/// Run a synchronous command with a timeout. Returns `Ok(output)` if the
/// command completes within the deadline, or an error on timeout/failure.
///
/// Stdout and stderr are piped and drained concurrently via reader threads
/// to prevent deadlocks when child output exceeds OS pipe buffer capacity.
pub fn run_command_with_timeout(
    cmd: &mut std::process::Command,
    timeout: Duration,
) -> crate::Result<std::process::Output> {
    use std::os::unix::process::CommandExt;

    let mut child = cmd
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .process_group(0) // spawn in its own process group
        .spawn()
        .map_err(|err| {
            crate::error::RalphError::Orchestration(format!("failed to spawn command: {err}"))
        })?;

    let child_pid = child.id();

    // Take ownership of stdout/stderr handles and drain them in background
    // threads. This prevents the child from blocking on a full pipe buffer.
    let stdout_handle = child.stdout.take();
    let stderr_handle = child.stderr.take();

    let stdout_thread = std::thread::spawn(move || -> Vec<u8> {
        let mut buf = Vec::new();
        if let Some(mut out) = stdout_handle {
            let _ = std::io::Read::read_to_end(&mut out, &mut buf);
        }
        buf
    });

    let stderr_thread = std::thread::spawn(move || -> Vec<u8> {
        let mut buf = Vec::new();
        if let Some(mut err) = stderr_handle {
            let _ = std::io::Read::read_to_end(&mut err, &mut buf);
        }
        buf
    });

    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {
                if Instant::now() >= deadline {
                    // Kill the entire process group so descendants that
                    // inherited pipe FDs are also terminated, allowing
                    // the reader threads to unblock promptly.
                    kill_process_group(child_pid);
                    let _ = child.wait();
                    // Do NOT join reader threads here — if any descendant
                    // somehow survived the group kill, joining would block
                    // indefinitely. The threads will be cleaned up when
                    // their pipe handles close.
                    return Err(crate::error::RalphError::Orchestration(
                        "command timed out".to_owned(),
                    ));
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(err) => {
                kill_process_group(child_pid);
                let _ = child.wait();
                return Err(crate::error::RalphError::Orchestration(format!(
                    "failed to check command status: {err}"
                )));
            }
        }
    };

    let stdout = stdout_thread.join().unwrap_or_default();
    let stderr = stderr_thread.join().unwrap_or_default();

    Ok(std::process::Output {
        status,
        stdout,
        stderr,
    })
}

/// Send SIGKILL to an entire process group identified by the child PID.
/// The child must have been spawned with `process_group(0)` so that its PID
/// equals its PGID.
fn kill_process_group(child_pid: u32) {
    if let Ok(raw) = i32::try_from(child_pid) {
        let pgid = Pid::from_raw(raw);
        let _ = killpg(pgid, Signal::SIGKILL);
    }
}

/// Check if a process with the given PID exists.
pub fn pid_exists(pid: u32) -> bool {
    if pid <= 1 {
        return false;
    }

    let Ok(raw_pid) = i32::try_from(pid) else {
        return false;
    };

    match kill(Pid::from_raw(raw_pid), None) {
        Ok(_) => true,
        Err(Errno::EPERM) => true,
        Err(Errno::ESRCH) => false,
        Err(_) => false,
    }
}

/// Check if a process group with the given PGID exists.
pub fn pgid_exists(pgid: u32) -> bool {
    if pgid <= 1 {
        return false;
    }
    let Ok(raw_pgid) = i32::try_from(pgid) else {
        return false;
    };
    // Sending signal 0 to -pgid checks the entire process group.
    match kill(Pid::from_raw(-raw_pgid), None) {
        Ok(_) => true,
        Err(Errno::EPERM) => true,
        Err(Errno::ESRCH) => false,
        Err(_) => false,
    }
}

/// Terminate a process group gracefully (SIGTERM), escalating to SIGKILL
/// after the given timeout.
pub async fn terminate_process_group(pgid: u32, timeout: Duration) {
    if pgid <= 1 {
        return;
    }

    let Ok(raw_pgid) = i32::try_from(pgid) else {
        return;
    };
    let pgid = Pid::from_raw(raw_pgid);
    let neg_pgid = Pid::from_raw(-raw_pgid);

    // Check if the group exists
    let exists = match kill(neg_pgid, None) {
        Ok(_) => true,
        Err(Errno::EPERM) => true,
        Err(Errno::ESRCH) => false,
        Err(_) => false,
    };
    if !exists {
        return;
    }

    // Send SIGTERM to the process group
    if let Err(err) = killpg(pgid, Signal::SIGTERM) {
        if err == Errno::ESRCH {
            return;
        }
    }

    // Wait for processes to exit
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        let still_exists = match kill(neg_pgid, None) {
            Ok(_) => true,
            Err(Errno::EPERM) => true,
            Err(Errno::ESRCH) => false,
            Err(_) => false,
        };
        if !still_exists {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    // Escalate to SIGKILL
    let _ = killpg(pgid, Signal::SIGKILL);
}

#[cfg(test)]
mod tests {
    use chrono::DateTime;
    use std::ffi::OsStr;
    use std::io;
    use std::path::Path;
    #[cfg(unix)]
    use std::time::{Duration, Instant};

    use super::{
        build_ralph_auto_command, build_ralph_quick_dev_auto_command,
        build_ralph_quick_dev_run_command, build_ralph_run_command, format_retrigger_separator,
        has_content_for_separator,
    };
    #[cfg(unix)]
    use super::{pgid_exists, pid_exists, terminate_process_group};

    #[test]
    fn spawn_command_uses_long_idea_flag() {
        let tmp = tempfile::NamedTempFile::new().expect("create temp file");
        let cmd = build_ralph_auto_command(
            Path::new("/tmp/ralph"),
            Path::new("/tmp/worktree"),
            "implement feature",
            tmp.path(),
            None,
            None,
        )
        .expect("build command");
        let args: Vec<&OsStr> = cmd.as_std().get_args().collect();
        assert_eq!(
            args,
            vec![
                OsStr::new("auto"),
                OsStr::new("--idea"),
                OsStr::new("implement feature"),
                OsStr::new("--workspace-root"),
                OsStr::new("/tmp/worktree"),
            ]
        );
    }

    #[test]
    fn spawn_auto_command_includes_project_id() {
        let tmp = tempfile::NamedTempFile::new().expect("create temp file");
        let cmd = build_ralph_auto_command(
            Path::new("/tmp/ralph"),
            Path::new("/tmp/worktree"),
            "implement feature",
            tmp.path(),
            Some("issue-42"),
            None,
        )
        .expect("build command");
        let args: Vec<&OsStr> = cmd.as_std().get_args().collect();
        assert_eq!(
            args,
            vec![
                OsStr::new("auto"),
                OsStr::new("--idea"),
                OsStr::new("implement feature"),
                OsStr::new("--workspace-root"),
                OsStr::new("/tmp/worktree"),
                OsStr::new("--project-id"),
                OsStr::new("issue-42"),
            ]
        );
    }

    #[test]
    fn spawn_auto_command_includes_pr_url() {
        let tmp = tempfile::NamedTempFile::new().expect("create temp file");
        let cmd = build_ralph_auto_command(
            Path::new("/tmp/ralph"),
            Path::new("/tmp/worktree"),
            "implement feature",
            tmp.path(),
            Some("issue-42"),
            Some("https://github.com/acme/widgets/pull/99"),
        )
        .expect("build command");
        let args: Vec<&OsStr> = cmd.as_std().get_args().collect();
        assert_eq!(
            args,
            vec![
                OsStr::new("auto"),
                OsStr::new("--idea"),
                OsStr::new("implement feature"),
                OsStr::new("--workspace-root"),
                OsStr::new("/tmp/worktree"),
                OsStr::new("--project-id"),
                OsStr::new("issue-42"),
                OsStr::new("--pr-url"),
                OsStr::new("https://github.com/acme/widgets/pull/99"),
            ]
        );
    }

    #[test]
    fn spawn_run_command_uses_project_flag() {
        let tmp = tempfile::NamedTempFile::new().expect("create temp file");
        let cmd = build_ralph_run_command(
            Path::new("/tmp/ralph"),
            Path::new("/tmp/worktree"),
            "retry-project",
            tmp.path(),
            None,
        )
        .expect("build command");
        let args: Vec<&OsStr> = cmd.as_std().get_args().collect();
        assert_eq!(
            args,
            vec![
                OsStr::new("run"),
                OsStr::new("--project"),
                OsStr::new("retry-project"),
                OsStr::new("--until-complete"),
                OsStr::new("--workspace-root"),
                OsStr::new("/tmp/worktree"),
            ]
        );
    }

    #[test]
    fn spawn_run_command_includes_pr_url() {
        let tmp = tempfile::NamedTempFile::new().expect("create temp file");
        let cmd = build_ralph_run_command(
            Path::new("/tmp/ralph"),
            Path::new("/tmp/worktree"),
            "retry-project",
            tmp.path(),
            Some("https://github.com/acme/widgets/pull/7"),
        )
        .expect("build command");
        let args: Vec<&OsStr> = cmd.as_std().get_args().collect();
        assert_eq!(
            args,
            vec![
                OsStr::new("run"),
                OsStr::new("--project"),
                OsStr::new("retry-project"),
                OsStr::new("--until-complete"),
                OsStr::new("--workspace-root"),
                OsStr::new("/tmp/worktree"),
                OsStr::new("--pr-url"),
                OsStr::new("https://github.com/acme/widgets/pull/7"),
            ]
        );
    }

    #[test]
    fn append_mode_writes_retrigger_separator_for_non_empty_log() {
        let tmp = tempfile::tempdir().expect("create temp dir");
        let log_file = tmp.path().join("issue.log");
        std::fs::write(&log_file, "previous output\n").expect("seed existing log");

        let cmd = build_ralph_auto_command(
            Path::new("/tmp/ralph"),
            Path::new("/tmp/worktree"),
            "implement feature",
            &log_file,
            None,
            None,
        )
        .expect("build command");
        drop(cmd);

        let content = std::fs::read_to_string(&log_file).expect("read updated log");
        assert!(
            content.starts_with("previous output\n\n--- retrigger at "),
            "expected separator after existing content, got: {content:?}"
        );
        assert!(
            content.ends_with(" ---\n\n"),
            "separator should end with blank line, got: {content:?}"
        );

        let separator_line = content
            .lines()
            .find(|line| line.starts_with("--- retrigger at "))
            .expect("separator line present");
        let timestamp = separator_line
            .trim_start_matches("--- retrigger at ")
            .trim_end_matches(" ---");
        assert!(
            DateTime::parse_from_rfc3339(timestamp).is_ok(),
            "separator timestamp should be RFC3339 UTC, got: {timestamp}"
        );
    }

    #[test]
    fn append_mode_separator_has_blank_lines_when_no_trailing_newline() {
        let tmp = tempfile::tempdir().expect("create temp dir");
        let log_file = tmp.path().join("issue.log");
        std::fs::write(&log_file, "output without newline").expect("seed existing log");

        let cmd = build_ralph_auto_command(
            Path::new("/tmp/ralph"),
            Path::new("/tmp/worktree"),
            "implement feature",
            &log_file,
            None,
            None,
        )
        .expect("build command");
        drop(cmd);

        let content = std::fs::read_to_string(&log_file).expect("read updated log");
        assert!(
            content.starts_with("output without newline\n\n--- retrigger at "),
            "expected separator to be preceded by blank line, got: {content:?}"
        );
        assert!(
            content.ends_with(" ---\n\n"),
            "separator should end with blank line, got: {content:?}"
        );
    }

    #[test]
    fn metadata_inspection_failure_forces_conservative_separator_path() {
        let (has_content, force_conservative_separator) = has_content_for_separator(
            Path::new("/tmp/issue.log"),
            Err(io::Error::other("metadata probe failed")),
        );
        assert!(has_content);
        assert!(force_conservative_separator);
    }

    #[test]
    fn conservative_separator_format_is_used_on_probe_failure() {
        let separator = format_retrigger_separator("2026-03-04T03:32:00Z", None);
        assert_eq!(
            separator,
            "\n\n--- retrigger at 2026-03-04T03:32:00Z ---\n\n"
        );
    }

    #[cfg(unix)]
    #[test]
    fn test_pid_exists_self() {
        assert!(pid_exists(std::process::id()));
    }

    #[cfg(unix)]
    #[test]
    fn test_pid_exists_bogus() {
        assert!(!pid_exists(u32::MAX - 1));
    }

    #[cfg(unix)]
    #[test]
    fn test_pid_exists_rejects_low_pids() {
        assert!(!pid_exists(0));
        assert!(!pid_exists(1));
    }

    #[cfg(unix)]
    #[test]
    fn test_pgid_exists_current_process() {
        // The current process's PGID should exist.
        let pgid = nix::unistd::getpgrp();
        assert!(pgid_exists(pgid.as_raw() as u32));
    }

    #[cfg(unix)]
    #[test]
    fn test_pgid_exists_dead_group() {
        // A very high PGID almost certainly does not exist.
        assert!(!pgid_exists(u32::MAX - 1));
    }

    #[cfg(unix)]
    #[test]
    fn test_pgid_exists_boundary() {
        // Guard clause should reject 0 and 1.
        assert!(!pgid_exists(0));
        assert!(!pgid_exists(1));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn test_terminate_process_group_noop_for_low_pgid() {
        terminate_process_group(0, Duration::from_millis(50)).await;
        terminate_process_group(1, Duration::from_millis(50)).await;
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn test_terminate_process_group_dead_pgid() {
        let mut child = std::process::Command::new("sleep")
            .arg("30")
            .spawn()
            .expect("spawn child");
        let non_group_id = child.id();

        terminate_process_group(non_group_id, Duration::from_millis(50)).await;
        assert!(
            child.try_wait().expect("poll child").is_none(),
            "child should still be running"
        );

        let _ = child.kill();
        let _ = child.wait();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn test_terminate_spawned_process_group() {
        use std::os::unix::process::{CommandExt, ExitStatusExt};

        let mut child = std::process::Command::new("sleep")
            .arg("30")
            .process_group(0)
            .spawn()
            .expect("spawn process-group leader");
        let pgid = child.id();

        terminate_process_group(pgid, Duration::from_secs(2)).await;

        let deadline = Instant::now() + Duration::from_secs(3);
        let status = loop {
            if let Some(status) = child.try_wait().expect("poll child") {
                break status;
            }
            if Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                panic!("timed out waiting for child to exit");
            }
            std::thread::sleep(Duration::from_millis(50));
        };

        assert_eq!(status.signal(), Some(libc::SIGTERM));
    }

    // --- Quick-dev command builder tests ---

    #[test]
    fn quick_dev_auto_command_basic() {
        let tmp = tempfile::NamedTempFile::new().expect("create temp file");
        let cmd = build_ralph_quick_dev_auto_command(
            Path::new("/tmp/ralph"),
            Path::new("/tmp/worktree"),
            "implement feature",
            tmp.path(),
            None,
            None,
        )
        .expect("build command");
        let args: Vec<&OsStr> = cmd.as_std().get_args().collect();
        assert_eq!(
            args,
            vec![
                OsStr::new("quick-dev-auto"),
                OsStr::new("--idea"),
                OsStr::new("implement feature"),
                OsStr::new("--workspace-root"),
                OsStr::new("/tmp/worktree"),
            ]
        );
    }

    #[test]
    fn quick_dev_auto_command_includes_project_id() {
        let tmp = tempfile::NamedTempFile::new().expect("create temp file");
        let cmd = build_ralph_quick_dev_auto_command(
            Path::new("/tmp/ralph"),
            Path::new("/tmp/worktree"),
            "implement feature",
            tmp.path(),
            Some("issue-42"),
            None,
        )
        .expect("build command");
        let args: Vec<&OsStr> = cmd.as_std().get_args().collect();
        assert_eq!(
            args,
            vec![
                OsStr::new("quick-dev-auto"),
                OsStr::new("--idea"),
                OsStr::new("implement feature"),
                OsStr::new("--workspace-root"),
                OsStr::new("/tmp/worktree"),
                OsStr::new("--project-id"),
                OsStr::new("issue-42"),
            ]
        );
    }

    #[test]
    fn quick_dev_auto_command_includes_pr_url() {
        let tmp = tempfile::NamedTempFile::new().expect("create temp file");
        let cmd = build_ralph_quick_dev_auto_command(
            Path::new("/tmp/ralph"),
            Path::new("/tmp/worktree"),
            "implement feature",
            tmp.path(),
            Some("issue-42"),
            Some("https://github.com/acme/widgets/pull/99"),
        )
        .expect("build command");
        let args: Vec<&OsStr> = cmd.as_std().get_args().collect();
        assert_eq!(
            args,
            vec![
                OsStr::new("quick-dev-auto"),
                OsStr::new("--idea"),
                OsStr::new("implement feature"),
                OsStr::new("--workspace-root"),
                OsStr::new("/tmp/worktree"),
                OsStr::new("--project-id"),
                OsStr::new("issue-42"),
                OsStr::new("--pr-url"),
                OsStr::new("https://github.com/acme/widgets/pull/99"),
            ]
        );
    }

    #[test]
    fn quick_dev_run_command_basic() {
        let tmp = tempfile::NamedTempFile::new().expect("create temp file");
        let cmd = build_ralph_quick_dev_run_command(
            Path::new("/tmp/ralph"),
            Path::new("/tmp/worktree"),
            "issue-7",
            tmp.path(),
            None,
        )
        .expect("build command");
        let args: Vec<&OsStr> = cmd.as_std().get_args().collect();
        assert_eq!(
            args,
            vec![
                OsStr::new("quick-dev-run"),
                OsStr::new("--project"),
                OsStr::new("issue-7"),
                OsStr::new("--workspace-root"),
                OsStr::new("/tmp/worktree"),
            ]
        );
    }

    #[cfg(unix)]
    #[test]
    fn run_command_with_timeout_high_output_no_false_timeout() {
        use super::run_command_with_timeout;

        // Generate >128 KB of output (well above typical 64 KB pipe buffer).
        // Without concurrent draining, the child blocks on write and is
        // killed on timeout — a false failure.
        let mut cmd = std::process::Command::new("sh");
        cmd.args(["-c", "seq 1 50000"]);
        let result = run_command_with_timeout(&mut cmd, Duration::from_secs(30));
        let output = result.expect("high-output command should not time out");
        assert!(output.status.success(), "command should exit 0");
        assert!(
            output.stdout.len() > 128 * 1024,
            "expected >128KB stdout, got {} bytes",
            output.stdout.len()
        );
    }

    #[cfg(unix)]
    #[test]
    fn run_command_with_timeout_kills_group_on_timeout() {
        use super::run_command_with_timeout;

        // Spawn a shell that starts a long-lived background child (`sleep 60`)
        // inheriting the pipe FDs. Without process-group kill, the reader
        // threads would block on the still-open pipes until the sleep exits.
        let mut cmd = std::process::Command::new("sh");
        cmd.args(["-c", "sleep 60 & echo started; wait"]);

        let start = Instant::now();
        let result = run_command_with_timeout(&mut cmd, Duration::from_secs(2));
        let elapsed = start.elapsed();

        assert!(result.is_err(), "should return timeout error");
        let err_msg = format!("{}", result.unwrap_err());
        assert!(
            err_msg.contains("timed out"),
            "error should mention timeout, got: {err_msg}"
        );
        // The function must return promptly after the 2s timeout.
        // Without the fix, it would block ~60s waiting for `sleep 60` to exit.
        assert!(
            elapsed < Duration::from_secs(10),
            "timeout should return promptly, took {elapsed:?}"
        );
    }

    #[test]
    fn quick_dev_run_command_includes_pr_url() {
        let tmp = tempfile::NamedTempFile::new().expect("create temp file");
        let cmd = build_ralph_quick_dev_run_command(
            Path::new("/tmp/ralph"),
            Path::new("/tmp/worktree"),
            "issue-7",
            tmp.path(),
            Some("https://github.com/acme/widgets/pull/7"),
        )
        .expect("build command");
        let args: Vec<&OsStr> = cmd.as_std().get_args().collect();
        assert_eq!(
            args,
            vec![
                OsStr::new("quick-dev-run"),
                OsStr::new("--project"),
                OsStr::new("issue-7"),
                OsStr::new("--workspace-root"),
                OsStr::new("/tmp/worktree"),
                OsStr::new("--pr-url"),
                OsStr::new("https://github.com/acme/widgets/pull/7"),
            ]
        );
    }
}
