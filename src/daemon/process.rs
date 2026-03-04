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
/// Stdout and stderr are piped so the caller can inspect them.
pub fn run_command_with_timeout(
    cmd: &mut std::process::Command,
    timeout: Duration,
) -> crate::Result<std::process::Output> {
    let mut child = cmd
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|err| {
            crate::error::RalphError::Orchestration(format!("failed to spawn command: {err}"))
        })?;

    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(_status)) => {
                let output = child.wait_with_output().map_err(|err| {
                    crate::error::RalphError::Orchestration(format!(
                        "failed to collect command output: {err}"
                    ))
                })?;
                return Ok(output);
            }
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(crate::error::RalphError::Orchestration(
                        "command timed out".to_owned(),
                    ));
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(err) => {
                return Err(crate::error::RalphError::Orchestration(format!(
                    "failed to check command status: {err}"
                )));
            }
        }
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

pub fn terminate_process_group_blocking(pgid: u32, timeout: Duration) {
    match tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
    {
        Ok(runtime) => runtime.block_on(terminate_process_group(pgid, timeout)),
        Err(err) => {
            eprintln!("warning: failed to initialize tokio runtime for abort path: {err}");
        }
    }
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
    use super::{pid_exists, terminate_process_group_blocking};

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
    fn test_terminate_process_group_noop_for_low_pgid() {
        terminate_process_group_blocking(0, Duration::from_millis(50));
        terminate_process_group_blocking(1, Duration::from_millis(50));
    }

    #[cfg(unix)]
    #[test]
    fn test_terminate_process_group_dead_pgid() {
        let mut child = std::process::Command::new("sleep")
            .arg("30")
            .spawn()
            .expect("spawn child");
        let non_group_id = child.id();

        terminate_process_group_blocking(non_group_id, Duration::from_millis(50));
        assert!(
            child.try_wait().expect("poll child").is_none(),
            "child should still be running"
        );

        let _ = child.kill();
        let _ = child.wait();
    }

    #[cfg(unix)]
    #[test]
    fn test_terminate_spawned_process_group() {
        use std::os::unix::process::{CommandExt, ExitStatusExt};

        let mut child = std::process::Command::new("sleep")
            .arg("30")
            .process_group(0)
            .spawn()
            .expect("spawn process-group leader");
        let pgid = child.id();

        terminate_process_group_blocking(pgid, Duration::from_secs(2));

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
