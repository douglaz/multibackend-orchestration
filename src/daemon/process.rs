use std::path::Path;
use std::time::{Duration, Instant};

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
) -> Result<SpawnedChild> {
    let mut cmd = build_ralph_auto_command(ralph_bin, worktree_path, idea, log_file, project_id)?;

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
) -> Result<SpawnedChild> {
    let mut cmd = build_ralph_run_command(ralph_bin, worktree_path, project_id, log_file)?;

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
) -> Result<Command> {
    let file = std::fs::File::create(log_file).map_err(|err| {
        RalphError::Orchestration(format!(
            "failed to create log file {}: {err}",
            log_file.display()
        ))
    })?;
    let file_clone = file.try_clone().map_err(|err| {
        RalphError::Orchestration(format!("failed to clone log file handle: {err}"))
    })?;

    let mut cmd = Command::new(ralph_bin);
    cmd.args(["auto", "--idea", idea]);
    if let Some(project_id) = project_id {
        cmd.args(["--project-id", project_id]);
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
) -> Result<Command> {
    let file = std::fs::File::create(log_file).map_err(|err| {
        RalphError::Orchestration(format!(
            "failed to create log file {}: {err}",
            log_file.display()
        ))
    })?;
    let file_clone = file.try_clone().map_err(|err| {
        RalphError::Orchestration(format!("failed to clone log file handle: {err}"))
    })?;

    let mut cmd = Command::new(ralph_bin);
    cmd.args(["run", "--project", project_id, "--until-complete"])
        .current_dir(worktree_path)
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
    use std::ffi::OsStr;
    use std::path::Path;
    #[cfg(unix)]
    use std::time::{Duration, Instant};

    use super::{build_ralph_auto_command, build_ralph_run_command};
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
        )
        .expect("build command");
        let args: Vec<&OsStr> = cmd.as_std().get_args().collect();
        assert_eq!(
            args,
            vec![
                OsStr::new("auto"),
                OsStr::new("--idea"),
                OsStr::new("implement feature"),
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
        )
        .expect("build command");
        let args: Vec<&OsStr> = cmd.as_std().get_args().collect();
        assert_eq!(
            args,
            vec![
                OsStr::new("auto"),
                OsStr::new("--idea"),
                OsStr::new("implement feature"),
                OsStr::new("--project-id"),
                OsStr::new("issue-42"),
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
            ]
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
}
