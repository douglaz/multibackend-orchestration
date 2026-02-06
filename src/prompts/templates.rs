use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use crate::Result;

pub fn render_template(path: &Path, vars: &BTreeMap<String, String>) -> Result<String> {
    let mut template = fs::read_to_string(path)?;
    for (key, value) in vars {
        let needle = format!("{{{{{key}}}}}");
        template = template.replace(&needle, value);
    }
    Ok(template)
}

pub fn default_planner_template() -> &'static str {
    r#"You are a software architect planning features for a project.

Given `prompt.md` and `state.json`, determine the next feature.
Return markdown body only. Use one of these H1 values:
- `# Feature: <name>`
- `# Project Completion Request`
"#
}

pub fn default_implementer_template() -> &'static str {
    r#"You are a software implementer.
Return markdown body only, no YAML frontmatter.
"#
}

pub fn default_reviewer_template() -> &'static str {
    r#"You are a reviewer. Return markdown body only.
Use one of these H1 values:
- `# Review: APPROVED`
- `# Review: SUGGESTIONS`
"#
}

pub fn default_completer_template() -> &'static str {
    r#"You are a completion validator. Return markdown body only.
Use one of:
- `# Verdict: COMPLETE`
- `# Verdict: CONTINUE`
"#
}
