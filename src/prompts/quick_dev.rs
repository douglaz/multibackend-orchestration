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

pub fn build_quick_dev_final_review_prompt(
    template_path: &Path,
    vars: &BTreeMap<String, String>,
) -> Result<String> {
    render_template_with_fallback(
        template_path,
        vars,
        default_quick_dev_final_review_template(),
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

CRITICAL FORMAT REQUIREMENTS:
- Return markdown body only (no YAML frontmatter)
- Your response MUST begin with one exact, case-sensitive H1 as the VERY FIRST LINE:
  - `# Review: SATISFIED`
  - `# Review: CHANGES REQUESTED`
- No preamble or commentary before the H1

If satisfied, explain why briefly and confirm the implementation is ready.
If changes are required, provide concrete, actionable fixes.

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

fn default_quick_dev_final_review_template() -> &'static str {
    r#"You are performing a quick-dev final review pass. Verify whether the implementation is fully complete.

CRITICAL FORMAT REQUIREMENTS:
- Return markdown body only (no YAML frontmatter)
- Your response MUST begin with one exact, case-sensitive H1 as the VERY FIRST LINE:
  - `# Final Review: COMPLETE`
  - `# Final Review: ISSUES FOUND`
- No preamble or commentary before the H1

If complete, briefly confirm all requirements are met.
If issues are found, list precise blocking issues.

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
