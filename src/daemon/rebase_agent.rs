use std::path::Path;

use crate::error::RalphError;
use crate::git;
use crate::Result;

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

pub fn classify_rebase_failure(
    exit_code: i32,
    stderr: &[u8],
    worktree_path: &Path,
) -> RebaseFailureKind {
    if exit_code != 1 {
        return RebaseFailureKind::Other;
    }

    let stderr_text = String::from_utf8_lossy(stderr);
    let has_conflict_indicator =
        stderr_text.contains("CONFLICT") || stderr_text.contains("could not apply");

    if !has_conflict_indicator {
        return RebaseFailureKind::Other;
    }

    match git::has_conflicts(worktree_path) {
        Ok(true) => RebaseFailureKind::Conflict,
        _ => RebaseFailureKind::Other,
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;
    use std::process::Command;

    use tempfile::TempDir;

    use super::{
        classify_rebase_failure, parse_rebase_agent_backend, RebaseAgentBackend, RebaseFailureKind,
    };

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
