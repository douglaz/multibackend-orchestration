//! Unit tests for template rendering functionality.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use ralph::prompts::templates::{
    default_completer_template, default_implementer_template, default_planner_template,
    default_prompt_reviewer_template, default_reviewer_template, render_template,
    render_template_with_fallback,
};
use tempfile::TempDir;

#[test]
fn test_render_template_replaces_single_variable() {
    let temp_dir = TempDir::new().unwrap();
    let template_path = temp_dir.path().join("test.md");
    fs::write(&template_path, "Hello, {{name}}!").unwrap();

    let mut vars = BTreeMap::new();
    vars.insert("name".to_owned(), "World".to_owned());

    let result = render_template(&template_path, &vars).unwrap();
    assert_eq!(result, "Hello, World!");
}

#[test]
fn test_render_template_replaces_multiple_variables() {
    let temp_dir = TempDir::new().unwrap();
    let template_path = temp_dir.path().join("test.md");
    fs::write(
        &template_path,
        "Project: {{project_id}}\nLoop: {{loop_number}}\nPhase: {{phase}}",
    )
    .unwrap();

    let mut vars = BTreeMap::new();
    vars.insert("project_id".to_owned(), "issue-1".to_owned());
    vars.insert("loop_number".to_owned(), "3".to_owned());
    vars.insert("phase".to_owned(), "reviewing".to_owned());

    let result = render_template(&template_path, &vars).unwrap();
    assert_eq!(result, "Project: issue-1\nLoop: 3\nPhase: reviewing");
}

#[test]
fn test_render_template_preserves_unmatched_variables() {
    let temp_dir = TempDir::new().unwrap();
    let template_path = temp_dir.path().join("test.md");
    fs::write(&template_path, "{{known}} and {{unknown}}").unwrap();

    let mut vars = BTreeMap::new();
    vars.insert("known".to_owned(), "replaced".to_owned());

    let result = render_template(&template_path, &vars).unwrap();
    assert_eq!(result, "replaced and {{unknown}}");
}

#[test]
fn test_render_template_empty_vars() {
    let temp_dir = TempDir::new().unwrap();
    let template_path = temp_dir.path().join("test.md");
    fs::write(&template_path, "Static content only").unwrap();

    let vars = BTreeMap::new();
    let result = render_template(&template_path, &vars).unwrap();
    assert_eq!(result, "Static content only");
}

#[test]
fn test_render_template_file_not_found() {
    let result = render_template(Path::new("/nonexistent/template.md"), &BTreeMap::new());
    assert!(result.is_err());
}

#[test]
fn test_default_planner_template_contains_required_sections() {
    let template = default_planner_template();

    // Check for key structural elements
    assert!(template.contains("# Feature: <name>"));
    assert!(template.contains("## Description"));
    assert!(template.contains("## Acceptance Criteria"));
    assert!(template.contains("## Files to Modify/Create"));
    assert!(template.contains("## Dependencies"));
    assert!(template.contains("# Project Completion Request"));
    assert!(template.contains("## Rationale"));

    // Check for template variables
    assert!(template.contains("{{master_prompt}}"));
    assert!(template.contains("{{state_json}}"));
    assert!(template.contains("{{previous_specs}}"));
}

#[test]
fn test_default_implementer_template_contains_required_sections() {
    let template = default_implementer_template();

    // Check for initial implementation format
    assert!(template.contains("# Implementation Notes"));
    assert!(template.contains("## Decisions Made"));
    assert!(template.contains("## Spec Deviations"));
    assert!(template.contains("## Testing"));

    // Check for review response format
    assert!(template.contains("# Implementation Response (Iteration {{iteration}})"));
    assert!(template.contains("## Changes Made"));
    assert!(template.contains("## Could Not Address"));

    // Check for template variables
    assert!(template.contains("{{feature_spec}}"));
    assert!(template.contains("{{review_feedback}}"));
    assert!(template.contains("{{review_history}}"));
    assert!(template.contains("{{master_prompt}}"));
    assert!(template.contains("{{current_diff}}"));
}

#[test]
fn test_default_reviewer_template_contains_required_sections() {
    let template = default_reviewer_template();

    // Check for approval format
    assert!(template.contains("# Review: APPROVED"));
    assert!(template.contains("## Acceptance Criteria Checklist"));
    assert!(template.contains("## Commit Message"));

    // Check for suggestions format
    assert!(template.contains("# Review: SUGGESTIONS"));
    assert!(template.contains("## Required Changes"));
    assert!(template.contains("## Recommended Improvements"));

    // Check for template variables
    assert!(template.contains("{{master_prompt}}"));
    assert!(template.contains("{{feature_spec}}"));
    assert!(template.contains("{{implementation_notes}}"));
    assert!(template.contains("{{current_diff}}"));
    assert!(template.contains("{{review_history}}"));
}

#[test]
fn test_default_completer_template_contains_required_sections() {
    let template = default_completer_template();

    // Check for complete verdict format
    assert!(template.contains("# Verdict: COMPLETE"));

    // Check for continue verdict format
    assert!(template.contains("# Verdict: CONTINUE"));
    assert!(template.contains("## Missing Requirements"));
    assert!(template.contains("## Recommended Next Features"));

    // Check for template variables
    assert!(template.contains("{{master_prompt}}"));
    assert!(template.contains("{{state_json}}"));
    assert!(template.contains("{{prior_specs}}"));
    assert!(template.contains("{{completion_request}}"));
}

#[test]
fn test_default_prompt_reviewer_template_contains_required_sections() {
    let template = default_prompt_reviewer_template();
    assert!(template.starts_with("You are a prompt reviewer"));
    assert!(template.contains("# Prompt Review"));
    assert!(template.contains("## Issues Found"));
    assert!(template.contains("## Refined Prompt"));
    assert!(template.contains("{{prompt_content}}"));
}

#[test]
fn test_render_template_with_fallback_uses_default_when_file_missing() {
    let missing = Path::new("/nonexistent/template.md");
    let mut vars = BTreeMap::new();
    vars.insert("name".to_owned(), "World".to_owned());

    let result = render_template_with_fallback(missing, &vars, "Hello, {{name}}!").unwrap();
    assert_eq!(result, "Hello, World!");
}

#[test]
fn test_render_template_with_fallback_prefers_file_when_present() {
    let temp_dir = TempDir::new().unwrap();
    let template_path = temp_dir.path().join("test.md");
    fs::write(&template_path, "From file: {{name}}").unwrap();

    let mut vars = BTreeMap::new();
    vars.insert("name".to_owned(), "World".to_owned());

    let result =
        render_template_with_fallback(&template_path, &vars, "From fallback: {{name}}").unwrap();
    assert_eq!(result, "From file: World");
}
