use std::path::PathBuf;
use std::process::Command;

use serde_json::Value;

use crate::cli::{ConfigArgs, ConfigCommand};
use crate::config::{
    resolve_effective_config, CommitMessageStyle, ProjectConfig, PromptChangeAction,
    RunWorkflowOverrides,
};
use crate::project::load_project_config_if_exists;
use crate::util::lock::ProjectLock;
use crate::workspace::Workspace;
use crate::{error::RalphError, Result};

pub fn execute(args: ConfigArgs) -> Result<()> {
    let mut workspace = Workspace::discover()?;
    match args.command {
        ConfigCommand::Show(show_args) => {
            let scope = resolve_scope(
                &workspace,
                show_args.scope.global,
                show_args.scope.project.as_deref(),
            )?;
            execute_show(&workspace, &scope)
        }
        ConfigCommand::Get(get_args) => {
            let scope = resolve_scope(
                &workspace,
                get_args.scope.global,
                get_args.scope.project.as_deref(),
            )?;
            execute_get(&workspace, &scope, &get_args.key)
        }
        ConfigCommand::Set(set_args) => {
            let scope = resolve_scope(
                &workspace,
                set_args.scope.global,
                set_args.scope.project.as_deref(),
            )?;
            execute_set(&mut workspace, &scope, &set_args.key, &set_args.value)
        }
        ConfigCommand::Edit(edit_args) => {
            let scope = resolve_scope(
                &workspace,
                edit_args.scope.global,
                edit_args.scope.project.as_deref(),
            )?;
            execute_edit(&workspace, &scope)
        }
    }
}

#[derive(Debug, Clone)]
enum ConfigScope {
    Global,
    Project(String),
}

fn resolve_scope(
    workspace: &Workspace,
    global: bool,
    project: Option<&str>,
) -> Result<ConfigScope> {
    if global && project.is_some() {
        return Err(RalphError::Validation(
            "--global and --project are mutually exclusive".to_owned(),
        ));
    }

    if global {
        return Ok(ConfigScope::Global);
    }

    if let Some(project_id) = project {
        ensure_project_exists(workspace, project_id)?;
        return Ok(ConfigScope::Project(project_id.to_owned()));
    }

    if let Some(active) = workspace.index.active_project.clone() {
        return Ok(ConfigScope::Project(active));
    }

    Ok(ConfigScope::Global)
}

fn ensure_project_exists(workspace: &Workspace, project_id: &str) -> Result<()> {
    if workspace.index.get_project(project_id).is_none() {
        return Err(RalphError::ProjectNotFound(project_id.to_owned()));
    }
    Ok(())
}

fn execute_show(workspace: &Workspace, scope: &ConfigScope) -> Result<()> {
    match scope {
        ConfigScope::Global => {
            let value = serde_json::to_value(&workspace.config)?;
            println!("{}", serde_json::to_string_pretty(&value)?);
            Ok(())
        }
        ConfigScope::Project(project_id) => {
            let project_dir = workspace.project_dir(project_id);
            let project_config = load_project_config_if_exists(&project_dir)?;
            let effective = resolve_effective_config(
                &workspace.root,
                &project_dir,
                workspace.config.clone(),
                project_config.clone(),
                RunWorkflowOverrides::default(),
            )?;

            let value = serde_json::json!({
                "scope": {
                    "type": "project",
                    "project": project_id,
                },
                "workspace": effective.global.workspace,
                "backends": effective.global.backends,
                "workflow": {
                    "starting_backend": effective.workflow.starting_backend,
                    "prompt_review_enabled": effective.workflow.prompt_review_enabled,
                    "prompt_review_backend": effective.workflow.prompt_review_backend,
                    "planner_backend": effective.workflow.planner_backend,
                    "implementer_backend": effective.workflow.implementer_backend,
                    "reviewer_backend": effective.workflow.reviewer_backend,
                    "qa_backend": effective.workflow.qa_backend,
                    "completer_backend": effective.workflow.completer_backend,
                    "qa_enabled": effective.workflow.qa_enabled,
                    "max_qa_iterations": effective.workflow.max_qa_iterations,
                    "max_review_iterations": effective.workflow.max_review_iterations,
                    "auto_commit": effective.workflow.auto_commit,
                    "commit_message_style": effective.workflow.commit_message_style,
                    "commit_tag_format": effective.workflow.commit_tag_format,
                    "prompt_change_action": effective.workflow.prompt_change_action,
                },
                "templates": {
                    "planner": effective.templates.planner,
                    "implementer": effective.templates.implementer,
                    "reviewer": effective.templates.reviewer,
                    "prompt_reviewer": effective.templates.prompt_reviewer,
                    "completer": effective.templates.completer,
                    "qa": effective.templates.qa,
                },
                "git": effective.global.git,
                "project_overrides": project_config,
            });
            println!("{}", serde_json::to_string_pretty(&value)?);
            Ok(())
        }
    }
}

/// Map shorthand alias keys to their canonical dotted form.
fn resolve_config_alias(key: &str) -> &str {
    match key {
        "planner_backend" => "workflow.planner_backend",
        "qa_backend" => "workflow.qa_backend",
        _ => key,
    }
}

fn execute_get(workspace: &Workspace, scope: &ConfigScope, key: &str) -> Result<()> {
    let key = resolve_config_alias(key);
    let value = match scope {
        ConfigScope::Global => serde_json::to_value(&workspace.config)?,
        ConfigScope::Project(project_id) => {
            let project_dir = workspace.project_dir(project_id);
            let project_config = load_project_config_if_exists(&project_dir)?;
            let effective = resolve_effective_config(
                &workspace.root,
                &project_dir,
                workspace.config.clone(),
                project_config,
                RunWorkflowOverrides::default(),
            )?;

            serde_json::json!({
                "workspace": effective.global.workspace,
                "backends": effective.global.backends,
                "workflow": {
                    "starting_backend": effective.workflow.starting_backend,
                    "prompt_review_enabled": effective.workflow.prompt_review_enabled,
                    "prompt_review_backend": effective.workflow.prompt_review_backend,
                    "planner_backend": effective.workflow.planner_backend,
                    "implementer_backend": effective.workflow.implementer_backend,
                    "reviewer_backend": effective.workflow.reviewer_backend,
                    "qa_backend": effective.workflow.qa_backend,
                    "completer_backend": effective.workflow.completer_backend,
                    "qa_enabled": effective.workflow.qa_enabled,
                    "max_qa_iterations": effective.workflow.max_qa_iterations,
                    "max_review_iterations": effective.workflow.max_review_iterations,
                    "auto_commit": effective.workflow.auto_commit,
                    "commit_message_style": effective.workflow.commit_message_style,
                    "commit_tag_format": effective.workflow.commit_tag_format,
                    "prompt_change_action": effective.workflow.prompt_change_action,
                },
                "templates": {
                    "planner": effective.templates.planner,
                    "implementer": effective.templates.implementer,
                    "reviewer": effective.templates.reviewer,
                    "prompt_reviewer": effective.templates.prompt_reviewer,
                    "completer": effective.templates.completer,
                    "qa": effective.templates.qa,
                },
                "git": effective.global.git,
            })
        }
    };

    let selected = lookup_path(&value, key)?;
    print_selected_value(selected);
    Ok(())
}

fn execute_set(
    workspace: &mut Workspace,
    scope: &ConfigScope,
    key: &str,
    raw_value: &str,
) -> Result<()> {
    let key = resolve_config_alias(key);
    match scope {
        ConfigScope::Global => {
            set_global_value(&mut workspace.config, key, raw_value)?;
            workspace.save_config()?;
            println!("updated global config: {key}");
            Ok(())
        }
        ConfigScope::Project(project_id) => {
            let project_dir = workspace.project_dir(project_id);
            let _lock = ProjectLock::acquire(&project_dir, project_id)?;

            let mut project_cfg = load_project_config_if_exists(&project_dir)?.unwrap_or_default();
            set_project_value(&mut project_cfg, key, raw_value)?;
            project_cfg.save(&project_dir.join("config.toml"))?;
            println!("updated project config ({project_id}): {key}");
            Ok(())
        }
    }
}

fn execute_edit(workspace: &Workspace, scope: &ConfigScope) -> Result<()> {
    let editor = std::env::var("EDITOR").unwrap_or_else(|_| "vi".to_owned());

    let path = match scope {
        ConfigScope::Global => workspace.root.join("ralph.toml"),
        ConfigScope::Project(project_id) => {
            let project_dir = workspace.project_dir(project_id);
            ensure_project_exists(workspace, project_id)?;
            let path = project_dir.join("config.toml");
            if !path.exists() {
                ProjectConfig::default().save(&path)?;
            }
            path
        }
    };

    let status = Command::new(&editor).arg(&path).status().map_err(|err| {
        RalphError::Orchestration(format!("failed to launch editor '{editor}': {err}"))
    })?;

    if !status.success() {
        return Err(RalphError::Orchestration(format!(
            "editor '{editor}' exited with non-zero status"
        )));
    }

    println!("edited {}", path.display());
    Ok(())
}

fn print_selected_value(value: &Value) {
    match value {
        Value::String(s) => println!("{s}"),
        _ => println!(
            "{}",
            serde_json::to_string_pretty(value).unwrap_or_else(|_| "null".to_owned())
        ),
    }
}

fn lookup_path<'a>(value: &'a Value, key: &str) -> Result<&'a Value> {
    let mut current = value;
    for segment in key.split('.') {
        current = current
            .get(segment)
            .ok_or_else(|| RalphError::Validation(format!("config key not found: {key}")))?;
    }
    Ok(current)
}

fn set_global_value(
    config: &mut crate::config::GlobalConfig,
    key: &str,
    raw_value: &str,
) -> Result<()> {
    match key {
        "workspace.version" => config.workspace.version = raw_value.to_owned(),
        "workspace.default_backend" => {
            ensure_backend(raw_value)?;
            config.workspace.default_backend = raw_value.to_owned();
        }
        "workspace.tmux" => {
            config.workspace.tmux = parse_bool(raw_value, key)?;
        }
        "workspace.tmux_session" => {
            config.workspace.tmux_session = raw_value.to_owned();
        }
        "workspace.tmux_window_keep_seconds" => {
            config.workspace.tmux_window_keep_seconds = parse_u64(raw_value, key)?;
        }
        "workflow.max_review_iterations" => {
            config.workflow.max_review_iterations = parse_u32(raw_value, key)?;
        }
        "workflow.auto_commit" => {
            config.workflow.auto_commit = parse_bool(raw_value, key)?;
        }
        "workflow.commit_message_style" => {
            config.workflow.commit_message_style = parse_commit_message_style(raw_value)?;
        }
        "workflow.commit_tag_format" => {
            config.workflow.commit_tag_format = raw_value.to_owned();
        }
        "workflow.prompt_change_action" => {
            config.workflow.prompt_change_action = parse_prompt_change_action(raw_value)?;
        }
        "workflow.prompt_review_enabled" => {
            config.workflow.prompt_review_enabled = parse_bool(raw_value, key)?;
        }
        "workflow.prompt_review_backend" => {
            ensure_backend(raw_value)?;
            config.workflow.prompt_review_backend = raw_value.to_owned();
        }
        "workflow.planner_backend" => {
            config.workflow.planner_backend = parse_optional_backend(raw_value)?;
        }
        "workflow.implementer_backend" => {
            config.workflow.implementer_backend = parse_optional_backend(raw_value)?;
        }
        "workflow.reviewer_backend" => {
            config.workflow.reviewer_backend = parse_optional_backend(raw_value)?;
        }
        "workflow.qa_backend" => {
            config.workflow.qa_backend = parse_optional_backend(raw_value)?;
        }
        "workflow.completer_backend" => {
            config.workflow.completer_backend = parse_optional_backend(raw_value)?;
        }
        "workflow.qa_enabled" => {
            config.workflow.qa_enabled = parse_bool(raw_value, key)?;
        }
        "workflow.max_qa_iterations" => {
            config.workflow.max_qa_iterations = parse_u32(raw_value, key)?;
        }
        "templates.planner" => config.templates.planner = raw_value.to_owned(),
        "templates.implementer" => config.templates.implementer = raw_value.to_owned(),
        "templates.reviewer" => config.templates.reviewer = raw_value.to_owned(),
        "templates.prompt_reviewer" => config.templates.prompt_reviewer = raw_value.to_owned(),
        "templates.completer" => config.templates.completer = raw_value.to_owned(),
        "templates.qa" => config.templates.qa = raw_value.to_owned(),
        "git.auto_branch" => config.git.auto_branch = parse_bool(raw_value, key)?,
        "git.branch_format" => config.git.branch_format = raw_value.to_owned(),
        "git.sign_commits" => config.git.sign_commits = parse_bool(raw_value, key)?,
        "git.base_branch" => config.git.base_branch = raw_value.to_owned(),
        "backends.claude.command" => config.backends.claude.command = raw_value.to_owned(),
        "backends.codex.command" => config.backends.codex.command = raw_value.to_owned(),
        "backends.claude.timeout_seconds" => {
            config.backends.claude.timeout_seconds = parse_u64(raw_value, key)?;
        }
        "backends.codex.timeout_seconds" => {
            config.backends.codex.timeout_seconds = parse_u64(raw_value, key)?;
        }
        "backends.claude.args" => config.backends.claude.args = parse_string_list(raw_value)?,
        "backends.codex.args" => config.backends.codex.args = parse_string_list(raw_value)?,
        _ if key.starts_with("backends.claude.models.") => {
            let role = key.trim_start_matches("backends.claude.models.");
            set_backend_model(&mut config.backends.claude.models, role, raw_value)?;
        }
        _ if key.starts_with("backends.codex.models.") => {
            let role = key.trim_start_matches("backends.codex.models.");
            set_backend_model(&mut config.backends.codex.models, role, raw_value)?;
        }
        _ if key.starts_with("backends.claude.env.") => {
            let env_key = key.trim_start_matches("backends.claude.env.");
            config
                .backends
                .claude
                .env
                .insert(env_key.to_owned(), raw_value.to_owned());
        }
        _ if key.starts_with("backends.codex.env.") => {
            let env_key = key.trim_start_matches("backends.codex.env.");
            config
                .backends
                .codex
                .env
                .insert(env_key.to_owned(), raw_value.to_owned());
        }
        _ => {
            return Err(RalphError::Validation(format!(
                "unsupported global config key: {key}"
            )))
        }
    }

    Ok(())
}

fn set_project_value(config: &mut ProjectConfig, key: &str, raw_value: &str) -> Result<()> {
    match key {
        "workflow.starting_backend" => {
            config.workflow.starting_backend = parse_optional_backend(raw_value)?;
        }
        "workflow.max_review_iterations" => {
            config.workflow.max_review_iterations = parse_optional_u32(raw_value, key)?;
        }
        "workflow.auto_commit" => {
            config.workflow.auto_commit = parse_optional_bool(raw_value, key)?;
        }
        "workflow.commit_message_style" => {
            config.workflow.commit_message_style = parse_optional_commit_message_style(raw_value)?;
        }
        "workflow.prompt_change_action" => {
            config.workflow.prompt_change_action = parse_optional_prompt_change_action(raw_value)?;
        }
        "workflow.prompt_review_enabled" => {
            config.workflow.prompt_review_enabled = parse_optional_bool(raw_value, key)?;
        }
        "workflow.prompt_review_backend" => {
            config.workflow.prompt_review_backend = parse_optional_backend(raw_value)?;
        }
        "workflow.planner_backend" => {
            config.workflow.planner_backend = parse_optional_backend(raw_value)?;
        }
        "workflow.implementer_backend" => {
            config.workflow.implementer_backend = parse_optional_backend(raw_value)?;
        }
        "workflow.reviewer_backend" => {
            config.workflow.reviewer_backend = parse_optional_backend(raw_value)?;
        }
        "workflow.qa_backend" => {
            config.workflow.qa_backend = parse_optional_backend(raw_value)?;
        }
        "workflow.completer_backend" => {
            config.workflow.completer_backend = parse_optional_backend(raw_value)?;
        }
        "workflow.qa_enabled" => {
            config.workflow.qa_enabled = parse_optional_bool(raw_value, key)?;
        }
        "workflow.max_qa_iterations" => {
            config.workflow.max_qa_iterations = parse_optional_u32(raw_value, key)?;
        }
        "templates.planner" => config.templates.planner = parse_optional_string(raw_value),
        "templates.implementer" => config.templates.implementer = parse_optional_string(raw_value),
        "templates.reviewer" => config.templates.reviewer = parse_optional_string(raw_value),
        "templates.prompt_reviewer" => {
            config.templates.prompt_reviewer = parse_optional_string(raw_value)
        }
        "templates.completer" => config.templates.completer = parse_optional_string(raw_value),
        "templates.qa" => config.templates.qa = parse_optional_string(raw_value),
        _ => {
            return Err(RalphError::Validation(format!(
                "unsupported project config key: {key}"
            )))
        }
    }
    Ok(())
}

fn parse_bool(raw: &str, key: &str) -> Result<bool> {
    raw.parse::<bool>().map_err(|_| {
        RalphError::Validation(format!("key '{key}' expects boolean value (true/false)"))
    })
}

fn parse_u32(raw: &str, key: &str) -> Result<u32> {
    raw.parse::<u32>()
        .map_err(|_| RalphError::Validation(format!("key '{key}' expects unsigned integer value")))
}

fn parse_u64(raw: &str, key: &str) -> Result<u64> {
    raw.parse::<u64>()
        .map_err(|_| RalphError::Validation(format!("key '{key}' expects unsigned integer value")))
}

fn parse_optional_bool(raw: &str, key: &str) -> Result<Option<bool>> {
    if raw == "null" {
        return Ok(None);
    }
    Ok(Some(parse_bool(raw, key)?))
}

fn parse_optional_u32(raw: &str, key: &str) -> Result<Option<u32>> {
    if raw == "null" {
        return Ok(None);
    }
    Ok(Some(parse_u32(raw, key)?))
}

fn parse_commit_message_style(raw: &str) -> Result<CommitMessageStyle> {
    match raw {
        "conventional" => Ok(CommitMessageStyle::Conventional),
        "descriptive" => Ok(CommitMessageStyle::Descriptive),
        "minimal" => Ok(CommitMessageStyle::Minimal),
        _ => Err(RalphError::Validation(
            "commit_message_style must be one of: conventional, descriptive, minimal".to_owned(),
        )),
    }
}

fn parse_optional_commit_message_style(raw: &str) -> Result<Option<CommitMessageStyle>> {
    if raw == "null" {
        return Ok(None);
    }
    Ok(Some(parse_commit_message_style(raw)?))
}

fn parse_prompt_change_action(raw: &str) -> Result<PromptChangeAction> {
    match raw {
        "continue" => Ok(PromptChangeAction::Continue),
        "restart-loop" => Ok(PromptChangeAction::RestartLoop),
        "abort" => Ok(PromptChangeAction::Abort),
        _ => Err(RalphError::Validation(
            "prompt_change_action must be one of: continue, restart-loop, abort".to_owned(),
        )),
    }
}

fn parse_optional_prompt_change_action(raw: &str) -> Result<Option<PromptChangeAction>> {
    if raw == "null" {
        return Ok(None);
    }
    Ok(Some(parse_prompt_change_action(raw)?))
}

fn parse_optional_backend(raw: &str) -> Result<Option<String>> {
    if raw == "null" {
        return Ok(None);
    }
    ensure_backend(raw)?;
    Ok(Some(raw.to_owned()))
}

fn ensure_backend(raw: &str) -> Result<()> {
    crate::cli::backend_spec::validate_backend_spec_name(raw)
}

fn parse_optional_string(raw: &str) -> Option<String> {
    if raw == "null" {
        None
    } else {
        Some(raw.to_owned())
    }
}

fn set_backend_model(
    models: &mut crate::config::global::BackendRoleModels,
    role: &str,
    raw_value: &str,
) -> Result<()> {
    let value = if raw_value == "null" {
        None
    } else {
        Some(raw_value.to_owned())
    };
    match role {
        "planner" => models.planner = value,
        "implementer" => models.implementer = value,
        "reviewer" => models.reviewer = value,
        "qa" => models.qa = value,
        "completer" => models.completer = value,
        "reformatter" => models.reformatter = value,
        _ => {
            return Err(RalphError::Validation(format!(
                "unknown backend model role: {role}"
            )))
        }
    }
    Ok(())
}

fn parse_string_list(raw: &str) -> Result<Vec<String>> {
    if raw.trim().starts_with('[') {
        let value: Value = serde_json::from_str(raw).map_err(|_| {
            RalphError::Validation(
                "args must be JSON array (e.g. [\"--flag\"]) or comma-separated list".to_owned(),
            )
        })?;
        let arr = value.as_array().ok_or_else(|| {
            RalphError::Validation(
                "args must be JSON array (e.g. [\"--flag\"]) or comma-separated list".to_owned(),
            )
        })?;
        let mut out = Vec::with_capacity(arr.len());
        for item in arr {
            let Some(s) = item.as_str() else {
                return Err(RalphError::Validation(
                    "args JSON array items must be strings".to_owned(),
                ));
            };
            out.push(s.to_owned());
        }
        return Ok(out);
    }

    let parts = raw
        .split(',')
        .map(|v| v.trim())
        .filter(|v| !v.is_empty())
        .map(|v| v.to_owned())
        .collect::<Vec<_>>();
    Ok(parts)
}

#[allow(dead_code)]
fn config_path_for_scope(workspace: &Workspace, scope: &ConfigScope) -> PathBuf {
    match scope {
        ConfigScope::Global => workspace.root.join("ralph.toml"),
        ConfigScope::Project(project_id) => workspace.project_dir(project_id).join("config.toml"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ensure_backend_accepts_bare_claude() {
        ensure_backend("claude").expect("bare claude should pass");
    }

    #[test]
    fn ensure_backend_accepts_bare_codex() {
        ensure_backend("codex").expect("bare codex should pass");
    }

    #[test]
    fn ensure_backend_accepts_claude_with_model() {
        ensure_backend("claude(opus)").expect("claude(opus) should pass");
    }

    #[test]
    fn ensure_backend_accepts_codex_with_model() {
        ensure_backend("codex(gpt-5.3-codex-xhigh)").expect("codex with model should pass");
    }

    #[test]
    fn ensure_backend_rejects_unknown_base() {
        let err = ensure_backend("unknown(opus)").expect_err("unknown backend should fail");
        assert!(err.to_string().contains("unknown backend"));
    }

    #[test]
    fn ensure_backend_rejects_unknown_bare() {
        let err = ensure_backend("foobar").expect_err("unknown bare backend should fail");
        assert!(err.to_string().contains("unknown backend"));
    }

    #[test]
    fn ensure_backend_rejects_empty_model() {
        ensure_backend("claude()").expect_err("empty model should fail");
    }

    #[test]
    fn ensure_backend_rejects_missing_close_paren() {
        ensure_backend("claude(opus").expect_err("missing close paren should fail");
    }

    #[test]
    fn ensure_backend_rejects_empty_name_with_model() {
        ensure_backend("(opus)").expect_err("empty name should fail");
    }

    #[test]
    fn parse_optional_backend_accepts_claude_with_model() {
        let result = parse_optional_backend("claude(opus)").expect("should parse successfully");
        assert_eq!(result, Some("claude(opus)".to_owned()));
    }

    #[test]
    fn parse_optional_backend_accepts_bare_name() {
        let result = parse_optional_backend("codex").expect("should parse successfully");
        assert_eq!(result, Some("codex".to_owned()));
    }

    #[test]
    fn parse_optional_backend_accepts_null() {
        let result = parse_optional_backend("null").expect("should parse successfully");
        assert_eq!(result, None);
    }

    #[test]
    fn parse_optional_backend_rejects_unknown() {
        parse_optional_backend("unknown(opus)").expect_err("unknown backend should fail");
    }

    #[test]
    fn parse_optional_backend_rejects_malformed() {
        parse_optional_backend("claude()").expect_err("malformed spec should fail");
    }

    #[test]
    fn resolve_config_alias_maps_qa_backend() {
        assert_eq!(resolve_config_alias("qa_backend"), "workflow.qa_backend");
    }
}
