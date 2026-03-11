use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use crate::util::time::{format_timestamp_yyyymmddhhmmss, now_utc};
use crate::Result;

pub const ARTIFACT_TIMESTAMP_LEN: usize = 14;

/// Sanitize a backend spec for use in filenames by replacing path-unsafe characters.
/// e.g. `claude(model/v2)` → `claude-model-v2`
pub(crate) fn slugify_backend(spec: &str) -> String {
    let s: String = spec
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect();
    s.trim_matches('-').to_owned()
}

#[derive(Debug, Clone)]
pub enum ArtifactKind {
    Spec,
    ImplNotes,
    ReviewFeedback { iteration: u32 },
    ImplResponse { iteration: u32 },
    ReviewApproved { iterations: u32 },
    QaPass { iteration: u32 },
    QaFail { iteration: u32 },
    ImplQaResponse { iteration: u32 },
    AcceptancePass,
    AcceptanceFail,
    TerminationRequest,
    CompleterVerdict,
    CompleterVerdictBackend { backend: String },
    FinalReviewProposals { backend: String },
    FinalReviewPlannerPositions,
    FinalReviewVotes { backend: String },
    FinalReviewArbiterRuling,
    FinalReviewExit { outcome: String },
    QuickDevPlanImplement,
    QuickDevCodexReview { satisfied: bool },
    QuickDevApplyFixes { iteration: u32 },
    QuickDevFinalReview { role: String, complete: bool },
    QuickDevForceComplete,
    PreCommitCheckFailure { iteration: u32 },
    ImplPreCommitResponse { iteration: u32 },
}

impl ArtifactKind {
    pub fn base_type(&self) -> &'static str {
        match self {
            Self::Spec => "spec",
            Self::ImplNotes => "impl-notes",
            Self::ReviewFeedback { .. } => "review-feedback",
            Self::ImplResponse { .. } => "impl-response",
            Self::ReviewApproved { .. } => "review-approved",
            Self::QaPass { .. } => "qa-pass",
            Self::QaFail { .. } => "qa-fail",
            Self::ImplQaResponse { .. } => "impl-qa-response",
            Self::AcceptancePass => "acceptance-pass",
            Self::AcceptanceFail => "acceptance-fail",
            Self::TerminationRequest => "termination-request",
            Self::CompleterVerdict => "completer-verdict",
            Self::CompleterVerdictBackend { .. } => "completer-verdict",
            Self::FinalReviewProposals { .. } => "final-review-proposals",
            Self::FinalReviewPlannerPositions => "final-review-planner-positions",
            Self::FinalReviewVotes { .. } => "final-review-votes",
            Self::FinalReviewArbiterRuling => "final-review-arbiter-ruling",
            Self::FinalReviewExit { .. } => "final-review-exit",
            Self::QuickDevPlanImplement => "quick-dev-plan-implement",
            Self::QuickDevCodexReview { .. } => "quick-dev-codex-review",
            Self::QuickDevApplyFixes { .. } => "quick-dev-apply-fixes",
            Self::QuickDevFinalReview { .. } => "quick-dev-final-review",
            Self::QuickDevForceComplete => "quick-dev-force-complete",
            Self::PreCommitCheckFailure { .. } => "pre-commit-failure",
            Self::ImplPreCommitResponse { .. } => "impl-pre-commit-response",
        }
    }

    pub fn file_name(&self) -> String {
        match self {
            Self::Spec => "spec.md".to_owned(),
            Self::ImplNotes => "impl-notes.md".to_owned(),
            Self::ReviewFeedback { iteration } => {
                format!("review-{iteration:03}-feedback.md")
            }
            Self::ImplResponse { iteration } => {
                format!("impl-response-{iteration:03}.md")
            }
            Self::ReviewApproved { .. } => "review-approved.md".to_owned(),
            Self::QaPass { iteration } => format!("qa-{iteration:03}-pass.md"),
            Self::QaFail { iteration } => format!("qa-{iteration:03}-fail.md"),
            Self::ImplQaResponse { iteration } => {
                format!("impl-qa-response-{iteration:03}.md")
            }
            Self::AcceptancePass => "acceptance-pass.md".to_owned(),
            Self::AcceptanceFail => "acceptance-fail.md".to_owned(),
            Self::TerminationRequest => "termination-request.md".to_owned(),
            Self::CompleterVerdict => "completer-verdict.md".to_owned(),
            Self::CompleterVerdictBackend { backend } => {
                format!("completer-verdict-{}.md", slugify_backend(backend))
            }
            Self::FinalReviewProposals { backend } => {
                format!("final-review-proposals-{}.md", slugify_backend(backend))
            }
            Self::FinalReviewPlannerPositions => "final-review-planner-positions.md".to_owned(),
            Self::FinalReviewVotes { backend } => {
                format!("final-review-votes-{}.md", slugify_backend(backend))
            }
            Self::FinalReviewArbiterRuling => "final-review-arbiter-ruling.md".to_owned(),
            Self::FinalReviewExit { outcome } => format!("final-review-exit-{outcome}.md"),
            Self::QuickDevPlanImplement => "quick-dev-plan-implement.md".to_owned(),
            Self::QuickDevCodexReview { satisfied } => {
                if *satisfied {
                    "quick-dev-codex-review-satisfied.md".to_owned()
                } else {
                    "quick-dev-codex-review-changes-requested.md".to_owned()
                }
            }
            Self::QuickDevApplyFixes { iteration } => {
                format!("quick-dev-apply-fixes-{iteration:03}.md")
            }
            Self::QuickDevFinalReview { role, complete } => {
                let outcome = if *complete { "complete" } else { "issues" };
                format!("quick-dev-final-review-{role}-{outcome}.md")
            }
            Self::QuickDevForceComplete => "quick-dev-force-complete.md".to_owned(),
            Self::PreCommitCheckFailure { iteration } => {
                format!("pre-commit-failure-{iteration:03}.md")
            }
            Self::ImplPreCommitResponse { iteration } => {
                format!("impl-pre-commit-response-{iteration:03}.md")
            }
        }
    }

    pub fn file_name_with_timestamp(&self, timestamp: &str) -> String {
        format!("{timestamp}-{}", self.file_name())
    }

    pub fn iteration(&self) -> Option<u32> {
        match self {
            Self::ReviewFeedback { iteration }
            | Self::ImplResponse { iteration }
            | Self::QaPass { iteration }
            | Self::QaFail { iteration }
            | Self::ImplQaResponse { iteration }
            | Self::QuickDevApplyFixes { iteration }
            | Self::PreCommitCheckFailure { iteration }
            | Self::ImplPreCommitResponse { iteration } => Some(*iteration),
            _ => None,
        }
    }

    pub fn iterations(&self) -> Option<u32> {
        match self {
            Self::ReviewApproved { iterations } => Some(*iterations),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ArtifactWriteInput<'a> {
    pub project_id: &'a str,
    pub loop_number: u32,
    pub loop_slug: &'a str,
    pub backend: &'a str,
    pub role: &'a str,
    pub kind: ArtifactKind,
    pub body: &'a str,
}

#[derive(Debug, Clone)]
pub struct ProjectScopedArtifactWriteInput<'a> {
    pub artifact: &'a str,
    pub file_name: &'a str,
    pub project_id: &'a str,
    pub backend: &'a str,
    pub role: &'a str,
    pub body: &'a str,
}

pub fn write_artifact(project_dir: &Path, input: ArtifactWriteInput<'_>) -> Result<PathBuf> {
    let loop_dir_name = format!("{:03}-{}", input.loop_number, input.loop_slug);
    let loop_dir = project_dir.join("loops").join(loop_dir_name);
    fs::create_dir_all(&loop_dir)?;

    let now = now_utc();
    let created_at = now.to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let timestamp = format_timestamp_yyyymmddhhmmss(now);
    let artifact_path = loop_dir.join(input.kind.file_name_with_timestamp(&timestamp));
    let body = strip_backend_frontmatter(input.body);

    let mut fm = String::from("---\n");
    fm.push_str(&format!("artifact: {}\n", input.kind.base_type()));
    fm.push_str(&format!("loop: {}\n", input.loop_number));
    if let Some(iteration) = input.kind.iteration() {
        fm.push_str(&format!("iteration: {iteration}\n"));
    }
    if let Some(iterations) = input.kind.iterations() {
        fm.push_str(&format!("iterations: {iterations}\n"));
    }
    fm.push_str(&format!("project: {}\n", input.project_id));
    fm.push_str(&format!("backend: {}\n", input.backend));
    fm.push_str(&format!("role: {}\n", input.role));
    fm.push_str(&format!("created_at: {created_at}\n"));
    fm.push_str("---\n\n");

    let content = format!("{}{body}\n", fm);
    fs::write(&artifact_path, content)?;

    Ok(artifact_path)
}

pub fn write_project_scoped_artifact(
    project_dir: &Path,
    input: ProjectScopedArtifactWriteInput<'_>,
) -> Result<PathBuf> {
    let now = now_utc();
    let created_at = now.to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let artifact_path = project_dir.join(input.file_name);
    let body = strip_backend_frontmatter(input.body);

    let mut fm = String::from("---\n");
    fm.push_str(&format!("artifact: {}\n", input.artifact));
    fm.push_str(&format!("project: {}\n", input.project_id));
    fm.push_str(&format!("backend: {}\n", input.backend));
    fm.push_str(&format!("role: {}\n", input.role));
    fm.push_str(&format!("created_at: {created_at}\n"));
    fm.push_str("---\n\n");

    let content = format!("{}{body}\n", fm);
    fs::write(&artifact_path, content)?;

    Ok(artifact_path)
}

pub fn artifact_relative_path(project_dir: &Path, artifact_path: &Path) -> String {
    artifact_path
        .strip_prefix(project_dir)
        .unwrap_or(artifact_path)
        .to_string_lossy()
        .replace('\\', "/")
}

pub fn parse_artifact_filename_timestamp(file_name: &str) -> Option<String> {
    let (prefix, _) = file_name.split_once('-')?;
    if prefix.len() == ARTIFACT_TIMESTAMP_LEN && prefix.chars().all(|c| c.is_ascii_digit()) {
        Some(prefix.to_owned())
    } else {
        None
    }
}

pub fn resolve_artifact_path_by_suffix(
    project_dir: &Path,
    loop_number: u32,
    loop_slug: &str,
    suffix: &str,
) -> Result<Option<String>> {
    let loop_dir_name = format!("{loop_number:03}-{loop_slug}");
    let loop_dir = project_dir.join("loops").join(loop_dir_name);

    let read_dir = match fs::read_dir(&loop_dir) {
        Ok(entries) => entries,
        Err(err) if err.kind() == ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(err.into()),
    };

    let mut best: Option<(Option<String>, String)> = None;
    for entry in read_dir {
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            continue;
        }

        let file_name = entry.file_name();
        let Some(file_name) = file_name.to_str() else {
            continue;
        };

        let timestamp = if file_name == suffix {
            None
        } else if let Some(prefix) = parse_artifact_filename_timestamp(file_name) {
            let expected_suffix = format!("-{suffix}");
            if file_name.ends_with(&expected_suffix) {
                Some(prefix)
            } else {
                continue;
            }
        } else {
            continue;
        };

        let rel = artifact_relative_path(project_dir, &entry.path());
        match &best {
            None => best = Some((timestamp, rel)),
            Some((best_ts, best_rel)) => {
                if is_candidate_better(timestamp.as_deref(), best_ts.as_deref(), &rel, best_rel) {
                    best = Some((timestamp, rel));
                }
            }
        }
    }

    Ok(best.map(|(_, rel)| rel))
}

fn is_candidate_better(
    candidate_ts: Option<&str>,
    best_ts: Option<&str>,
    candidate_rel: &str,
    best_rel: &str,
) -> bool {
    match (candidate_ts, best_ts) {
        (Some(c), Some(b)) => c > b || (c == b && candidate_rel > best_rel),
        (Some(_), None) => true,
        (None, Some(_)) => false,
        (None, None) => candidate_rel > best_rel,
    }
}

pub(crate) fn strip_backend_frontmatter(raw: &str) -> String {
    let trimmed = raw.trim();
    if !trimmed.starts_with("---") {
        return trimmed.to_owned();
    }

    let mut lines = trimmed.lines();
    let first = lines.next();
    if first != Some("---") {
        return trimmed.to_owned();
    }

    let mut in_frontmatter = true;
    let mut out = String::new();

    for line in lines {
        if in_frontmatter && line.trim() == "---" {
            in_frontmatter = false;
            continue;
        }
        if !in_frontmatter {
            out.push_str(line);
            out.push('\n');
        }
    }

    if in_frontmatter {
        trimmed.to_owned()
    } else {
        out.trim().to_owned()
    }
}

#[cfg(test)]
mod tests {
    use regex::Regex;
    use tempfile::TempDir;

    use super::{
        resolve_artifact_path_by_suffix, write_artifact, write_project_scoped_artifact,
        ArtifactKind, ArtifactWriteInput, ProjectScopedArtifactWriteInput,
    };

    #[test]
    fn write_artifact_prefixes_filename_with_timestamp() {
        let temp = TempDir::new().expect("temp dir");
        let project_dir = temp.path();

        let path = write_artifact(
            project_dir,
            ArtifactWriteInput {
                project_id: "demo",
                loop_number: 1,
                loop_slug: "sample",
                backend: "claude",
                role: "planner",
                kind: ArtifactKind::Spec,
                body: "# Feature: Demo\n\n## Description\nx\n\n## Acceptance Criteria\n- [ ] x\n\n## Files to Modify/Create\n- `a` - b\n\n## Dependencies\n- Requires: none\n- Blocks: none",
            },
        )
        .expect("write artifact");

        let file_name = path
            .file_name()
            .and_then(|s| s.to_str())
            .expect("utf8 file name");
        let re = Regex::new(r"^\d{14}-spec\.md$").expect("regex");
        assert!(
            re.is_match(file_name),
            "expected timestamp-prefixed filename, got {file_name}"
        );
    }

    #[test]
    fn resolve_artifact_path_by_suffix_prefers_latest_timestamped_file() {
        let temp = TempDir::new().expect("temp dir");
        let loop_dir = temp.path().join("loops/001-demo");
        std::fs::create_dir_all(&loop_dir).expect("create loop dir");

        std::fs::write(loop_dir.join("review-001-feedback.md"), "legacy").expect("write legacy");
        std::fs::write(
            loop_dir.join("20260203055910-review-001-feedback.md"),
            "old timestamp",
        )
        .expect("write old");
        std::fs::write(
            loop_dir.join("20260203060159-review-001-feedback.md"),
            "new timestamp",
        )
        .expect("write new");

        let resolved =
            resolve_artifact_path_by_suffix(temp.path(), 1, "demo", "review-001-feedback.md")
                .expect("resolve")
                .expect("path should exist");

        assert_eq!(
            resolved,
            "loops/001-demo/20260203060159-review-001-feedback.md"
        );
    }

    #[test]
    fn qa_and_acceptance_artifact_kinds_render_expected_names_and_types() {
        assert_eq!(
            ArtifactKind::QaPass { iteration: 1 }.file_name(),
            "qa-001-pass.md"
        );
        assert_eq!(
            ArtifactKind::QaFail { iteration: 7 }.file_name(),
            "qa-007-fail.md"
        );
        assert_eq!(
            ArtifactKind::ImplQaResponse { iteration: 3 }.file_name(),
            "impl-qa-response-003.md"
        );
        assert_eq!(
            ArtifactKind::AcceptancePass.file_name(),
            "acceptance-pass.md"
        );
        assert_eq!(
            ArtifactKind::AcceptanceFail.file_name(),
            "acceptance-fail.md"
        );

        assert_eq!(ArtifactKind::QaPass { iteration: 1 }.base_type(), "qa-pass");
        assert_eq!(ArtifactKind::QaFail { iteration: 1 }.base_type(), "qa-fail");
        assert_eq!(
            ArtifactKind::ImplQaResponse { iteration: 1 }.base_type(),
            "impl-qa-response"
        );
        assert_eq!(ArtifactKind::AcceptancePass.base_type(), "acceptance-pass");
        assert_eq!(ArtifactKind::AcceptanceFail.base_type(), "acceptance-fail");
    }

    #[test]
    fn final_review_artifact_kinds_render_expected_names_and_types() {
        assert_eq!(
            ArtifactKind::FinalReviewProposals {
                backend: "claude".to_owned()
            }
            .file_name(),
            "final-review-proposals-claude.md"
        );
        assert_eq!(
            ArtifactKind::FinalReviewPlannerPositions.file_name(),
            "final-review-planner-positions.md"
        );
        assert_eq!(
            ArtifactKind::FinalReviewVotes {
                backend: "codex(gpt-5)".to_owned()
            }
            .file_name(),
            "final-review-votes-codex-gpt-5.md"
        );
        assert_eq!(
            ArtifactKind::FinalReviewArbiterRuling.file_name(),
            "final-review-arbiter-ruling.md"
        );
        assert_eq!(
            ArtifactKind::FinalReviewExit {
                outcome: "approved".to_owned()
            }
            .file_name(),
            "final-review-exit-approved.md"
        );

        assert_eq!(
            ArtifactKind::FinalReviewProposals {
                backend: "claude".to_owned()
            }
            .base_type(),
            "final-review-proposals"
        );
        assert_eq!(
            ArtifactKind::FinalReviewPlannerPositions.base_type(),
            "final-review-planner-positions"
        );
        assert_eq!(
            ArtifactKind::FinalReviewVotes {
                backend: "codex".to_owned()
            }
            .base_type(),
            "final-review-votes"
        );
        assert_eq!(
            ArtifactKind::FinalReviewArbiterRuling.base_type(),
            "final-review-arbiter-ruling"
        );
        assert_eq!(
            ArtifactKind::FinalReviewExit {
                outcome: "restart".to_owned()
            }
            .base_type(),
            "final-review-exit"
        );
    }

    #[test]
    fn write_project_scoped_artifact_writes_project_frontmatter_schema() {
        let temp = TempDir::new().expect("temp dir");
        let project_dir = temp.path();

        let path = write_project_scoped_artifact(
            project_dir,
            ProjectScopedArtifactWriteInput {
                artifact: "prompt-review",
                file_name: "prompt-review.md",
                project_id: "demo",
                backend: "codex(gpt-5.4-xhigh)",
                role: "prompt_reviewer",
                body: "# Prompt Review\n\n## Issues Found\n- mock\n\n## Refined Prompt\nThis is a sufficiently long refined prompt.",
            },
        )
        .expect("write project artifact");

        assert_eq!(
            path,
            project_dir.join("prompt-review.md"),
            "project-scoped artifact should be written directly under project root"
        );

        let content = std::fs::read_to_string(path).expect("read artifact");
        assert!(content.contains("artifact: prompt-review"));
        assert!(content.contains("project: demo"));
        assert!(content.contains("backend: codex(gpt-5.4-xhigh)"));
        assert!(content.contains("role: prompt_reviewer"));
        assert!(content.contains("created_at: "));
        assert!(
            !content.contains("\nloop: "),
            "project-scoped artifact must not include loop field"
        );
        assert!(
            !content.contains("\niteration: "),
            "project-scoped artifact must not include iteration field"
        );
        assert!(
            !content.contains("\niterations: "),
            "project-scoped artifact must not include iterations field"
        );
    }
}
