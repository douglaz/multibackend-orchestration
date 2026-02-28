use std::path::PathBuf;

use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimeoutKind {
    Idle,
    Walltime,
}

#[derive(Debug, Error)]
pub enum RalphError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("yaml error: {0}")]
    Yaml(#[from] serde_yaml::Error),

    #[error("toml decode error: {0}")]
    TomlDecode(#[from] toml::de::Error),

    #[error("toml encode error: {0}")]
    TomlEncode(#[from] toml::ser::Error),

    #[error("workspace not found (expected .ralph in current dir or ancestors)")]
    WorkspaceNotFound,

    #[error("active project is not set")]
    ActiveProjectNotSet,

    #[error("project not found: {0}")]
    ProjectNotFound(String),

    #[error("invalid input: {0}")]
    Validation(String),

    #[error("invalid init target '{path}': {reason}")]
    InitTargetInvalid { path: PathBuf, reason: String },

    #[error("state is locked for project {project_id}: {lock_path}")]
    StateLocked {
        project_id: String,
        lock_path: PathBuf,
    },

    #[error("daemon is already running for repo {repo_root}: {lock_path}")]
    DaemonLocked {
        repo_root: PathBuf,
        lock_path: PathBuf,
    },

    #[error("corrupted state at {path}: {reason}")]
    CorruptedState { path: PathBuf, reason: String },

    #[error("backend unavailable: {backend}")]
    BackendUnavailable { backend: String },

    #[error("tmux is not installed or not on PATH; install tmux to use tmux mode")]
    TmuxUnavailable,

    #[error(
        "backend timeout: {backend} (idle_seconds={idle_seconds}, timeout_kind={timeout_kind:?})"
    )]
    BackendTimeout {
        backend: String,
        idle_seconds: u64,
        timeout_kind: TimeoutKind,
    },

    #[error("backend command failed for {backend}: {details}")]
    BackendCommandFailed { backend: String, details: String },

    #[error(
        "BackendTimeoutExhausted: backend timeout retries exhausted for {backend} during {phase} (role={role}, timeout={timeout_secs}s) after {attempts} attempts"
    )]
    BackendTimeoutExhausted {
        backend: String,
        phase: String,
        role: String,
        timeout_secs: u64,
        attempts: u8,
    },

    #[error("parse error: {0}")]
    ParseError(String),

    #[error("parse retries exhausted for role {role} during {phase} after {attempts} attempts")]
    ParseRetriesExhausted {
        role: String,
        phase: String,
        attempts: u8,
    },

    #[error("review iteration limit exceeded for loop {loop_number}, max={max_iterations}")]
    ReviewIterationLimitExceeded {
        loop_number: u32,
        max_iterations: u32,
    },

    #[error("QA iteration limit exceeded for loop {loop_number}, max={max_iterations}")]
    QaIterationLimitExceeded {
        loop_number: u32,
        max_iterations: u32,
    },

    #[error("git conflict: {details}")]
    GitConflict { details: String },

    #[error("branch mismatch: expected '{expected}', got '{actual}'")]
    BranchMismatch { expected: String, actual: String },

    #[error("orchestration error: {0}")]
    Orchestration(String),

    #[error("PRD pipeline failed: {0}")]
    PrdPipelineFailed(String),

    #[error("PRD validation failed: {0}")]
    PrdValidationFailed(String),

    #[error("PRD missing information -- see missing_info_report.md")]
    PrdMissingInfo,

    #[error("quick PRD failed: {0}")]
    QuickPrdFailed(String),

    #[error("interactive PRD failed: {0}")]
    InteractivePrdFailed(String),

    #[error("PRD cache mismatch: {0}")]
    PrdCacheMismatch(String),

    #[error("unsupported operation: {0}")]
    Unsupported(String),
}

impl RalphError {
    /// Returns true if this is a transient quota/rate-limit error that should
    /// be treated as a soft failure (skip the backend, don't fail the task).
    pub fn is_quota_error(&self) -> bool {
        match self {
            RalphError::BackendCommandFailed { details, .. } => {
                details.contains("TerminalQuotaError")
                    || details.contains("RESOURCE_EXHAUSTED")
                    || details.contains("MODEL_CAPACITY_EXHAUSTED")
                    || details.contains("quota will reset")
                    || details.contains("hit your usage limit")
                    || details.contains("creditsExhausted")
                    || details.contains("add more credits")
                    || details.contains("insufficient_quota")
                    || details.contains("billing_hard_limit_reached")
            }
            _ => false,
        }
    }

    /// Returns true when a failure is likely transient and safe to retry.
    pub fn is_transient(&self) -> bool {
        match self {
            RalphError::Validation(_)
            | RalphError::InitTargetInvalid { .. }
            | RalphError::WorkspaceNotFound
            | RalphError::Json(_)
            | RalphError::Yaml(_)
            | RalphError::TomlDecode(_)
            | RalphError::ActiveProjectNotSet
            | RalphError::ProjectNotFound(_)
            | RalphError::GitConflict { .. }
            | RalphError::BranchMismatch { .. }
            | RalphError::TmuxUnavailable
            | RalphError::PrdValidationFailed(_)
            | RalphError::PrdMissingInfo
            | RalphError::PrdCacheMismatch(_)
            | RalphError::TomlEncode(_)
            | RalphError::Unsupported(_) => false,
            RalphError::Orchestration(message) => {
                let lower = message.to_ascii_lowercase();
                lower.contains("timeout")
                    || lower.contains("timed out")
                    || lower.contains("network")
                    || lower.contains("connection")
                    || lower.contains("transport")
                    || lower.contains("rate limit")
                    || lower.contains("too many requests")
                    || lower.contains("temporar")
                    || lower.contains("unavailable")
                    || lower.contains("try again")
                    || lower.contains("failed to execute")
                    || lower.contains("subprocess")
                    || lower.contains("broken pipe")
            }
            _ => true,
        }
    }

    pub fn exit_code(&self) -> i32 {
        match self {
            Self::Validation(_)
            | Self::WorkspaceNotFound
            | Self::ProjectNotFound(_)
            | Self::ActiveProjectNotSet
            | Self::PrdCacheMismatch(_) => 2,
            Self::StateLocked { .. } | Self::DaemonLocked { .. } => 3,
            Self::PrdPipelineFailed(_) => 10,
            Self::PrdValidationFailed(_) => 11,
            Self::PrdMissingInfo => 12,
            Self::QuickPrdFailed(_) => 13,
            Self::InteractivePrdFailed(_) => 14,
            _ => 1,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{RalphError, TimeoutKind};

    #[test]
    fn backend_timeout_display_and_debug_include_idle_variant_context() {
        let err = RalphError::BackendTimeout {
            backend: "claude".to_owned(),
            idle_seconds: 12,
            timeout_kind: TimeoutKind::Idle,
        };

        let display = err.to_string();
        assert!(display.contains("claude"));
        assert!(display.contains("idle_seconds=12"));
        assert!(display.contains("timeout_kind=Idle"));

        let debug = format!("{err:?}");
        assert!(debug.contains("idle_seconds: 12"));
        assert!(debug.contains("timeout_kind: Idle"));
    }

    #[test]
    fn backend_timeout_display_and_debug_include_walltime_variant_context() {
        let err = RalphError::BackendTimeout {
            backend: "codex".to_owned(),
            idle_seconds: 30,
            timeout_kind: TimeoutKind::Walltime,
        };

        let display = err.to_string();
        assert!(display.contains("codex"));
        assert!(display.contains("idle_seconds=30"));
        assert!(display.contains("timeout_kind=Walltime"));

        let debug = format!("{err:?}");
        assert!(debug.contains("idle_seconds: 30"));
        assert!(debug.contains("timeout_kind: Walltime"));
    }

    #[test]
    fn is_quota_error_matches_known_patterns() {
        let patterns = [
            "TerminalQuotaError",
            "RESOURCE_EXHAUSTED",
            "MODEL_CAPACITY_EXHAUSTED",
            "quota will reset",
            "hit your usage limit",
            "creditsExhausted",
            "add more credits",
            "insufficient_quota",
            "billing_hard_limit_reached",
        ];
        for pat in patterns {
            let err = RalphError::BackendCommandFailed {
                backend: "test".to_owned(),
                details: format!("error: {pat} occurred"),
            };
            assert!(err.is_quota_error(), "expected is_quota_error() for: {pat}");
        }
    }

    #[test]
    fn is_quota_error_rejects_unrelated_errors() {
        let err = RalphError::BackendCommandFailed {
            backend: "test".to_owned(),
            details: "parse error: missing top-level H1".to_owned(),
        };
        assert!(!err.is_quota_error());
    }

    #[test]
    fn interactive_prd_failed_has_expected_exit_code() {
        let err = RalphError::InteractivePrdFailed("boom".to_owned());
        assert_eq!(err.exit_code(), 14);
    }

    #[test]
    fn is_transient_distinguishes_terminal_and_transient_errors() {
        let terminal = RalphError::BranchMismatch {
            expected: "ralph/issue-93".to_owned(),
            actual: "main".to_owned(),
        };
        assert!(!terminal.is_transient());

        let transient = RalphError::Orchestration("network timeout while calling gh".to_owned());
        assert!(transient.is_transient());
    }

    #[test]
    fn branch_mismatch_display_includes_expected_and_actual() {
        let err = RalphError::BranchMismatch {
            expected: "ralph/issue-93".to_owned(),
            actual: "main".to_owned(),
        };

        let message = err.to_string();
        assert!(message.contains("branch mismatch"));
        assert!(message.contains("ralph/issue-93"));
        assert!(message.contains("main"));
    }
}
