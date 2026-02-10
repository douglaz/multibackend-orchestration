//! PRD stage prompt builders and deterministic output parsers.

use std::collections::BTreeMap;

use crate::workflow::parser::strip_frontmatter;

use super::state::Stage;

// ============================================================================
// Embedded Stage Prompt Templates
// ============================================================================

const IDEATION_PROMPT: &str = r#"You are a product ideation specialist. Generate comprehensive ideation content for the following product idea.

**Product Idea:**
{{idea}}

**User-Provided Context:**
{{answers}}

**Required Output Format:**
Your response must be a markdown document with the following exact section headings:

## Core Concept
## Target Users
## Key Problems Solved
## Proposed Features
## Success Metrics
## Constraints & Assumptions

Each section should be substantive and specific to the product idea.
"#;

const RESEARCH_PROMPT: &str = r#"You are a technical research analyst. Conduct research for the following product idea.

**Product Idea:**
{{idea}}

**User-Provided Context:**
{{answers}}

**Ideation Output:**
{{ideation}}

**Required Output Format:**
Your response must be a markdown document with the following exact section headings:

## Market Context
## Technical Landscape
## Comparable Solutions
## Technical Feasibility
## Risk Assessment

Each section should be substantive and specific to the product idea.
"#;

const SYNTHESIS_PROMPT: &str = r#"You are a product strategist. Synthesize ideation and research into a coherent product strategy.

**Product Idea:**
{{idea}}

**User-Provided Context:**
{{answers}}

**Ideation Output:**
{{ideation}}

**Research Output:**
{{research}}

**Required Output Format:**
Your response must be a markdown document with the following exact section headings:

## Product Vision
## User Stories
## Feature Prioritization
## Architecture Overview
## MVP Scope
## Open Questions

Each section should be substantive and specific to the product idea.
"#;

const PRD_PROMPT: &str = r#"You are a technical product manager. Generate a comprehensive Product Requirements Document (PRD) for the following product idea.

**Product Idea:**
{{idea}}

**User-Provided Context:**
{{answers}}

**Ideation Output:**
{{ideation}}

**Research Output:**
{{research}}

**Synthesis Output:**
{{synthesis}}

**Required Output Format:**
Your response must be a markdown document with the following exact section headings:

## Executive Summary
## Goals & Non-Goals
## User Stories
## Functional Requirements
## Non-Functional Requirements
## Technical Architecture
## Data Model
## API Design
## Security Considerations
## Testing Strategy
## Rollout Plan
## Success Metrics
## Open Questions

Each section should be substantive and specific to the product idea.
"#;

const GAP_ANALYSIS_PROMPT: &str = r#"You are a requirements analyst. Analyze the following {{stage_name}} output for information gaps, missing fields, ambiguities, and questions that need clarification.

**Product Idea:**
{{idea}}

**User-Provided Context:**
{{answers}}

**{{stage_name}} Output:**
{{stage_output}}

**Task:**
Identify any missing information, ambiguities, or questions that should be asked to the user to improve the {{stage_name}} output.

**Required Output Format:**
Your response MUST be a single fenced JSON block in the following format:

```json
{
  "missing_fields": [
    {"field": "field_name", "description": "what information is missing"}
  ],
  "ambiguities": [
    {"area": "area_name", "description": "what is unclear or ambiguous"}
  ],
  "questions": [
    {
      "key": "unique_question_key",
      "prompt": "Question to ask the user?",
      "kind": "FreeText",
      "suggested_default": null,
      "impact_stage": "{{impact_stage}}"
    }
  ],
  "suggested_defaults": [
    {"key": "default_key", "value": "default_value", "rationale": "why this default"}
  ]
}
```

Valid values for `kind`: "FreeText", {"Choice": ["option1", "option2"]}, "YesNo"
Valid values for `impact_stage`: "Ideation", "Research", "Synthesis", "Prd"

If there are no gaps, return an empty structure with empty arrays for all fields.
"#;

const VALIDATION_PROMPT: &str = r#"You are a PRD reviewer. Validate the following final PRD for completeness and clarity.

**Product Idea:**
{{idea}}

**User-Provided Context:**
{{answers}}

**Final PRD:**
{{prd}}

**Task:**
Review the PRD and determine if it is complete and ready for implementation. Check that all required sections are present, substantive, and free of critical gaps.

**Required Output Format:**
Your response MUST be a single fenced JSON block in the following format:

```json
{
  "valid": true,
  "issues": []
}
```

If the PRD has issues, set `valid` to `false` and list each issue:

```json
{
  "valid": false,
  "issues": [
    {"field": "field_name", "description": "what is missing or unclear"}
  ]
}
```
"#;

// ============================================================================
// Required Sections Per Stage
// ============================================================================

fn required_sections(stage: Stage) -> &'static [&'static str] {
    match stage {
        Stage::Ideation => &[
            "## Core Concept",
            "## Target Users",
            "## Key Problems Solved",
            "## Proposed Features",
            "## Success Metrics",
            "## Constraints & Assumptions",
        ],
        Stage::Research => &[
            "## Market Context",
            "## Technical Landscape",
            "## Comparable Solutions",
            "## Technical Feasibility",
            "## Risk Assessment",
        ],
        Stage::Synthesis => &[
            "## Product Vision",
            "## User Stories",
            "## Feature Prioritization",
            "## Architecture Overview",
            "## MVP Scope",
            "## Open Questions",
        ],
        Stage::Prd => &[
            "## Executive Summary",
            "## Goals & Non-Goals",
            "## User Stories",
            "## Functional Requirements",
            "## Non-Functional Requirements",
            "## Technical Architecture",
            "## Data Model",
            "## API Design",
            "## Security Considerations",
            "## Testing Strategy",
            "## Rollout Plan",
            "## Success Metrics",
            "## Open Questions",
        ],
    }
}

// ============================================================================
// Prompt Builder
// ============================================================================

/// Builder for stage prompts with dependency injection.
pub struct StagePromptBuilder {
    idea: String,
    answers: BTreeMap<String, String>,
    stage_outputs: BTreeMap<Stage, String>,
}

impl StagePromptBuilder {
    /// Creates a new prompt builder with the given idea, answers, and prior stage outputs.
    pub fn new(
        idea: String,
        answers: BTreeMap<String, String>,
        stage_outputs: BTreeMap<Stage, String>,
    ) -> Self {
        Self {
            idea,
            answers,
            stage_outputs,
        }
    }

    /// Builds the prompt for a specific stage, injecting prior stage outputs as dependencies.
    pub fn build_stage_prompt(&self, stage: Stage) -> String {
        let template = match stage {
            Stage::Ideation => IDEATION_PROMPT,
            Stage::Research => RESEARCH_PROMPT,
            Stage::Synthesis => SYNTHESIS_PROMPT,
            Stage::Prd => PRD_PROMPT,
        };

        let answers_formatted = self.format_answers();

        let mut replacements = vec![
            ("{{idea}}", self.idea.as_str()),
            ("{{answers}}", answers_formatted.as_str()),
        ];

        // Inject prior stage outputs based on stage dependency rules
        let ideation = self.stage_outputs.get(&Stage::Ideation).map(|s| s.as_str());
        let research = self.stage_outputs.get(&Stage::Research).map(|s| s.as_str());
        let synthesis = self
            .stage_outputs
            .get(&Stage::Synthesis)
            .map(|s| s.as_str());

        match stage {
            Stage::Ideation => {
                // Ideation gets no prior outputs
            }
            Stage::Research => {
                // Research gets Ideation
                replacements.push(("{{ideation}}", ideation.unwrap_or("(not yet available)")));
            }
            Stage::Synthesis => {
                // Synthesis gets Ideation + Research
                replacements.push(("{{ideation}}", ideation.unwrap_or("(not yet available)")));
                replacements.push(("{{research}}", research.unwrap_or("(not yet available)")));
            }
            Stage::Prd => {
                // PRD gets all three
                replacements.push(("{{ideation}}", ideation.unwrap_or("(not yet available)")));
                replacements.push(("{{research}}", research.unwrap_or("(not yet available)")));
                replacements.push(("{{synthesis}}", synthesis.unwrap_or("(not yet available)")));
            }
        }

        render_template(template, &replacements)
    }

    /// Builds the gap analysis prompt for a specific stage output.
    pub fn build_gap_analysis_prompt(&self, stage: Stage, stage_output: &str) -> String {
        let stage_name = format!("{:?}", stage);
        let impact_stage = format!("{:?}", stage);
        let answers_formatted = self.format_answers();

        render_template(
            GAP_ANALYSIS_PROMPT,
            &[
                ("{{stage_name}}", stage_name.as_str()),
                ("{{idea}}", self.idea.as_str()),
                ("{{answers}}", answers_formatted.as_str()),
                ("{{stage_output}}", stage_output),
                ("{{impact_stage}}", impact_stage.as_str()),
            ],
        )
    }

    /// Builds the validation prompt for the final PRD.
    pub fn build_validation_prompt(&self, prd: &str) -> String {
        let answers_formatted = self.format_answers();

        render_template(
            VALIDATION_PROMPT,
            &[
                ("{{idea}}", self.idea.as_str()),
                ("{{answers}}", answers_formatted.as_str()),
                ("{{prd}}", prd),
            ],
        )
    }

    fn format_answers(&self) -> String {
        if self.answers.is_empty() {
            "(none provided)".to_string()
        } else {
            self.answers
                .iter()
                .map(|(k, v)| format!("- {}: {}", k, v))
                .collect::<Vec<_>>()
                .join("\n")
        }
    }
}

// ============================================================================
// Template Rendering
// ============================================================================

/// Simple inline placeholder replacement (no template files).
fn render_template(template: &str, replacements: &[(&str, &str)]) -> String {
    let mut result = template.to_string();
    for (placeholder, value) in replacements {
        result = result.replace(placeholder, value);
    }
    result
}

// ============================================================================
// Deterministic Output Checking
// ============================================================================

/// Result of checking a stage output for required sections.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StageOutputCheck {
    /// The cleaned output (frontmatter stripped).
    pub cleaned_output: String,
    /// List of missing required section headings.
    pub missing_sections: Vec<String>,
}

/// Checks a stage output for required sections after stripping frontmatter.
/// Returns a structured result with cleaned output and missing sections.
pub fn check_stage_output(stage: Stage, raw_output: &str) -> StageOutputCheck {
    let cleaned = strip_frontmatter(raw_output);
    let required = required_sections(stage);

    let mut missing_sections = Vec::new();
    for &section in required {
        if !cleaned.lines().any(|line| line.trim() == section) {
            missing_sections.push(section.to_string());
        }
    }

    StageOutputCheck {
        cleaned_output: cleaned,
        missing_sections,
    }
}

// ============================================================================
// Unit Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_render_template() {
        let template = "Hello {{name}}, you are {{age}} years old.";
        let result = render_template(template, &[("{{name}}", "Alice"), ("{{age}}", "30")]);
        assert_eq!(result, "Hello Alice, you are 30 years old.");
    }

    #[test]
    fn test_ideation_prompt() {
        let mut answers = BTreeMap::new();
        answers.insert("platform".to_string(), "Web".to_string());

        let builder =
            StagePromptBuilder::new("Build a todo app".to_string(), answers, BTreeMap::new());

        let prompt = builder.build_stage_prompt(Stage::Ideation);

        // Verify it contains the idea and answers
        assert!(prompt.contains("Build a todo app"));
        assert!(prompt.contains("platform: Web"));

        // Verify it does NOT contain prior stage placeholders (Ideation has no deps)
        assert!(!prompt.contains("{{ideation}}"));
        assert!(!prompt.contains("{{research}}"));
        assert!(!prompt.contains("{{synthesis}}"));

        // Verify required sections are specified
        assert!(prompt.contains("## Core Concept"));
        assert!(prompt.contains("## Target Users"));
        assert!(prompt.contains("## Key Problems Solved"));
        assert!(prompt.contains("## Proposed Features"));
        assert!(prompt.contains("## Success Metrics"));
        assert!(prompt.contains("## Constraints & Assumptions"));
    }

    #[test]
    fn test_research_prompt() {
        let mut stage_outputs = BTreeMap::new();
        stage_outputs.insert(Stage::Ideation, "Ideation content here".to_string());

        let builder = StagePromptBuilder::new(
            "Build a todo app".to_string(),
            BTreeMap::new(),
            stage_outputs,
        );

        let prompt = builder.build_stage_prompt(Stage::Research);

        // Verify it contains the idea and Ideation output
        assert!(prompt.contains("Build a todo app"));
        assert!(prompt.contains("Ideation content here"));

        // Verify required sections
        assert!(prompt.contains("## Market Context"));
        assert!(prompt.contains("## Technical Landscape"));
        assert!(prompt.contains("## Comparable Solutions"));
        assert!(prompt.contains("## Technical Feasibility"));
        assert!(prompt.contains("## Risk Assessment"));
    }

    #[test]
    fn test_synthesis_prompt() {
        let mut stage_outputs = BTreeMap::new();
        stage_outputs.insert(Stage::Ideation, "Ideation content".to_string());
        stage_outputs.insert(Stage::Research, "Research content".to_string());

        let builder = StagePromptBuilder::new(
            "Build a todo app".to_string(),
            BTreeMap::new(),
            stage_outputs,
        );

        let prompt = builder.build_stage_prompt(Stage::Synthesis);

        // Verify it contains Ideation + Research
        assert!(prompt.contains("Ideation content"));
        assert!(prompt.contains("Research content"));

        // Verify required sections
        assert!(prompt.contains("## Product Vision"));
        assert!(prompt.contains("## User Stories"));
        assert!(prompt.contains("## Feature Prioritization"));
        assert!(prompt.contains("## Architecture Overview"));
        assert!(prompt.contains("## MVP Scope"));
        assert!(prompt.contains("## Open Questions"));
    }

    #[test]
    fn test_prd_prompt() {
        let mut stage_outputs = BTreeMap::new();
        stage_outputs.insert(Stage::Ideation, "Ideation content".to_string());
        stage_outputs.insert(Stage::Research, "Research content".to_string());
        stage_outputs.insert(Stage::Synthesis, "Synthesis content".to_string());

        let builder = StagePromptBuilder::new(
            "Build a todo app".to_string(),
            BTreeMap::new(),
            stage_outputs,
        );

        let prompt = builder.build_stage_prompt(Stage::Prd);

        // Verify it contains all three prior stages
        assert!(prompt.contains("Ideation content"));
        assert!(prompt.contains("Research content"));
        assert!(prompt.contains("Synthesis content"));

        // Verify required sections
        assert!(prompt.contains("## Executive Summary"));
        assert!(prompt.contains("## Goals & Non-Goals"));
        assert!(prompt.contains("## User Stories"));
        assert!(prompt.contains("## Functional Requirements"));
        assert!(prompt.contains("## Non-Functional Requirements"));
        assert!(prompt.contains("## Technical Architecture"));
        assert!(prompt.contains("## Data Model"));
        assert!(prompt.contains("## API Design"));
        assert!(prompt.contains("## Security Considerations"));
        assert!(prompt.contains("## Testing Strategy"));
        assert!(prompt.contains("## Rollout Plan"));
        assert!(prompt.contains("## Success Metrics"));
        assert!(prompt.contains("## Open Questions"));
    }

    #[test]
    fn test_gap_analysis_prompt() {
        let builder = StagePromptBuilder::new(
            "Build a todo app".to_string(),
            BTreeMap::new(),
            BTreeMap::new(),
        );

        let prompt = builder.build_gap_analysis_prompt(Stage::Ideation, "Some ideation output");

        // Verify fenced JSON instruction pattern
        assert!(prompt.contains("```json"));
        assert!(prompt.contains("missing_fields"));
        assert!(prompt.contains("ambiguities"));
        assert!(prompt.contains("questions"));
        assert!(prompt.contains("suggested_defaults"));

        // Verify stage name and output are injected
        assert!(prompt.contains("Ideation"));
        assert!(prompt.contains("Some ideation output"));

        // Verify impact_stage is documented
        assert!(prompt.contains("impact_stage"));
    }

    #[test]
    fn test_validation_prompt() {
        let builder = StagePromptBuilder::new(
            "Build a todo app".to_string(),
            BTreeMap::new(),
            BTreeMap::new(),
        );

        let prompt = builder.build_validation_prompt("Final PRD content");

        // Verify fenced JSON instruction pattern
        assert!(prompt.contains("```json"));
        assert!(prompt.contains(r#""valid""#));
        assert!(prompt.contains(r#""issues""#));

        // Verify PRD content is injected
        assert!(prompt.contains("Final PRD content"));
    }

    #[test]
    fn test_check_stage_output_valid() {
        let output = r#"
## Core Concept
Some content

## Target Users
Some content

## Key Problems Solved
Some content

## Proposed Features
Some content

## Success Metrics
Some content

## Constraints & Assumptions
Some content
"#;

        let check = check_stage_output(Stage::Ideation, output);
        assert!(check.missing_sections.is_empty());
        assert_eq!(check.cleaned_output, output.trim());
    }

    #[test]
    fn test_check_stage_output_missing_sections() {
        let output = r#"
## Core Concept
Some content

## Target Users
Some content
"#;

        let check = check_stage_output(Stage::Ideation, output);
        assert_eq!(check.missing_sections.len(), 4);
        assert!(check
            .missing_sections
            .contains(&"## Key Problems Solved".to_string()));
        assert!(check
            .missing_sections
            .contains(&"## Proposed Features".to_string()));
        assert!(check
            .missing_sections
            .contains(&"## Success Metrics".to_string()));
        assert!(check
            .missing_sections
            .contains(&"## Constraints & Assumptions".to_string()));
    }

    #[test]
    fn test_check_stage_output_with_frontmatter() {
        let output = r#"---
artifact: ideation
created_at: 2026-02-10T20:00:00Z
---

## Core Concept
Some content

## Target Users
Some content

## Key Problems Solved
Some content

## Proposed Features
Some content

## Success Metrics
Some content

## Constraints & Assumptions
Some content
"#;

        let check = check_stage_output(Stage::Ideation, output);
        // Frontmatter should be stripped, and all sections should be present
        assert!(check.missing_sections.is_empty());
        assert!(!check.cleaned_output.contains("---"));
        assert!(!check.cleaned_output.contains("artifact:"));
    }

    #[test]
    fn test_required_sections_per_stage() {
        assert_eq!(required_sections(Stage::Ideation).len(), 6);
        assert_eq!(required_sections(Stage::Research).len(), 5);
        assert_eq!(required_sections(Stage::Synthesis).len(), 6);
        assert_eq!(required_sections(Stage::Prd).len(), 13);
    }

    #[test]
    fn test_format_answers_empty() {
        let builder = StagePromptBuilder::new("idea".to_string(), BTreeMap::new(), BTreeMap::new());
        assert_eq!(builder.format_answers(), "(none provided)");
    }

    #[test]
    fn test_format_answers_with_data() {
        let mut answers = BTreeMap::new();
        answers.insert("platform".to_string(), "Web".to_string());
        answers.insert("target".to_string(), "Developers".to_string());

        let builder = StagePromptBuilder::new("idea".to_string(), answers, BTreeMap::new());
        let formatted = builder.format_answers();

        assert!(formatted.contains("- platform: Web"));
        assert!(formatted.contains("- target: Developers"));
    }
}
