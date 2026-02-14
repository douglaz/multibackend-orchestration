use std::path::Path;
use std::time::{Duration, Instant};

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
) -> Result<SpawnedChild> {
    let mut cmd = build_ralph_auto_command(ralph_bin, worktree_path, idea, log_file)?;

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

fn build_ralph_auto_command(
    ralph_bin: &Path,
    worktree_path: &Path,
    idea: &str,
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
    cmd.args(["auto", "--idea", idea])
        .current_dir(worktree_path)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::from(file))
        .stderr(std::process::Stdio::from(file_clone));
    Ok(cmd)
}

/// Check if a process with the given PID exists.
pub fn pid_exists(pid: u32) -> bool {
    // Use kill(pid, 0) to probe without sending a signal.
    // SAFETY: kill with signal 0 just checks for process existence.
    unsafe { libc::kill(pid as i32, 0) == 0 }
}

/// Terminate a process group gracefully (SIGTERM), escalating to SIGKILL
/// after the given timeout.
pub async fn terminate_process_group(pgid: u32, timeout: Duration) {
    if pgid == 0 {
        return;
    }

    let neg_pgid = -(pgid as i32);

    // Check if the group exists
    // SAFETY: kill with signal 0 just checks for process existence.
    let exists = unsafe { libc::kill(neg_pgid, 0) == 0 };
    if !exists {
        return;
    }

    // Send SIGTERM to the process group
    // SAFETY: sending SIGTERM to a known process group.
    unsafe {
        libc::kill(neg_pgid, libc::SIGTERM);
    }

    // Wait for processes to exit
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        // SAFETY: kill with signal 0 checks existence.
        let still_exists = unsafe { libc::kill(neg_pgid, 0) == 0 };
        if !still_exists {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    // Escalate to SIGKILL
    // SAFETY: sending SIGKILL to a known process group.
    unsafe {
        libc::kill(neg_pgid, libc::SIGKILL);
    }
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

    use super::build_ralph_auto_command;

    #[test]
    fn spawn_command_uses_long_idea_flag() {
        let tmp = tempfile::NamedTempFile::new().expect("create temp file");
        let cmd = build_ralph_auto_command(
            Path::new("/tmp/ralph"),
            Path::new("/tmp/worktree"),
            "implement feature",
            tmp.path(),
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
}
