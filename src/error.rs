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

    #[error("PRD cache mismatch: {0}")]
    PrdCacheMismatch(String),

    #[error("unsupported operation: {0}")]
    Unsupported(String),
}

impl RalphError {
    pub fn exit_code(&self) -> i32 {
        match self {
            Self::Validation(_)
            | Self::WorkspaceNotFound
            | Self::ProjectNotFound(_)
            | Self::ActiveProjectNotSet
            | Self::PrdCacheMismatch(_) => 2,
            Self::StateLocked { .. } => 3,
            Self::PrdPipelineFailed(_) => 10,
            Self::PrdValidationFailed(_) => 11,
            Self::PrdMissingInfo => 12,
            Self::QuickPrdFailed(_) => 13,
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
}
