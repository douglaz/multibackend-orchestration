use std::collections::BTreeMap;
use std::path::Path;

use crate::prompts::templates::render_template_with_fallback;
use crate::Result;

pub fn build_quick_dev_plan_implement_prompt(
    template_path: &Path,
    vars: &BTreeMap<String, String>,
) -> Result<String> {
    render_template_with_fallback(
        template_path,
        vars,
        default_quick_dev_plan_implement_template(),
    )
}

pub fn build_quick_dev_codex_review_prompt(
    template_path: &Path,
    vars: &BTreeMap<String, String>,
) -> Result<String> {
    render_template_with_fallback(
        template_path,
        vars,
        default_quick_dev_codex_review_template(),
    )
}

pub fn build_quick_dev_apply_fixes_prompt(
    template_path: &Path,
    vars: &BTreeMap<String, String>,
) -> Result<String> {
    render_template_with_fallback(
        template_path,
        vars,
        default_quick_dev_apply_fixes_template(),
    )
}

fn default_quick_dev_plan_implement_template() -> &'static str {
    r#"You are a software developer handling the quick-dev plan-and-implement phase.

Produce a practical implementation for the requested work, keeping scope tight and aligned to the specification.

CRITICAL FORMAT REQUIREMENTS:
- Return markdown body only (no YAML frontmatter)
- Your response MUST begin with an H1 as the VERY FIRST LINE
- No preamble or commentary before the H1

## Context Provided

## System Guardrails
{{system_guardrails}}

## Feature Specification
{{feature_spec}}

## Master Prompt
{{master_prompt}}

## Current Diff
{{current_diff}}
"#
}

fn default_quick_dev_codex_review_template() -> &'static str {
    r#"You are the quick-dev reviewer. Evaluate the implementation against the provided specification and current diff.

Review focus:
1. Does the implementation satisfy the spec requirements?
2. For new or modified functions: trace all callers in the diff. Verify the change is correct for every code path, not just the intended one. Flag over-broad integration (e.g., a function wired into a generic entry point when it should only run in specific contexts).
3. Are there logic errors, missing edge cases, or incorrect error handling?
4. Provide concrete, actionable fixes for any issues found.

CRITICAL FORMAT REQUIREMENTS:
- Return markdown body only (no YAML frontmatter)
- Your response MUST begin with one exact, case-sensitive H1 as the VERY FIRST LINE:
  - `# Review: SATISFIED`
  - `# Review: CHANGES REQUESTED`
- No preamble or commentary before the H1

If satisfied, explain why briefly and confirm the implementation is ready.
If changes are required, provide concrete, actionable fixes with file paths.

## Context Provided

## System Guardrails
{{system_guardrails}}

## Feature Specification
{{feature_spec}}

## Master Prompt
{{master_prompt}}

## Current Diff
{{current_diff}}
"#
}

fn default_quick_dev_apply_fixes_template() -> &'static str {
    r#"You are the implementer in quick-dev apply-fixes phase.

Apply the reviewer-requested changes exactly, minimizing unrelated edits.

CRITICAL FORMAT REQUIREMENTS:
- Return markdown body only (no YAML frontmatter)
- Your response MUST begin with an H1 as the VERY FIRST LINE
- No preamble or commentary before the H1

## Context Provided

## System Guardrails
{{system_guardrails}}

## Feature Specification
{{feature_spec}}

## Reviewer Feedback
{{review_feedback}}

## Master Prompt
{{master_prompt}}

## Current Diff
{{current_diff}}
"#
}
