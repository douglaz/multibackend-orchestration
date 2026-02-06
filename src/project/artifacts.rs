use std::fs;
use std::path::{Path, PathBuf};

use crate::util::time::now_iso8601;
use crate::Result;

#[derive(Debug, Clone)]
pub enum ArtifactKind {
    Spec,
    ImplNotes,
    ReviewFeedback { iteration: u32 },
    ImplResponse { iteration: u32 },
    ReviewApproved { iterations: u32 },
    TerminationRequest,
    CompleterVerdict,
}

impl ArtifactKind {
    pub fn base_type(&self) -> &'static str {
        match self {
            Self::Spec => "spec",
            Self::ImplNotes => "impl-notes",
            Self::ReviewFeedback { .. } => "review-feedback",
            Self::ImplResponse { .. } => "impl-response",
            Self::ReviewApproved { .. } => "review-approved",
            Self::TerminationRequest => "termination-request",
            Self::CompleterVerdict => "completer-verdict",
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
            Self::TerminationRequest => "termination-request.md".to_owned(),
            Self::CompleterVerdict => "completer-verdict.md".to_owned(),
        }
    }

    pub fn iteration(&self) -> Option<u32> {
        match self {
            Self::ReviewFeedback { iteration } | Self::ImplResponse { iteration } => {
                Some(*iteration)
            }
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

pub fn write_artifact(project_dir: &Path, input: ArtifactWriteInput<'_>) -> Result<PathBuf> {
    let loop_dir_name = format!("{:03}-{}", input.loop_number, input.loop_slug);
    let loop_dir = project_dir.join("loops").join(loop_dir_name);
    fs::create_dir_all(&loop_dir)?;

    let artifact_path = loop_dir.join(input.kind.file_name());
    let created_at = now_iso8601();
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

pub fn artifact_relative_path(project_dir: &Path, artifact_path: &Path) -> String {
    artifact_path
        .strip_prefix(project_dir)
        .unwrap_or(artifact_path)
        .to_string_lossy()
        .replace('\\', "/")
}

fn strip_backend_frontmatter(raw: &str) -> String {
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
