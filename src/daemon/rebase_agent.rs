use std::path::Path;
use std::time::{Duration, Instant};

use crate::daemon::process;
use crate::error::RalphError;
use crate::git;
use crate::Result;

/// Maximum number of resolve/continue iterations before giving up.
const MAX_ITERATIONS: u32 = 10;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RebaseAgentBackend {
    None,
    Claude { model: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RebaseFailureKind {
    Conflict,
    Other,
}

/// Internal error enum for the rebase agent, mapped into `RalphError` at the boundary.
#[derive(Debug)]
enum AgentError {
    Timeout(String),
    SpawnFailed(String),
    AgentNonZero {
        exit_code: i32,
        stdout: String,
        stderr: String,
    },
    UnresolvedConflicts,
    IterationCapReached,
    RebaseContinueFailed(String),
}

impl AgentError {
    fn into_ralph_error(self) -> RalphError {
        match self {
            AgentError::Timeout(msg) => {
                RalphError::Orchestration(format!(
                    "rebase agent timeout (agent resolution was attempted): {msg}"
                ))
            }
            AgentError::SpawnFailed(msg) => {
                RalphError::Orchestration(format!(
                    "rebase agent spawn failed (agent resolution was attempted): {msg}"
                ))
            }
            AgentError::AgentNonZero {
                exit_code,
                stdout,
                stderr,
            } => {
                let mut msg = format!(
                    "rebase agent exited with non-zero status (agent resolution was attempted): {exit_code}"
                );
                if !stderr.is_empty() {
                    msg.push_str(&format!("\nstderr: {stderr}"));
                }
                if !stdout.is_empty() {
                    msg.push_str(&format!("\nstdout: {stdout}"));
                }
                RalphError::Orchestration(msg)
            }
            AgentError::UnresolvedConflicts => RalphError::Orchestration(
                "rebase agent completed but conflicts remain unresolved (agent resolution was attempted)".to_owned(),
            ),
            AgentError::IterationCapReached => RalphError::Orchestration(format!(
                "rebase agent iteration cap reached ({MAX_ITERATIONS} iterations) (agent resolution was attempted)"
            )),
            AgentError::RebaseContinueFailed(msg) => {
                RalphError::Orchestration(format!(
                    "git rebase --continue failed (agent resolution was attempted): {msg}"
                ))
            }
        }
    }
}

pub fn parse_rebase_agent_backend(raw: &str) -> Result<RebaseAgentBackend> {
    let value = raw.trim();

    if value.is_empty() {
        return Err(RalphError::Validation(
            "daemon rebase agent backend cannot be empty; supported values: none, claude, claude(<model>)".to_owned(),
        ));
    }

    if value == "none" {
        return Ok(RebaseAgentBackend::None);
    }

    if value == "claude" {
        return Ok(RebaseAgentBackend::Claude {
            model: "opus".to_owned(),
        });
    }

    if value.starts_with("claude(") && value.ends_with(')') {
        let model = &value["claude(".len()..value.len() - 1];
        let model = model.trim();
        if model.is_empty() {
            return Err(RalphError::Validation(
                "daemon rebase agent backend 'claude(...)' requires a non-empty model".to_owned(),
            ));
        }
        return Ok(RebaseAgentBackend::Claude {
            model: model.to_owned(),
        });
    }

    Err(RalphError::Validation(format!(
        "unsupported daemon rebase agent backend '{value}'; supported values: none, claude, claude(<model>)"
    )))
}

/// Pure criteria check: does exit code + stderr indicate a likely conflict?
///
/// This is unit-testable without I/O — it only inspects the exit code and
/// stderr content. The caller must separately verify with an I/O-based
/// conflict probe (e.g. `git::has_conflicts_with_timeout`) before treating
/// the failure as a confirmed conflict.
pub fn classify_rebase_failure_pure(exit_code: i32, stderr: &[u8]) -> RebaseFailureKind {
    if exit_code != 1 {
        return RebaseFailureKind::Other;
    }

    let stderr_text = String::from_utf8_lossy(stderr);
    let has_conflict_indicator =
        stderr_text.contains("CONFLICT") || stderr_text.contains("could not apply");

    if has_conflict_indicator {
        RebaseFailureKind::Conflict
    } else {
        RebaseFailureKind::Other
    }
}

/// Full conflict classification: pure criteria + I/O conflict probe.
///
/// Combines `classify_rebase_failure_pure` (exit code + stderr markers) with
/// an unbounded `git::has_conflicts` check. For deadline-bounded contexts,
/// callers should use `classify_rebase_failure_pure` followed by a separate
/// `git::has_conflicts_with_timeout` call.
pub fn classify_rebase_failure(
    exit_code: i32,
    stderr: &[u8],
    worktree_path: &Path,
) -> RebaseFailureKind {
    if classify_rebase_failure_pure(exit_code, stderr) != RebaseFailureKind::Conflict {
        return RebaseFailureKind::Other;
    }

    match git::has_conflicts(worktree_path) {
        Ok(true) => RebaseFailureKind::Conflict,
        _ => RebaseFailureKind::Other,
    }
}

/// Build the fixed prompt template for the rebase agent.
pub fn build_agent_prompt(rebase_target: &str, conflicting_files: &[String]) -> String {
    let file_list = conflicting_files
        .iter()
        .map(|f| format!("- {f}"))
        .collect::<Vec<_>>()
        .join("\n");

    format!(
        "You are resolving merge conflicts during a git rebase onto `{rebase_target}`.\n\
         \n\
         The following files have unresolved merge conflicts:\n\
         {file_list}\n\
         \n\
         Instructions:\n\
         1. Open each conflicting file and resolve the conflict markers (<<<<<<<, =======, >>>>>>>).\n\
         2. After resolving each file, stage it with `git add <file>`.\n\
         3. Do NOT run `git rebase --continue` or `git rebase --abort`.\n\
         4. Do NOT modify any files that are not listed above.\n\
         5. Ensure all conflict markers are fully resolved before staging.\n"
    )
}

/// Check if a rebase is currently in progress in the given worktree.
pub fn is_rebase_in_progress(worktree_path: &Path) -> bool {
    let git_dir = worktree_path.join(".git");
    // Standard repo layout
    if git_dir.join("rebase-merge").exists() || git_dir.join("rebase-apply").exists() {
        return true;
    }
    // Worktree layout: .git is a file pointing to the real gitdir
    if git_dir.is_file() {
        if let Ok(content) = std::fs::read_to_string(&git_dir) {
            let gitdir_line = content.trim();
            if let Some(path) = gitdir_line.strip_prefix("gitdir: ") {
                let real_git_dir = Path::new(path);
                if real_git_dir.join("rebase-merge").exists()
                    || real_git_dir.join("rebase-apply").exists()
                {
                    return true;
                }
            }
        }
    }
    false
}

/// Abort a rebase in progress, bounded by the given timeout budget.
fn abort_rebase_if_in_progress(worktree_path: &Path, deadline: Instant) {
    if is_rebase_in_progress(worktree_path) {
        let timeout = deadline
            .checked_duration_since(Instant::now())
            .unwrap_or(Duration::from_secs(10));
        // Use at least 5 seconds for abort even if budget is nearly exhausted
        let timeout = timeout.max(Duration::from_secs(5));
        let _ = process::run_command_with_timeout(
            std::process::Command::new("git")
                .args(["rebase", "--abort"])
                .current_dir(worktree_path),
            timeout,
        );
    }
}

/// Compute remaining time from a deadline; return error if expired.
fn remaining_budget(deadline: Instant, label: &str) -> std::result::Result<Duration, AgentError> {
    let now = Instant::now();
    if now >= deadline {
        return Err(AgentError::Timeout(format!(
            "{label}: timeout budget exhausted"
        )));
    }
    Ok(deadline - now)
}

/// Build the agent command for the given backend.
fn build_agent_command(
    worktree_path: &Path,
    backend: &RebaseAgentBackend,
    prompt: &str,
) -> std::process::Command {
    match backend {
        RebaseAgentBackend::Claude { model } => {
            let mut cmd = std::process::Command::new("claude");
            cmd.args([
                "-p",
                "--permission-mode",
                "acceptEdits",
                "--allowedTools",
                "Bash,Edit,Write,Read,Glob,Grep",
                "--model",
                model,
                prompt,
            ])
            .current_dir(worktree_path);
            cmd
        }
        RebaseAgentBackend::None => {
            unreachable!("build_agent_command should not be called with None backend")
        }
    }
}

/// Resolve rebase conflicts using an AI agent in a loop.
///
/// Public entrypoint accepting a raw backend string (`agent_backend`). The
/// string is parsed internally; invalid or unsupported values produce a clear
/// `RalphError`. The special value `"none"` disables agent invocation and
/// returns an error indicating no agent was attempted.
///
/// Each iteration: read conflicts, prompt agent, verify resolution, run `--continue`.
/// If `--continue` introduces new conflicts, repeat. Shared deadline across all steps.
pub fn resolve_rebase_conflicts(
    worktree_path: &Path,
    rebase_target: &str,
    agent_backend: &str,
    deadline: Instant,
) -> Result<()> {
    let backend = parse_rebase_agent_backend(agent_backend)?;
    if let RebaseAgentBackend::None = backend {
        // Agent disabled — abort rebase and return error for existing fallback path
        abort_rebase_if_in_progress(worktree_path, deadline);
        return Err(RalphError::Orchestration(
            "rebase agent backend is 'none'; agent resolution was skipped/disabled".to_owned(),
        ));
    }
    resolve_rebase_conflicts_typed(worktree_path, rebase_target, &backend, deadline)
}

/// Internal typed entrypoint for conflict resolution (backend already parsed).
fn resolve_rebase_conflicts_typed(
    worktree_path: &Path,
    rebase_target: &str,
    agent_backend: &RebaseAgentBackend,
    deadline: Instant,
) -> Result<()> {
    let result = resolve_loop(worktree_path, rebase_target, agent_backend, deadline);
    match result {
        Ok(()) => Ok(()),
        Err(agent_err) => {
            abort_rebase_if_in_progress(worktree_path, deadline);
            Err(agent_err.into_ralph_error())
        }
    }
}

fn resolve_loop(
    worktree_path: &Path,
    rebase_target: &str,
    agent_backend: &RebaseAgentBackend,
    deadline: Instant,
) -> std::result::Result<(), AgentError> {
    for iteration in 0..MAX_ITERATIONS {
        eprintln!(
            "rebase-agent: iteration {}/{MAX_ITERATIONS}",
            iteration + 1
        );

        // Step 1: Read conflicting files (with timeout)
        let files_budget = remaining_budget(
            deadline,
            &format!("conflicting_files (iteration {})", iteration + 1),
        )?;
        let files = git::conflicting_files_with_timeout(worktree_path, files_budget)
            .map_err(|e| {
                AgentError::SpawnFailed(format!("failed to read conflicting files: {e}"))
            })?;

        if files.is_empty() {
            // No conflicts — try rebase --continue directly
            let budget = remaining_budget(
                deadline,
                &format!("rebase --continue (iteration {})", iteration + 1),
            )?;
            run_rebase_continue(worktree_path, budget)?;

            // Verify rebase is actually complete
            if !is_rebase_in_progress(worktree_path) {
                return Ok(());
            }
            // Rebase still in progress — continue looping
            eprintln!("rebase-agent: rebase --continue succeeded but rebase still in progress, continuing");
            continue;
        }

        // Step 2: Build prompt
        let prompt = build_agent_prompt(rebase_target, &files);

        // Step 3: Invoke agent
        let agent_budget = remaining_budget(
            deadline,
            &format!("agent invocation (iteration {})", iteration + 1),
        )?;
        let mut cmd = build_agent_command(worktree_path, agent_backend, &prompt);
        let output = process::run_command_with_timeout(&mut cmd, agent_budget)
            .map_err(|e| AgentError::SpawnFailed(format!("{e}")))?;

        if !output.status.success() {
            let code = output.status.code().unwrap_or(-1);
            return Err(AgentError::AgentNonZero {
                exit_code: code,
                stdout: String::from_utf8_lossy(&output.stdout).trim().to_owned(),
                stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
            });
        }

        // Step 4: Verify conflicts cleared (with timeout)
        let verify_budget = remaining_budget(
            deadline,
            &format!("has_conflicts check (iteration {})", iteration + 1),
        )?;
        let still_has_conflicts =
            git::has_conflicts_with_timeout(worktree_path, verify_budget).map_err(
                |e| {
                    AgentError::SpawnFailed(format!(
                        "failed to check conflicts after agent: {e}"
                    ))
                },
            )?;

        if still_has_conflicts {
            return Err(AgentError::UnresolvedConflicts);
        }

        // Step 5: Run git rebase --continue
        let continue_budget = remaining_budget(
            deadline,
            &format!("rebase --continue (iteration {})", iteration + 1),
        )?;
        match run_rebase_continue(worktree_path, continue_budget) {
            Ok(()) => {
                // Verify rebase is actually complete
                if !is_rebase_in_progress(worktree_path) {
                    return Ok(());
                }
                // Rebase still in progress (more commits) — check for new conflicts
                let check_budget = remaining_budget(
                    deadline,
                    &format!("post-continue conflict check (iteration {})", iteration + 1),
                )?;
                let new_conflicts =
                    git::has_conflicts_with_timeout(worktree_path, check_budget)
                        .map_err(|e| {
                            AgentError::SpawnFailed(format!(
                                "post-continue conflict check failed: {e}"
                            ))
                        })?;
                if new_conflicts {
                    eprintln!(
                        "rebase-agent: rebase --continue succeeded but new conflicts appeared, repeating loop"
                    );
                    continue;
                }
                // No conflicts but rebase still in progress — continue looping
                eprintln!("rebase-agent: rebase --continue succeeded, rebase still in progress, continuing");
                continue;
            }
            Err(AgentError::RebaseContinueFailed(_)) => {
                // Check if new conflicts appeared (multi-commit rebase)
                let check_budget = remaining_budget(
                    deadline,
                    &format!("post-failure conflict check (iteration {})", iteration + 1),
                )?;
                let new_conflicts =
                    git::has_conflicts_with_timeout(worktree_path, check_budget)
                        .map_err(|e| {
                            AgentError::SpawnFailed(format!(
                                "post-failure conflict check failed: {e}"
                            ))
                        })?;
                if new_conflicts {
                    eprintln!(
                        "rebase-agent: rebase --continue produced new conflicts, repeating loop"
                    );
                    continue;
                }
                // Non-conflict failure from --continue
                return Err(AgentError::RebaseContinueFailed(
                    "rebase --continue failed without new conflicts".to_owned(),
                ));
            }
            Err(other) => return Err(other),
        }
    }

    Err(AgentError::IterationCapReached)
}

/// Run `git rebase --continue` and check the result.
///
/// Returns `Ok(())` if the command exits successfully. The caller is
/// responsible for checking `is_rebase_in_progress()` and
/// `has_conflicts_with_timeout()` to decide whether to loop again.
fn run_rebase_continue(
    worktree_path: &Path,
    budget: Duration,
) -> std::result::Result<(), AgentError> {
    let output = process::run_command_with_timeout(
        std::process::Command::new("git")
            .args(["rebase", "--continue"])
            .env("GIT_EDITOR", "true")
            .current_dir(worktree_path),
        budget,
    )
    .map_err(|e| AgentError::SpawnFailed(format!("rebase --continue: {e}")))?;

    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        Err(AgentError::RebaseContinueFailed(stderr))
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;
    use std::process::Command;
    use std::time::{Duration, Instant};

    use tempfile::TempDir;

    use super::{
        build_agent_prompt, classify_rebase_failure, classify_rebase_failure_pure,
        is_rebase_in_progress, parse_rebase_agent_backend, remaining_budget,
        RebaseAgentBackend, RebaseFailureKind, MAX_ITERATIONS,
    };

    // ---- Backend parsing tests (from loop 1) ----

    #[test]
    fn parse_rebase_agent_backend_none() {
        let parsed = parse_rebase_agent_backend("none").expect("parse none");
        assert_eq!(parsed, RebaseAgentBackend::None);
    }

    #[test]
    fn parse_rebase_agent_backend_claude_defaults_to_opus() {
        let parsed = parse_rebase_agent_backend("claude").expect("parse claude");
        assert_eq!(
            parsed,
            RebaseAgentBackend::Claude {
                model: "opus".to_owned()
            }
        );
    }

    #[test]
    fn parse_rebase_agent_backend_claude_opus() {
        let parsed = parse_rebase_agent_backend("claude(opus)").expect("parse claude(opus)");
        assert_eq!(
            parsed,
            RebaseAgentBackend::Claude {
                model: "opus".to_owned()
            }
        );
    }

    #[test]
    fn parse_rebase_agent_backend_claude_sonnet() {
        let parsed = parse_rebase_agent_backend("claude(sonnet)").expect("parse claude(sonnet)");
        assert_eq!(
            parsed,
            RebaseAgentBackend::Claude {
                model: "sonnet".to_owned()
            }
        );
    }

    #[test]
    fn parse_rebase_agent_backend_rejects_empty_string() {
        let err = parse_rebase_agent_backend("   ").expect_err("empty should be rejected");
        assert!(
            err.to_string().contains("cannot be empty"),
            "expected clear empty-value error, got: {err}"
        );
    }

    #[test]
    fn parse_rebase_agent_backend_rejects_unknown_value() {
        let err =
            parse_rebase_agent_backend("codex(gpt-5)").expect_err("unknown backend should fail");
        assert!(
            err.to_string()
                .contains("unsupported daemon rebase agent backend"),
            "expected unsupported-value error, got: {err}"
        );
    }

    // ---- Classification tests (from loop 1) ----

    #[test]
    fn classify_rebase_failure_detects_conflict() {
        let repo = create_repo_with_conflict();

        let kind = classify_rebase_failure(1, b"CONFLICT (content): Merge conflict", repo.path());
        assert_eq!(kind, RebaseFailureKind::Conflict);
    }

    #[test]
    fn classify_rebase_failure_non_conflict_exit_code() {
        let repo = create_repo_with_conflict();

        let kind = classify_rebase_failure(2, b"CONFLICT (content): Merge conflict", repo.path());
        assert_eq!(kind, RebaseFailureKind::Other);
    }

    #[test]
    fn classify_rebase_failure_missing_conflict_indicator_in_stderr() {
        let repo = create_repo_with_conflict();

        let kind = classify_rebase_failure(1, b"fatal: unrelated failure", repo.path());
        assert_eq!(kind, RebaseFailureKind::Other);
    }

    #[test]
    fn classify_rebase_failure_has_conflicts_false() {
        let repo = create_clean_repo();

        let kind = classify_rebase_failure(1, b"could not apply abcdef", repo.path());
        assert_eq!(kind, RebaseFailureKind::Other);
    }

    // ---- Prompt rendering tests (loop 2) ----

    #[test]
    fn prompt_includes_rebase_target_and_files() {
        let prompt = build_agent_prompt(
            "origin/main",
            &["src/foo.rs".to_owned(), "README.md".to_owned()],
        );
        assert!(
            prompt.contains("origin/main"),
            "prompt should include rebase target"
        );
        assert!(
            prompt.contains("- src/foo.rs"),
            "prompt should list conflicting files"
        );
        assert!(
            prompt.contains("- README.md"),
            "prompt should list conflicting files"
        );
    }

    #[test]
    fn prompt_requires_git_add() {
        let prompt = build_agent_prompt("origin/main", &["file.rs".to_owned()]);
        assert!(
            prompt.contains("git add"),
            "prompt should require git add staging"
        );
    }

    #[test]
    fn prompt_forbids_rebase_continue_and_abort() {
        let prompt = build_agent_prompt("origin/main", &["file.rs".to_owned()]);
        assert!(
            prompt.contains("Do NOT run `git rebase --continue`"),
            "prompt should forbid rebase --continue"
        );
        assert!(
            prompt.contains("`git rebase --abort`"),
            "prompt should forbid rebase --abort"
        );
    }

    #[test]
    fn prompt_forbids_unrelated_file_edits() {
        let prompt = build_agent_prompt("origin/main", &["file.rs".to_owned()]);
        assert!(
            prompt.contains("Do NOT modify any files that are not listed"),
            "prompt should forbid unrelated file edits"
        );
    }

    // ---- Timeout accounting tests (loop 2) ----

    #[test]
    fn remaining_budget_returns_duration_when_not_expired() {
        let deadline = Instant::now() + Duration::from_secs(60);
        let budget = remaining_budget(deadline, "test").expect("should have budget");
        assert!(budget.as_secs() > 0);
    }

    #[test]
    fn remaining_budget_returns_error_when_expired() {
        let deadline = Instant::now() - Duration::from_secs(1);
        let result = remaining_budget(deadline, "test");
        assert!(result.is_err(), "expired deadline should return error");
    }

    // ---- Loop cap test (loop 2) ----

    #[test]
    fn max_iterations_constant_is_10() {
        assert_eq!(MAX_ITERATIONS, 10);
    }

    // ---- Rebase-in-progress detection (loop 2) ----

    #[test]
    fn is_rebase_in_progress_false_for_clean_repo() {
        let repo = create_clean_repo();
        assert!(!is_rebase_in_progress(repo.path()));
    }

    // ---- Abort-on-failure test (loop 2) ----

    #[test]
    fn resolve_with_none_backend_returns_error_via_string_entrypoint() {
        let repo = create_clean_repo();
        let deadline = Instant::now() + Duration::from_secs(10);
        let result =
            super::resolve_rebase_conflicts(repo.path(), "origin/main", "none", deadline);
        assert!(result.is_err(), "none backend should return error");
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("none") && err.contains("skipped/disabled"),
            "error should indicate none backend skipped resolution: {err}"
        );
    }

    #[test]
    fn resolve_with_invalid_backend_returns_clear_error() {
        let repo = create_clean_repo();
        let deadline = Instant::now() + Duration::from_secs(10);
        let result = super::resolve_rebase_conflicts(
            repo.path(),
            "origin/main",
            "codex(gpt-5)",
            deadline,
        );
        assert!(result.is_err(), "unsupported backend should return error");
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("unsupported"),
            "error should mention unsupported backend: {err}"
        );
    }

    #[test]
    fn resolve_with_empty_backend_returns_clear_error() {
        let repo = create_clean_repo();
        let deadline = Instant::now() + Duration::from_secs(10);
        let result =
            super::resolve_rebase_conflicts(repo.path(), "origin/main", "  ", deadline);
        assert!(result.is_err(), "empty backend should return error");
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("cannot be empty"),
            "error should mention empty backend: {err}"
        );
    }

    // ---- Pure classifier tests (loop 6) ----

    #[test]
    fn classify_pure_conflict_exit_code_1_with_conflict_marker() {
        let kind = classify_rebase_failure_pure(1, b"CONFLICT (content): Merge conflict");
        assert_eq!(kind, RebaseFailureKind::Conflict);
    }

    #[test]
    fn classify_pure_conflict_exit_code_1_with_could_not_apply() {
        let kind = classify_rebase_failure_pure(1, b"error: could not apply abc1234");
        assert_eq!(kind, RebaseFailureKind::Conflict);
    }

    #[test]
    fn classify_pure_non_conflict_exit_code_not_1() {
        let kind = classify_rebase_failure_pure(2, b"CONFLICT (content): Merge conflict");
        assert_eq!(kind, RebaseFailureKind::Other);
    }

    #[test]
    fn classify_pure_non_conflict_no_indicator() {
        let kind = classify_rebase_failure_pure(1, b"fatal: unrelated failure");
        assert_eq!(kind, RebaseFailureKind::Other);
    }

    // ---- Normalized "none" backend parsing (loop 6) ----

    #[test]
    fn parse_rebase_agent_backend_trimmed_none() {
        let parsed = parse_rebase_agent_backend(" none ").expect("parse trimmed none");
        assert_eq!(parsed, RebaseAgentBackend::None);
    }

    #[test]
    fn resolve_with_trimmed_none_backend_returns_skipped_error() {
        let repo = create_clean_repo();
        let deadline = Instant::now() + Duration::from_secs(10);
        let result =
            super::resolve_rebase_conflicts(repo.path(), "origin/main", " none ", deadline);
        assert!(result.is_err(), "trimmed none backend should return error");
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("skipped/disabled"),
            "error should indicate agent was skipped/disabled: {err}"
        );
    }

    // ---- Timeout-bounded classification behavior (loop 6) ----

    #[test]
    fn remaining_budget_expired_message_contains_timeout() {
        let deadline = Instant::now() - Duration::from_secs(1);
        let result = remaining_budget(deadline, "classification");
        assert!(result.is_err());
        let err = format!("{:?}", result.unwrap_err());
        assert!(
            err.contains("timeout"),
            "expired budget error should mention timeout: {err}"
        );
    }

    // ---- Attempted/skipped error message wording (loop 6) ----

    #[test]
    fn none_backend_error_says_skipped_disabled() {
        let repo = create_clean_repo();
        let deadline = Instant::now() + Duration::from_secs(10);
        let result =
            super::resolve_rebase_conflicts(repo.path(), "origin/main", "none", deadline);
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("skipped/disabled"),
            "disabled-path error should say 'skipped/disabled': {err}"
        );
    }

    #[test]
    fn agent_error_messages_say_attempted() {
        use super::AgentError;

        let timeout_err = AgentError::Timeout("test".to_owned()).into_ralph_error();
        assert!(
            timeout_err.to_string().contains("agent resolution was attempted"),
            "timeout error should say attempted: {}",
            timeout_err
        );

        let spawn_err = AgentError::SpawnFailed("test".to_owned()).into_ralph_error();
        assert!(
            spawn_err.to_string().contains("agent resolution was attempted"),
            "spawn error should say attempted: {}",
            spawn_err
        );

        let nonzero_err = AgentError::AgentNonZero {
            exit_code: 1,
            stdout: String::new(),
            stderr: String::new(),
        }
        .into_ralph_error();
        assert!(
            nonzero_err.to_string().contains("agent resolution was attempted"),
            "non-zero error should say attempted: {}",
            nonzero_err
        );

        let unresolved_err = AgentError::UnresolvedConflicts.into_ralph_error();
        assert!(
            unresolved_err.to_string().contains("agent resolution was attempted"),
            "unresolved error should say attempted: {}",
            unresolved_err
        );

        let cap_err = AgentError::IterationCapReached.into_ralph_error();
        assert!(
            cap_err.to_string().contains("agent resolution was attempted"),
            "iteration cap error should say attempted: {}",
            cap_err
        );

        let continue_err =
            AgentError::RebaseContinueFailed("test".to_owned()).into_ralph_error();
        assert!(
            continue_err.to_string().contains("agent resolution was attempted"),
            "continue-failed error should say attempted: {}",
            continue_err
        );
    }

    // ---- Helpers ----

    fn create_clean_repo() -> TempDir {
        let tmp = TempDir::new().expect("create tempdir");
        run_git_expect_ok(tmp.path(), &["init"]);
        run_git_expect_ok(tmp.path(), &["config", "user.email", "test@example.com"]);
        run_git_expect_ok(tmp.path(), &["config", "user.name", "Test User"]);
        tmp
    }

    fn create_repo_with_conflict() -> TempDir {
        let tmp = create_clean_repo();

        fs::write(tmp.path().join("conflict.txt"), "base\n").expect("write base");
        run_git_expect_ok(tmp.path(), &["add", "conflict.txt"]);
        run_git_expect_ok(tmp.path(), &["commit", "-m", "base"]);

        run_git_expect_ok(tmp.path(), &["checkout", "-b", "feature"]);
        fs::write(tmp.path().join("conflict.txt"), "feature\n").expect("write feature");
        run_git_expect_ok(tmp.path(), &["add", "conflict.txt"]);
        run_git_expect_ok(tmp.path(), &["commit", "-m", "feature"]);

        run_git_expect_ok(tmp.path(), &["checkout", "master"]);
        fs::write(tmp.path().join("conflict.txt"), "master\n").expect("write master");
        run_git_expect_ok(tmp.path(), &["add", "conflict.txt"]);
        run_git_expect_ok(tmp.path(), &["commit", "-m", "master"]);

        let output = Command::new("git")
            .args(["merge", "feature"])
            .current_dir(tmp.path())
            .output()
            .expect("run git merge");
        assert!(
            !output.status.success(),
            "expected merge conflict but merge succeeded"
        );
        assert!(
            crate::git::has_conflicts(tmp.path()).expect("read conflict status"),
            "expected repository to contain unresolved conflicts after merge"
        );

        tmp
    }

    fn run_git_expect_ok(repo: &Path, args: &[&str]) {
        let output = Command::new("git")
            .args(args)
            .current_dir(repo)
            .output()
            .expect("run git command");
        assert!(
            output.status.success(),
            "git {:?} failed.\nstdout:\n{}\nstderr:\n{}",
            args,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
