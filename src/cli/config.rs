use std::path::PathBuf;
use std::process::Command;

use serde_json::Value;

use crate::cli::{ConfigArgs, ConfigCommand};
use crate::config::{
    resolve_effective_config, CommitMessageStyle, PlannerStateInPrompt, PreviousSpecsInPrompt,
    ProjectConfig, PromptChangeAction, RunWorkflowOverrides,
};
use crate::daemon::rebase_agent::parse_rebase_agent_backend;
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

    if let Some(active) = workspace.active_project_id() {
        if workspace.project_exists(&active) {
            return Ok(ConfigScope::Project(active));
        }
        eprintln!(
            "warning: active project '{}' no longer exists; falling back to global scope. \
             Run `ralph project use <id>` to set a new active project.",
            active
        );
    }

    Ok(ConfigScope::Global)
}

fn ensure_project_exists(workspace: &Workspace, project_id: &str) -> Result<()> {
    if !workspace.project_exists(project_id) {
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
            let prompt_review_backend_alias =
                effective.workflow.prompt_review_backends.first().cloned();

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
                    "prompt_review_backend": prompt_review_backend_alias,
                    "prompt_review_backends": effective.workflow.prompt_review_backends,
                    "prompt_review_min_reviewers": effective.workflow.prompt_review_min_reviewers,
                    "planner_backend": effective.workflow.planner_backend,
                    "implementer_backend": effective.workflow.implementer_backend,
                    "reviewer_backend": effective.workflow.reviewer_backend,
                    "qa_backend": effective.workflow.qa_backend,
                    "completer_backend": effective.workflow.completer_backend,
                    "final_review_enabled": effective.workflow.final_review_enabled,
                    "final_review_backends": effective.workflow.final_review_backends,
                    "final_review_arbiter_backend": effective.workflow.final_review_arbiter_backend,
                    "final_review_min_reviewers": effective.workflow.final_review_min_reviewers,
                    "final_review_consensus_threshold": effective.workflow.final_review_consensus_threshold,
                    "max_final_review_restarts": effective.workflow.max_final_review_restarts,
                    "completion_backends": effective.workflow.completion_backends,
                    "completion_min_completers": effective.workflow.completion_min_completers,
                    "completion_consensus_threshold": effective.workflow.completion_consensus_threshold,
                    "qa_enabled": effective.workflow.qa_enabled,
                    "max_qa_iterations": effective.workflow.max_qa_iterations,
                    "max_review_iterations": effective.workflow.max_review_iterations,
                    "auto_commit": effective.workflow.auto_commit,
                    "commit_message_style": effective.workflow.commit_message_style,
                    "commit_tag_format": effective.workflow.commit_tag_format,
                    "prompt_change_action": effective.workflow.prompt_change_action,
                    "planner_state_in_prompt": effective.workflow.planner_state_in_prompt,
                    "planner_previous_specs_in_prompt": effective.workflow.planner_previous_specs_in_prompt,
                    "planner_max_prior_loops": effective.workflow.planner_max_prior_loops,
                    "max_review_history_entries_in_prompt": effective.workflow.max_review_history_entries_in_prompt,
                    "max_qa_history_entries_in_prompt": effective.workflow.max_qa_history_entries_in_prompt,
                    "include_history_when_session_reuse_enabled": effective.workflow.include_history_when_session_reuse_enabled,
                    "session_reuse_enabled": effective.workflow.session_reuse_enabled,
                    "session_reuse_roles": effective.workflow.session_reuse_roles,
                    "session_reuse_reset_on_prompt_change": effective.workflow.session_reuse_reset_on_prompt_change,
                    "session_reuse_reset_on_rollback": effective.workflow.session_reuse_reset_on_rollback,
                    "pre_commit_fmt": effective.workflow.pre_commit_fmt,
                    "pre_commit_clippy": effective.workflow.pre_commit_clippy,
                    "pre_commit_nix_build": effective.workflow.pre_commit_nix_build,
                    "pre_commit_fmt_auto_fix": effective.workflow.pre_commit_fmt_auto_fix,
                },
                "daemon": {
                    "poll_seconds": effective.daemon.poll_seconds,
                    "max_concurrent": effective.daemon.max_concurrent,
                    "labels": effective.daemon.labels,
                    "repo": effective.daemon.repo,
                    "refinement_enabled": effective.daemon.refinement_enabled,
                    "refinement_backend": effective.daemon.refinement_backend,
                    "auto_rebase_enabled": effective.daemon.auto_rebase_enabled,
                    "rebase_interval_seconds": effective.daemon.rebase_interval_seconds,
                    "max_rebases_per_cycle": effective.daemon.max_rebases_per_cycle,
                    "rebase_timeout_seconds": effective.daemon.rebase_timeout_seconds,
                    "rebase_agent_backend": effective.daemon.rebase_agent_backend,
                    "oracle_review_enabled": effective.daemon.oracle_review_enabled,
                    "oracle_review_timeout_secs": effective.daemon.oracle_review_timeout_secs,
                    "oracle_review_authors": effective.daemon.oracle_review_authors,
                    "oracle_review_max_per_cycle": effective.daemon.oracle_review_max_per_cycle,
                    "oracle_review_cooldown_secs": effective.daemon.oracle_review_cooldown_secs,
                    "oracle_review_args": effective.daemon.oracle_review_args,
                },
                "templates": {
                    "planner": effective.templates.planner,
                    "implementer": effective.templates.implementer,
                    "reviewer": effective.templates.reviewer,
                    "prompt_reviewer": effective.templates.prompt_reviewer,
                    "prompt_review_validator": effective.templates.prompt_review_validator,
                    "completer": effective.templates.completer,
                    "qa": effective.templates.qa,
                    "final_reviewer": effective.templates.final_reviewer,
                    "planner_position": effective.templates.planner_position,
                    "vote": effective.templates.vote,
                    "arbiter": effective.templates.arbiter,
                },
                "amendments": {
                    "unify_final_review": effective.amendments.unify_final_review,
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
            let prompt_review_backend_alias =
                effective.workflow.prompt_review_backends.first().cloned();

            serde_json::json!({
                "workspace": effective.global.workspace,
                "backends": effective.global.backends,
                "workflow": {
                    "starting_backend": effective.workflow.starting_backend,
                    "prompt_review_enabled": effective.workflow.prompt_review_enabled,
                    "prompt_review_backend": prompt_review_backend_alias,
                    "prompt_review_backends": effective.workflow.prompt_review_backends,
                    "prompt_review_min_reviewers": effective.workflow.prompt_review_min_reviewers,
                    "planner_backend": effective.workflow.planner_backend,
                    "implementer_backend": effective.workflow.implementer_backend,
                    "reviewer_backend": effective.workflow.reviewer_backend,
                    "qa_backend": effective.workflow.qa_backend,
                    "completer_backend": effective.workflow.completer_backend,
                    "final_review_enabled": effective.workflow.final_review_enabled,
                    "final_review_backends": effective.workflow.final_review_backends,
                    "final_review_arbiter_backend": effective.workflow.final_review_arbiter_backend,
                    "final_review_min_reviewers": effective.workflow.final_review_min_reviewers,
                    "final_review_consensus_threshold": effective.workflow.final_review_consensus_threshold,
                    "max_final_review_restarts": effective.workflow.max_final_review_restarts,
                    "completion_backends": effective.workflow.completion_backends,
                    "completion_min_completers": effective.workflow.completion_min_completers,
                    "completion_consensus_threshold": effective.workflow.completion_consensus_threshold,
                    "qa_enabled": effective.workflow.qa_enabled,
                    "max_qa_iterations": effective.workflow.max_qa_iterations,
                    "max_review_iterations": effective.workflow.max_review_iterations,
                    "auto_commit": effective.workflow.auto_commit,
                    "commit_message_style": effective.workflow.commit_message_style,
                    "commit_tag_format": effective.workflow.commit_tag_format,
                    "prompt_change_action": effective.workflow.prompt_change_action,
                    "planner_state_in_prompt": effective.workflow.planner_state_in_prompt,
                    "planner_previous_specs_in_prompt": effective.workflow.planner_previous_specs_in_prompt,
                    "planner_max_prior_loops": effective.workflow.planner_max_prior_loops,
                    "max_review_history_entries_in_prompt": effective.workflow.max_review_history_entries_in_prompt,
                    "max_qa_history_entries_in_prompt": effective.workflow.max_qa_history_entries_in_prompt,
                    "include_history_when_session_reuse_enabled": effective.workflow.include_history_when_session_reuse_enabled,
                    "session_reuse_enabled": effective.workflow.session_reuse_enabled,
                    "session_reuse_roles": effective.workflow.session_reuse_roles,
                    "session_reuse_reset_on_prompt_change": effective.workflow.session_reuse_reset_on_prompt_change,
                    "session_reuse_reset_on_rollback": effective.workflow.session_reuse_reset_on_rollback,
                    "pre_commit_fmt": effective.workflow.pre_commit_fmt,
                    "pre_commit_clippy": effective.workflow.pre_commit_clippy,
                    "pre_commit_nix_build": effective.workflow.pre_commit_nix_build,
                    "pre_commit_fmt_auto_fix": effective.workflow.pre_commit_fmt_auto_fix,
                },
                "daemon": {
                    "poll_seconds": effective.daemon.poll_seconds,
                    "max_concurrent": effective.daemon.max_concurrent,
                    "labels": effective.daemon.labels,
                    "repo": effective.daemon.repo,
                    "refinement_enabled": effective.daemon.refinement_enabled,
                    "refinement_backend": effective.daemon.refinement_backend,
                    "auto_rebase_enabled": effective.daemon.auto_rebase_enabled,
                    "rebase_interval_seconds": effective.daemon.rebase_interval_seconds,
                    "max_rebases_per_cycle": effective.daemon.max_rebases_per_cycle,
                    "rebase_timeout_seconds": effective.daemon.rebase_timeout_seconds,
                    "rebase_agent_backend": effective.daemon.rebase_agent_backend,
                    "oracle_review_enabled": effective.daemon.oracle_review_enabled,
                    "oracle_review_timeout_secs": effective.daemon.oracle_review_timeout_secs,
                    "oracle_review_authors": effective.daemon.oracle_review_authors,
                    "oracle_review_max_per_cycle": effective.daemon.oracle_review_max_per_cycle,
                    "oracle_review_cooldown_secs": effective.daemon.oracle_review_cooldown_secs,
                    "oracle_review_args": effective.daemon.oracle_review_args,
                },
                "templates": {
                    "planner": effective.templates.planner,
                    "implementer": effective.templates.implementer,
                    "reviewer": effective.templates.reviewer,
                    "prompt_reviewer": effective.templates.prompt_reviewer,
                    "prompt_review_validator": effective.templates.prompt_review_validator,
                    "completer": effective.templates.completer,
                    "qa": effective.templates.qa,
                    "final_reviewer": effective.templates.final_reviewer,
                    "planner_position": effective.templates.planner_position,
                    "vote": effective.templates.vote,
                    "arbiter": effective.templates.arbiter,
                },
                "amendments": {
                    "unify_final_review": effective.amendments.unify_final_review,
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
            let config_path = workspace.root.join("ralph.toml");
            crate::config::save_sparse(&config_path, key, &workspace.config)?;
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
    crate::config::set_global_config_value(config, key, raw_value)
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
            config.workflow.prompt_review_backend =
                parse_optional_required_backend(raw_value, "workflow.prompt_review_backend")?;
        }
        "workflow.prompt_review_backends" => {
            config.workflow.prompt_review_backends = parse_optional_string_list(raw_value)?;
        }
        "workflow.prompt_review_min_reviewers" => {
            config.workflow.prompt_review_min_reviewers = parse_optional_u32(raw_value, key)?;
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
        "workflow.final_review_enabled" => {
            config.workflow.final_review_enabled = parse_optional_bool(raw_value, key)?;
        }
        "workflow.final_review_backends" => {
            config.workflow.final_review_backends = parse_optional_string_list(raw_value)?;
        }
        "workflow.final_review_arbiter_backend" => {
            config.workflow.final_review_arbiter_backend = parse_optional_backend(raw_value)?;
        }
        "workflow.final_review_min_reviewers" => {
            config.workflow.final_review_min_reviewers = parse_optional_u32(raw_value, key)?;
        }
        "workflow.final_review_consensus_threshold" => {
            if raw_value == "null" {
                config.workflow.final_review_consensus_threshold = None;
            } else {
                let v: f64 = raw_value.parse().map_err(|_| {
                    RalphError::Validation(format!("key '{key}' expects float value"))
                })?;
                config.workflow.final_review_consensus_threshold = Some(v);
            }
        }
        "workflow.max_final_review_restarts" => {
            config.workflow.max_final_review_restarts = parse_optional_u32(raw_value, key)?;
        }
        "workflow.completion_backends" => {
            config.workflow.completion_backends = parse_optional_string_list(raw_value)?;
        }
        "workflow.completion_min_completers" => {
            config.workflow.completion_min_completers = parse_optional_u32(raw_value, key)?;
        }
        "workflow.completion_consensus_threshold" => {
            if raw_value == "null" {
                config.workflow.completion_consensus_threshold = None;
            } else {
                let v: f64 = raw_value.parse().map_err(|_| {
                    RalphError::Validation(format!("key '{key}' expects float value"))
                })?;
                config.workflow.completion_consensus_threshold = Some(v);
            }
        }
        "workflow.qa_enabled" => {
            config.workflow.qa_enabled = parse_optional_bool(raw_value, key)?;
        }
        "workflow.max_qa_iterations" => {
            config.workflow.max_qa_iterations = parse_optional_u32(raw_value, key)?;
        }
        "workflow.planner_state_in_prompt" => {
            config.workflow.planner_state_in_prompt =
                parse_optional_planner_state_in_prompt(raw_value)?;
        }
        "workflow.planner_previous_specs_in_prompt" => {
            config.workflow.planner_previous_specs_in_prompt =
                parse_optional_previous_specs_in_prompt(raw_value)?;
        }
        "workflow.planner_max_prior_loops" => {
            config.workflow.planner_max_prior_loops =
                parse_project_optional_usize_or_none(raw_value, key)?;
        }
        "workflow.max_review_history_entries_in_prompt" => {
            config.workflow.max_review_history_entries_in_prompt =
                parse_optional_usize(raw_value, key)?;
        }
        "workflow.max_qa_history_entries_in_prompt" => {
            config.workflow.max_qa_history_entries_in_prompt =
                parse_optional_usize(raw_value, key)?;
        }
        "workflow.include_history_when_session_reuse_enabled" => {
            config.workflow.include_history_when_session_reuse_enabled =
                parse_optional_bool(raw_value, key)?;
        }
        "workflow.session_reuse_enabled" => {
            config.workflow.session_reuse_enabled = parse_optional_bool(raw_value, key)?;
        }
        "workflow.session_reuse_roles" => {
            config.workflow.session_reuse_roles = parse_optional_session_reuse_roles(raw_value)?;
        }
        "workflow.session_reuse_reset_on_prompt_change" => {
            config.workflow.session_reuse_reset_on_prompt_change =
                parse_optional_bool(raw_value, key)?;
        }
        "workflow.session_reuse_reset_on_rollback" => {
            config.workflow.session_reuse_reset_on_rollback = parse_optional_bool(raw_value, key)?;
        }
        "workflow.pre_commit_fmt" => {
            config.workflow.pre_commit_fmt = parse_optional_bool(raw_value, key)?;
        }
        "workflow.pre_commit_clippy" => {
            config.workflow.pre_commit_clippy = parse_optional_bool(raw_value, key)?;
        }
        "workflow.pre_commit_nix_build" => {
            config.workflow.pre_commit_nix_build = parse_optional_bool(raw_value, key)?;
        }
        "workflow.pre_commit_fmt_auto_fix" => {
            config.workflow.pre_commit_fmt_auto_fix = parse_optional_bool(raw_value, key)?;
        }
        "templates.planner" => config.templates.planner = parse_optional_string(raw_value),
        "templates.implementer" => config.templates.implementer = parse_optional_string(raw_value),
        "templates.reviewer" => config.templates.reviewer = parse_optional_string(raw_value),
        "templates.prompt_reviewer" => {
            config.templates.prompt_reviewer = parse_optional_string(raw_value)
        }
        "templates.prompt_review_validator" => {
            config.templates.prompt_review_validator = parse_optional_string(raw_value)
        }
        "templates.completer" => config.templates.completer = parse_optional_string(raw_value),
        "templates.qa" => config.templates.qa = parse_optional_string(raw_value),
        "templates.final_reviewer" => {
            config.templates.final_reviewer = parse_optional_string(raw_value)
        }
        "templates.planner_position" => {
            config.templates.planner_position = parse_optional_string(raw_value)
        }
        "templates.vote" => config.templates.vote = parse_optional_string(raw_value),
        "templates.arbiter" => config.templates.arbiter = parse_optional_string(raw_value),
        "daemon.poll_seconds" => {
            config.daemon.poll_seconds = parse_optional_u64(raw_value, key)?;
        }
        "daemon.max_concurrent" => {
            config.daemon.max_concurrent = parse_optional_u32(raw_value, key)?;
        }
        "daemon.labels" => {
            config.daemon.labels = parse_optional_string_list(raw_value)?;
        }
        "daemon.repo" => config.daemon.repo = parse_optional_string(raw_value),
        "daemon.refinement_enabled" => {
            config.daemon.refinement_enabled = parse_optional_bool(raw_value, key)?;
        }
        "daemon.refinement_backend" => {
            config.daemon.refinement_backend = parse_optional_backend(raw_value)?;
        }
        "daemon.auto_rebase_enabled" => {
            config.daemon.auto_rebase_enabled = parse_optional_bool(raw_value, key)?;
        }
        "daemon.rebase_interval_seconds" => {
            config.daemon.rebase_interval_seconds = parse_optional_u64(raw_value, key)?;
        }
        "daemon.max_rebases_per_cycle" => {
            config.daemon.max_rebases_per_cycle = parse_optional_u32(raw_value, key)?;
        }
        "daemon.rebase_timeout_seconds" => {
            config.daemon.rebase_timeout_seconds = parse_optional_u64(raw_value, key)?;
        }
        "daemon.rebase_agent_backend" => {
            config.daemon.rebase_agent_backend = parse_optional_rebase_agent_backend(raw_value)?;
        }
        "amendments.unify_final_review" => {
            config.amendments.unify_final_review = parse_optional_bool(raw_value, key)?;
        }
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

fn parse_usize(raw: &str, key: &str) -> Result<usize> {
    raw.parse::<usize>()
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

fn parse_optional_u64(raw: &str, key: &str) -> Result<Option<u64>> {
    if raw == "null" {
        return Ok(None);
    }
    Ok(Some(parse_u64(raw, key)?))
}

fn parse_optional_usize(raw: &str, key: &str) -> Result<Option<usize>> {
    if raw == "null" {
        return Ok(None);
    }
    Ok(Some(parse_usize(raw, key)?))
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

fn parse_planner_state_in_prompt(raw: &str) -> Result<PlannerStateInPrompt> {
    match raw {
        "full-json" => Ok(PlannerStateInPrompt::FullJson),
        "summary" => Ok(PlannerStateInPrompt::Summary),
        _ => Err(RalphError::Validation(
            "planner_state_in_prompt must be one of: full-json, summary".to_owned(),
        )),
    }
}

fn parse_optional_planner_state_in_prompt(raw: &str) -> Result<Option<PlannerStateInPrompt>> {
    if raw == "null" {
        return Ok(None);
    }
    Ok(Some(parse_planner_state_in_prompt(raw)?))
}

fn parse_previous_specs_in_prompt(raw: &str) -> Result<PreviousSpecsInPrompt> {
    match raw {
        "none" => Ok(PreviousSpecsInPrompt::None),
        "titles" => Ok(PreviousSpecsInPrompt::Titles),
        "full-text" => Ok(PreviousSpecsInPrompt::FullText),
        _ => Err(RalphError::Validation(
            "planner_previous_specs_in_prompt must be one of: none, titles, full-text".to_owned(),
        )),
    }
}

fn parse_optional_previous_specs_in_prompt(raw: &str) -> Result<Option<PreviousSpecsInPrompt>> {
    if raw == "null" {
        return Ok(None);
    }
    Ok(Some(parse_previous_specs_in_prompt(raw)?))
}

/// Parse `"none"` as `None` (unlimited), integer as `Some(n)`.
fn parse_optional_usize_or_none(raw: &str, key: &str) -> Result<Option<usize>> {
    if raw == "none" {
        return Ok(None);
    }
    let n = raw.parse::<usize>().map_err(|_| {
        RalphError::Validation(format!(
            "key '{key}' expects unsigned integer or \"none\" for unlimited"
        ))
    })?;
    Ok(Some(n))
}

/// For project overrides: `"null"` = inherit, `"none"` = override to unlimited, integer = cap.
fn parse_project_optional_usize_or_none(raw: &str, key: &str) -> Result<Option<Option<usize>>> {
    if raw == "null" {
        return Ok(None);
    }
    Ok(Some(parse_optional_usize_or_none(raw, key)?))
}

fn parse_optional_backend(raw: &str) -> Result<Option<String>> {
    if raw == "null" {
        return Ok(None);
    }
    ensure_backend(raw)?;
    Ok(Some(raw.to_owned()))
}

fn parse_optional_required_backend(raw: &str, label: &str) -> Result<Option<String>> {
    if raw == "null" {
        return Ok(None);
    }
    ensure_required_backend(raw, label)?;
    Ok(Some(raw.to_owned()))
}

fn ensure_backend(raw: &str) -> Result<()> {
    crate::cli::backend_spec::validate_backend_spec_name(raw)
}

fn ensure_required_backend(raw: &str, label: &str) -> Result<()> {
    let parsed = crate::backend::parse_backend_spec(raw)?;
    if parsed.optional {
        return Err(RalphError::Validation(format!(
            "optional backend specs (?backend) are not supported for {label}; optional syntax is allowed only in panel backend lists"
        )));
    }
    ensure_backend(raw)
}

const KNOWN_ROLES: &[&str] = &["planner", "implementer", "reviewer", "qa", "completer"];

fn parse_session_reuse_roles(raw: &str) -> Result<Vec<String>> {
    let roles = parse_string_list(raw)?;
    for role in &roles {
        if !KNOWN_ROLES.contains(&role.as_str()) {
            return Err(RalphError::Validation(format!(
                "unknown role '{}' in session_reuse_roles; valid roles: {}",
                role,
                KNOWN_ROLES.join(", ")
            )));
        }
    }
    Ok(roles)
}

fn parse_optional_session_reuse_roles(raw: &str) -> Result<Option<Vec<String>>> {
    if raw == "null" {
        return Ok(None);
    }
    Ok(Some(parse_session_reuse_roles(raw)?))
}

fn parse_optional_string(raw: &str) -> Option<String> {
    if raw == "null" {
        None
    } else {
        Some(raw.to_owned())
    }
}

fn parse_optional_rebase_agent_backend(raw: &str) -> Result<Option<String>> {
    if raw == "null" {
        return Ok(None);
    }
    parse_rebase_agent_backend(raw)?;
    Ok(Some(raw.trim().to_owned()))
}

fn parse_optional_string_list(raw: &str) -> Result<Option<Vec<String>>> {
    if raw == "null" {
        return Ok(None);
    }
    Ok(Some(parse_string_list(raw)?))
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
        ensure_backend("codex(gpt-5.4-xhigh)").expect("codex with model should pass");
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
    fn ensure_required_backend_rejects_optional_syntax() {
        let err = ensure_required_backend("?openrouter", "workflow.prompt_review_backend")
            .expect_err("optional syntax should be rejected for required surfaces");
        assert!(err.to_string().contains(
            "optional backend specs (?backend) are not supported for workflow.prompt_review_backend"
        ));
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
    fn parse_optional_backend_accepts_optional_openrouter() {
        let result = parse_optional_backend("?openrouter(gpt-5.4-xhigh)")
            .expect("optional openrouter should parse successfully");
        assert_eq!(result, Some("?openrouter(gpt-5.4-xhigh)".to_owned()));
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
    fn set_global_value_rejects_optional_prompt_review_backend_alias() {
        let mut config = crate::config::GlobalConfig::default();
        let err = set_global_value(
            &mut config,
            "workflow.prompt_review_backend",
            "?openrouter(gpt-5.4-xhigh)",
        )
        .expect_err("optional syntax should be rejected for singular alias");
        assert!(err.to_string().contains(
            "optional backend specs (?backend) are not supported for workflow.prompt_review_backend"
        ));
    }

    #[test]
    fn set_project_value_rejects_optional_prompt_review_backend_alias() {
        let mut config = crate::config::ProjectConfig::default();
        let err = set_project_value(&mut config, "workflow.prompt_review_backend", "?claude")
            .expect_err("optional syntax should be rejected for project singular alias");
        assert!(err.to_string().contains(
            "optional backend specs (?backend) are not supported for workflow.prompt_review_backend"
        ));
    }

    #[test]
    fn resolve_config_alias_maps_qa_backend() {
        assert_eq!(resolve_config_alias("qa_backend"), "workflow.qa_backend");
    }

    #[test]
    fn set_global_value_sets_and_clears_role_timeout() {
        let mut config = crate::config::GlobalConfig::default();
        set_global_value(
            &mut config,
            "backends.claude.role_timeouts.acceptance_qa",
            "42",
        )
        .expect("acceptance_qa timeout should set");
        assert_eq!(config.backends.claude.role_timeouts.acceptance_qa, Some(42));

        set_global_value(
            &mut config,
            "backends.claude.role_timeouts.acceptance_qa",
            "null",
        )
        .expect("acceptance_qa timeout should clear");
        assert_eq!(config.backends.claude.role_timeouts.acceptance_qa, None);
    }

    #[test]
    fn set_global_value_rejects_unknown_role_timeout() {
        let mut config = crate::config::GlobalConfig::default();
        let err = set_global_value(&mut config, "backends.claude.role_timeouts.bogus", "42")
            .expect_err("unknown timeout role should fail");
        assert!(err.to_string().contains("unknown backend timeout role"));
    }

    #[test]
    fn set_global_value_updates_history_capping_fields() {
        let mut config = crate::config::GlobalConfig::default();

        set_global_value(
            &mut config,
            "workflow.max_review_history_entries_in_prompt",
            "7",
        )
        .expect("set review history cap");
        set_global_value(
            &mut config,
            "workflow.max_qa_history_entries_in_prompt",
            "4",
        )
        .expect("set qa history cap");
        set_global_value(
            &mut config,
            "workflow.include_history_when_session_reuse_enabled",
            "true",
        )
        .expect("set include-history flag");

        assert_eq!(config.workflow.max_review_history_entries_in_prompt, 7);
        assert_eq!(config.workflow.max_qa_history_entries_in_prompt, 4);
        assert!(config.workflow.include_history_when_session_reuse_enabled);
    }

    #[test]
    fn set_project_value_updates_history_capping_fields() {
        let mut config = crate::config::ProjectConfig::default();

        set_project_value(
            &mut config,
            "workflow.max_review_history_entries_in_prompt",
            "6",
        )
        .expect("set review history cap override");
        set_project_value(
            &mut config,
            "workflow.max_qa_history_entries_in_prompt",
            "3",
        )
        .expect("set qa history cap override");
        set_project_value(
            &mut config,
            "workflow.include_history_when_session_reuse_enabled",
            "false",
        )
        .expect("set include-history override");

        assert_eq!(
            config.workflow.max_review_history_entries_in_prompt,
            Some(6)
        );
        assert_eq!(config.workflow.max_qa_history_entries_in_prompt, Some(3));
        assert_eq!(
            config.workflow.include_history_when_session_reuse_enabled,
            Some(false)
        );
    }
}
