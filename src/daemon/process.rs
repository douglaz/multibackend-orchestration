use std::io::Read;
use std::time::{Duration, Instant};

use nix::errno::Errno;
use nix::sys::signal::{kill, killpg, Signal};
use nix::unistd::Pid;

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
            let _ = Read::read_to_end(&mut out, &mut buf);
        }
        buf
    });

    let stderr_thread = std::thread::spawn(move || -> Vec<u8> {
        let mut buf = Vec::new();
        if let Some(mut err) = stderr_handle {
            let _ = Read::read_to_end(&mut err, &mut buf);
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
    #[cfg(unix)]
    use std::time::{Duration, Instant};

    #[cfg(unix)]
    use super::{pid_exists, terminate_process_group};

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

    #[cfg(unix)]
    #[test]
    fn run_command_with_timeout_high_output_no_false_timeout() {
        use super::run_command_with_timeout;

        // Generate >128 KB of output (well above typical 64 KB pipe buffer).
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
        assert!(
            elapsed < Duration::from_secs(10),
            "timeout should return promptly, took {elapsed:?}"
        );
    }
}
