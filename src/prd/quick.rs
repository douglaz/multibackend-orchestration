//! Quick PRD foundational types, prompts, helper functions, and runtime pipeline.

use std::fs::{self, File, OpenOptions};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use fs2::FileExt;
use serde::{Deserialize, Serialize};

use crate::backend::Backend;
use crate::error::RalphError;
use crate::prd::gaps::extract_fenced_json;
use crate::util::hash::sha256_hex;
use crate::util::time::now_iso8601;
use crate::workflow::parser::strip_frontmatter;
use crate::Result;

const REQUIRED_SECTIONS: [&str; 6] = [
    "## Summary",
    "## Acceptance Criteria",
    "## Technical Approach",
    "## Files & Modules",
    "## Testing Strategy",
    "## Out of Scope",
];

/// Options for running quick PRD generation.
#[derive(Debug, Clone)]
pub struct QuickPrdOptions {
    pub idea: String,
    pub writer_spec: String,
    pub reviewer_spec: String,
    pub max_revisions: u32,
    pub dry_run: bool,
}

/// Result of a completed quick PRD run.
#[derive(Debug, Clone)]
pub struct QuickPrdResult {
    pub spec_path: PathBuf,
    pub cache_dir: PathBuf,
    pub revision_count: u32,
    pub approved: bool,
    pub summary: String,
}

/// Metadata persisted for a quick PRD run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QuickPrdMeta {
    pub idea: String,
    pub idea_hash: String,
    pub writer_backend: String,
    pub reviewer_backend: String,
    pub started_at: String,
    pub completed_at: String,
    pub revision_count: u32,
    pub approved: bool,
    pub draft_time_secs: f64,
    pub review_times_secs: Vec<f64>,
    pub revision_times_secs: Vec<f64>,
}

/// Structured reviewer response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewFeedback {
    pub approved: bool,
    pub issues: Vec<ReviewIssue>,
}

/// A single reviewer issue.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewIssue {
    pub area: String,
    pub feedback: String,
}

/// Draft prompt used by the writer backend.
pub const DRAFT_PROMPT: &str = r#"You are a senior software engineer writing a focused engineering specification.

**Feature Idea:**
{{idea}}

**Required Output Format:**
Your response must be a markdown document with the following exact section headings:

## Summary
## Acceptance Criteria
## Technical Approach
## Files & Modules
## Testing Strategy
## Out of Scope

Each section should be concise, specific, and implementation-ready.
"#;

/// Review prompt used by the reviewer backend.
pub const REVIEW_PROMPT: &str = r#"You are a senior engineer reviewing an engineering specification for completeness and feasibility.

**Feature Idea:**
{{idea}}

**Engineering Spec:**
{{spec}}

**Task:**
Review the spec for: technical feasibility, missing edge cases, completeness of acceptance criteria, testing coverage, and clarity.

**Required Output Format:**
Your response MUST be a single fenced JSON block:

```json
{"approved": true, "issues": []}
```

If issues found:

```json
{"approved": false, "issues": [{"area": "...", "feedback": "..."}]}
```
"#;

/// Revision prompt used by the writer backend.
pub const REVISION_PROMPT: &str = r#"You are a senior software engineer revising an engineering specification based on review feedback.

**Feature Idea:**
{{idea}}

**Current Spec:**
{{spec}}

**Review Issues:**
{{issues}}

**Task:**
Address each review issue and produce an updated specification. You MUST preserve the same 6 required section headings:
## Summary, ## Acceptance Criteria, ## Technical Approach, ## Files & Modules, ## Testing Strategy, ## Out of Scope
"#;

/// Simple inline placeholder replacement.
pub fn render_prompt(template: &str, replacements: &[(&str, &str)]) -> String {
    let mut result = template.to_string();
    for (placeholder, value) in replacements {
        result = result.replace(placeholder, value);
    }
    result
}

/// Checks quick PRD spec output for required sections after frontmatter removal.
pub fn check_spec_sections(raw: &str) -> (String, Vec<String>) {
    let cleaned = strip_frontmatter(raw);
    let mut missing_sections = Vec::new();

    for section in REQUIRED_SECTIONS {
        if !cleaned.lines().any(|line| line.trim() == section) {
            missing_sections.push(section.to_string());
        }
    }

    (cleaned, missing_sections)
}

/// Parses reviewer feedback from a fenced JSON payload.
pub fn parse_review_feedback(raw: &str) -> Result<ReviewFeedback> {
    let fenced_json = extract_fenced_json(raw)?;
    let feedback = serde_json::from_str::<ReviewFeedback>(fenced_json)?;
    Ok(feedback)
}

/// Formats issues as a numbered list for revision prompts.
pub fn format_issues(issues: &[ReviewIssue]) -> String {
    if issues.is_empty() {
        return "(none)".to_string();
    }

    issues
        .iter()
        .enumerate()
        .map(|(index, issue)| format!("{}. {}: {}", index + 1, issue.area, issue.feedback))
        .collect::<Vec<_>>()
        .join("\n")
}

const MAX_SECTION_RETRIES: u8 = 2;

/// Runs a review backend call with up to 3 parse attempts.
/// On parse failure, retries with a strict reformat prompt requesting a single fenced JSON block.
/// Returns an error if all 3 attempts fail (no silent fallback).
pub async fn run_review_with_retry(
    backend: Arc<dyn Backend>,
    prompt: String,
) -> Result<ReviewFeedback> {
    let mut current_prompt = prompt;

    for attempt in 1..=3_u8 {
        let raw = backend.execute(&current_prompt).await?;
        match parse_review_feedback(&raw) {
            Ok(feedback) => return Ok(feedback),
            Err(parse_error) => {
                if attempt == 3 {
                    return Err(RalphError::QuickPrdFailed(format!(
                        "failed to parse review feedback after 3 attempts: {parse_error}"
                    )));
                }
                current_prompt = format!(
                    "CRITICAL: Your previous review response could not be parsed.\n\n\
                     Error: {parse_error}\n\n\
                     Return ONLY a single fenced JSON block with this exact schema:\n\
                     ```json\n\
                     {{\"approved\": true/false, \"issues\": [{{\"area\": \"...\", \"feedback\": \"...\"}}]}}\n\
                     ```\n\
                     Use valid JSON, no prose before or after the fenced block.\n\n\
                     Previous response:\n---\n{raw}\n---\n"
                );
            }
        }
    }

    unreachable!("loop should return or error before reaching this point")
}

/// Exclusive file lock for quick-prd cache directory.
#[derive(Debug)]
struct QuickPrdLock {
    _file: File,
}

impl QuickPrdLock {
    fn acquire(lock_path: &PathBuf) -> Result<Self> {
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(lock_path)?;

        if file.try_lock_exclusive().is_err() {
            return Err(RalphError::QuickPrdFailed(format!(
                "quick-prd cache is locked: {}",
                lock_path.display()
            )));
        }

        Ok(Self { _file: file })
    }
}

/// The quick-prd pipeline driver.
pub struct QuickPrdPipeline {
    writer: Arc<dyn Backend>,
    reviewer: Arc<dyn Backend>,
    options: QuickPrdOptions,
}

impl QuickPrdPipeline {
    pub fn new(
        writer: Arc<dyn Backend>,
        reviewer: Arc<dyn Backend>,
        options: QuickPrdOptions,
    ) -> Self {
        Self {
            writer,
            reviewer,
            options,
        }
    }

    pub async fn run(self) -> Result<QuickPrdResult> {
        self.run_in(std::env::current_dir()?).await
    }

    async fn run_in(self, working_dir: PathBuf) -> Result<QuickPrdResult> {
        let started_at = now_iso8601();
        let idea_hash = sha256_hex(&self.options.idea)[..12].to_owned();

        // Create cache directory and acquire lock
        let cache_dir = working_dir
            .join(".ralph")
            .join("quick-prd")
            .join(&idea_hash);
        fs::create_dir_all(&cache_dir)?;
        let lock_path = cache_dir.join(".lock");
        let _lock = QuickPrdLock::acquire(&lock_path)?;

        // --- Draft step ---
        let draft_prompt = render_prompt(DRAFT_PROMPT, &[("{{idea}}", &self.options.idea)]);
        let draft_start = Instant::now();
        let mut current_spec = self.run_draft_with_section_retry(&draft_prompt).await?;
        let draft_time_secs = draft_start.elapsed().as_secs_f64();

        // Cache draft
        fs::write(cache_dir.join("draft.md"), &current_spec)?;

        // --- Review/Revision loop ---
        let mut review_times_secs = Vec::new();
        let mut revision_times_secs = Vec::new();
        let mut approved = false;
        let mut revision_count: u32 = 0;

        for n in 1..=self.options.max_revisions {
            // Build review prompt
            let review_prompt = render_prompt(
                REVIEW_PROMPT,
                &[
                    ("{{idea}}", &self.options.idea),
                    ("{{spec}}", &current_spec),
                ],
            );

            // Run review with retry
            let review_start = Instant::now();
            let feedback = run_review_with_retry(self.reviewer.clone(), review_prompt).await?;
            review_times_secs.push(review_start.elapsed().as_secs_f64());

            // Cache review
            let review_json = serde_json::to_string_pretty(&feedback)?;
            fs::write(cache_dir.join(format!("review-{n}.json")), &review_json)?;

            // Check approval (treat approved:false with empty issues as approved)
            if feedback.approved || feedback.issues.is_empty() {
                approved = true;
                break;
            }

            // Build revision prompt
            let formatted_issues = format_issues(&feedback.issues);
            let revision_prompt = render_prompt(
                REVISION_PROMPT,
                &[
                    ("{{idea}}", &self.options.idea),
                    ("{{spec}}", &current_spec),
                    ("{{issues}}", &formatted_issues),
                ],
            );

            // Run revision
            let revision_start = Instant::now();
            let revised = self.writer.execute(&revision_prompt).await?;
            revision_times_secs.push(revision_start.elapsed().as_secs_f64());

            // Section-check revision output
            let (cleaned, _missing) = check_spec_sections(&revised);
            current_spec = cleaned;

            // Cache revision
            fs::write(cache_dir.join(format!("revision-{n}.md")), &current_spec)?;
            revision_count = n;
        }

        // --- Finalization ---
        // Write the final spec inside the cache directory (under .ralph/) so it
        // never pollutes the repo root. Previously this wrote to working_dir/SPEC.md
        // which could get committed by `git add -A` if cleanup was interrupted.
        let spec_path = cache_dir.join("SPEC.md");
        fs::write(&spec_path, &current_spec)?;

        let meta = QuickPrdMeta {
            idea: self.options.idea.clone(),
            idea_hash,
            writer_backend: self.options.writer_spec.clone(),
            reviewer_backend: self.options.reviewer_spec.clone(),
            started_at,
            completed_at: now_iso8601(),
            revision_count,
            approved,
            draft_time_secs,
            review_times_secs,
            revision_times_secs,
        };
        let meta_json = serde_json::to_string_pretty(&meta)?;
        fs::write(cache_dir.join("meta.json"), format!("{meta_json}\n"))?;

        let summary = if approved {
            format!(
                "Quick PRD completed: approved after {} revision(s)",
                revision_count
            )
        } else {
            format!(
                "Quick PRD completed: NOT approved after {} revision(s) (max revisions exhausted)",
                revision_count
            )
        };

        Ok(QuickPrdResult {
            spec_path,
            cache_dir,
            revision_count,
            approved,
            summary,
        })
    }

    /// Runs the draft step with up to MAX_SECTION_RETRIES retries for missing sections.
    async fn run_draft_with_section_retry(&self, prompt: &str) -> Result<String> {
        for attempt in 0..=MAX_SECTION_RETRIES {
            let raw = self.writer.execute(prompt).await?;
            let (cleaned, missing) = check_spec_sections(&raw);

            if missing.is_empty() || attempt == MAX_SECTION_RETRIES {
                return Ok(cleaned);
            }
        }

        unreachable!("loop should return before reaching this point")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_spec() -> &'static str {
        "## Summary\nBody\n## Acceptance Criteria\nBody\n## Technical Approach\nBody\n## Files & Modules\nBody\n## Testing Strategy\nBody\n## Out of Scope\nBody"
    }

    #[test]
    fn test_render_prompt() {
        let template = "Hello {{name}}, role={{role}}";
        let rendered = render_prompt(template, &[("{{name}}", "Alice"), ("{{role}}", "writer")]);
        assert_eq!(rendered, "Hello Alice, role=writer");
    }

    #[test]
    fn test_dry_run_renders_prompt() {
        let idea = "add retry logic to backend execute()";
        let rendered = render_prompt(DRAFT_PROMPT, &[("{{idea}}", idea)]);
        assert!(rendered.contains(idea));
    }

    #[test]
    fn test_check_spec_sections_all_present() {
        let (cleaned, missing) = check_spec_sections(valid_spec());
        assert_eq!(cleaned, valid_spec());
        assert!(missing.is_empty());
    }

    #[test]
    fn test_check_spec_sections_some_missing() {
        let raw = "## Summary\nBody\n## Acceptance Criteria\nBody\n## Testing Strategy\nBody";
        let (_, missing) = check_spec_sections(raw);
        assert_eq!(
            missing,
            vec![
                "## Technical Approach".to_string(),
                "## Files & Modules".to_string(),
                "## Out of Scope".to_string(),
            ]
        );
    }

    #[test]
    fn test_check_spec_sections_with_frontmatter() {
        let raw = format!("---\nartifact: spec\n---\n{}", valid_spec());
        let (cleaned, missing) = check_spec_sections(&raw);
        assert_eq!(cleaned, valid_spec());
        assert!(missing.is_empty());
    }

    #[test]
    fn test_parse_review_feedback_approved() {
        let raw = "```json\n{\"approved\": true, \"issues\": []}\n```";
        let parsed = parse_review_feedback(raw).expect("feedback should parse");
        assert!(parsed.approved);
        assert!(parsed.issues.is_empty());
    }

    #[test]
    fn test_parse_review_feedback_rejected() {
        let raw = "prefix\n```json\n{\"approved\": false, \"issues\": [{\"area\": \"testing\", \"feedback\": \"add integration test\"}]}\n```\nsuffix";
        let parsed = parse_review_feedback(raw).expect("feedback should parse");
        assert!(!parsed.approved);
        assert_eq!(parsed.issues.len(), 1);
        assert_eq!(parsed.issues[0].area, "testing");
        assert_eq!(parsed.issues[0].feedback, "add integration test");
    }

    #[test]
    fn test_parse_review_feedback_malformed() {
        let raw = "{\"approved\": true, \"issues\": []}";
        assert!(parse_review_feedback(raw).is_err());
    }

    #[test]
    fn test_review_feedback_serde_roundtrip() {
        let feedback = ReviewFeedback {
            approved: false,
            issues: vec![
                ReviewIssue {
                    area: "feasibility".to_string(),
                    feedback: "clarify migration strategy".to_string(),
                },
                ReviewIssue {
                    area: "testing".to_string(),
                    feedback: "define failure-mode tests".to_string(),
                },
            ],
        };

        let json = serde_json::to_string(&feedback).unwrap();
        let roundtrip: ReviewFeedback = serde_json::from_str(&json).unwrap();
        assert_eq!(feedback, roundtrip);
    }

    #[test]
    fn test_format_issues() {
        let issues = vec![
            ReviewIssue {
                area: "acceptance criteria".to_string(),
                feedback: "add timeout behavior".to_string(),
            },
            ReviewIssue {
                area: "technical approach".to_string(),
                feedback: "include rollback strategy".to_string(),
            },
        ];

        let formatted = format_issues(&issues);
        assert_eq!(
            formatted,
            "1. acceptance criteria: add timeout behavior\n2. technical approach: include rollback strategy"
        );
    }

    // --- Async pipeline tests ---

    use crate::backend::MockBackend;

    fn mock_approved_review() -> String {
        "```json\n{\"approved\": true, \"issues\": []}\n```".to_string()
    }

    fn mock_rejected_review() -> String {
        "```json\n{\"approved\": false, \"issues\": [{\"area\": \"testing\", \"feedback\": \"add edge case tests\"}]}\n```".to_string()
    }

    #[tokio::test]
    async fn test_review_parse_retry_success() {
        // First response is malformed, second is valid
        let backend = Arc::new(MockBackend::new(
            "reviewer",
            vec!["no json here".to_string(), mock_approved_review()],
        ));

        let feedback = run_review_with_retry(backend.clone(), "review this".to_string())
            .await
            .expect("should succeed on retry");
        assert!(feedback.approved);
        assert!(feedback.issues.is_empty());
        assert_eq!(backend.call_count().await, 2);
    }

    #[tokio::test]
    async fn test_review_parse_retry_exhaustion() {
        // All 3 responses are malformed
        let backend = Arc::new(MockBackend::new(
            "reviewer",
            vec!["bad1".to_string(), "bad2".to_string(), "bad3".to_string()],
        ));

        let err = run_review_with_retry(backend.clone(), "review this".to_string())
            .await
            .expect_err("should fail after 3 attempts");
        assert!(matches!(err, RalphError::QuickPrdFailed(_)));
        assert_eq!(backend.call_count().await, 3);
    }

    #[tokio::test]
    async fn test_empty_issues_auto_approval() {
        // approved: false but empty issues → treated as approved
        let reviewer_response = "```json\n{\"approved\": false, \"issues\": []}\n```".to_string();
        let writer = Arc::new(MockBackend::new("writer", vec![valid_spec().to_string()]));
        let reviewer = Arc::new(MockBackend::new("reviewer", vec![reviewer_response]));

        let temp = tempfile::TempDir::new().unwrap();
        let working_dir = temp.path().to_path_buf();

        let options = QuickPrdOptions {
            idea: "test idea".to_string(),
            writer_spec: "mock-writer".to_string(),
            reviewer_spec: "mock-reviewer".to_string(),
            max_revisions: 2,
            dry_run: false,
        };

        let pipeline = QuickPrdPipeline::new(writer, reviewer, options);
        let result = pipeline
            .run_in(working_dir.clone())
            .await
            .expect("pipeline should succeed");

        assert!(result.approved);
        assert_eq!(result.revision_count, 0);
    }

    #[tokio::test]
    async fn test_revision_artifact_writing() {
        let writer = Arc::new(MockBackend::new(
            "writer",
            vec![
                valid_spec().to_string(), // draft
                valid_spec().to_string(), // revision-1
            ],
        ));
        let reviewer = Arc::new(MockBackend::new(
            "reviewer",
            vec![
                mock_rejected_review(), // review-1 (rejected)
                mock_approved_review(), // review-2 (approved)
            ],
        ));

        let temp = tempfile::TempDir::new().unwrap();
        let working_dir = temp.path().to_path_buf();

        let options = QuickPrdOptions {
            idea: "revision test".to_string(),
            writer_spec: "mock-writer".to_string(),
            reviewer_spec: "mock-reviewer".to_string(),
            max_revisions: 3,
            dry_run: false,
        };

        let pipeline = QuickPrdPipeline::new(writer, reviewer, options);
        let result = pipeline
            .run_in(working_dir.clone())
            .await
            .expect("pipeline should succeed");

        assert!(result.approved);
        assert_eq!(result.revision_count, 1);
        assert!(result.cache_dir.join("draft.md").exists());
        assert!(result.cache_dir.join("review-1.json").exists());
        assert!(result.cache_dir.join("revision-1.md").exists());
        assert!(result.cache_dir.join("review-2.json").exists());
        assert!(result.cache_dir.join("SPEC.md").exists());
        assert!(!working_dir.join("SPEC.md").exists(), "SPEC.md should not be in repo root");
        assert!(result.cache_dir.join("meta.json").exists());
    }

    #[tokio::test]
    async fn test_section_retry_limit() {
        // Writer returns incomplete spec all 3 times (1 initial + 2 retries)
        let incomplete = "## Summary\nBody\n## Acceptance Criteria\nBody";
        let writer = Arc::new(MockBackend::new(
            "writer",
            vec![
                incomplete.to_string(),
                incomplete.to_string(),
                incomplete.to_string(),
            ],
        ));
        let reviewer = Arc::new(MockBackend::new("reviewer", vec![mock_approved_review()]));

        let temp = tempfile::TempDir::new().unwrap();
        let working_dir = temp.path().to_path_buf();

        let options = QuickPrdOptions {
            idea: "section retry".to_string(),
            writer_spec: "mock-writer".to_string(),
            reviewer_spec: "mock-reviewer".to_string(),
            max_revisions: 1,
            dry_run: false,
        };

        let pipeline = QuickPrdPipeline::new(writer.clone(), reviewer, options);
        let result = pipeline
            .run_in(working_dir.clone())
            .await
            .expect("pipeline should succeed with best-effort");

        // Writer called 3 times for draft (1 + 2 retries)
        // Even though sections missing, pipeline proceeds best-effort
        assert_eq!(writer.call_count().await, 3);
        assert!(result.cache_dir.join("SPEC.md").exists());
        assert!(!working_dir.join("SPEC.md").exists(), "SPEC.md should not be in repo root");
        assert!(result.approved);
    }
}
