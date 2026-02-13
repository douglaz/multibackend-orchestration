use std::path::PathBuf;

use serde_json::{json, Value};

use crate::cli::backend_spec::validate_backend_spec_name;
use crate::config::{resolve_effective_config, PromptChangeAction, RunWorkflowOverrides};
use crate::error::RalphError;
use crate::mcp::protocol::CallToolResult;
use crate::mcp::tail_events::collect_tail_events;
use crate::prd::quick::{render_prompt, QuickPrdOptions, QuickPrdPipeline, DRAFT_PROMPT};
use crate::project::lifecycle::{
    create_project, load_project_state, CreateProjectOptions, PromptSource,
};
use crate::project::load_project_config_if_exists;
use crate::workflow::orchestrator::{Orchestrator, RunOptions};
use crate::workspace::Workspace;

/// Dispatch a tool call to the appropriate handler.
///
/// Returns a `CallToolResult` JSON value on success, or a `String` error
/// message on failure (which the server wraps as `isError: true`).
pub async fn handle_tool_call(name: &str, arguments: Value) -> Result<Value, String> {
    match name {
        "project_new" => handle_project_new(arguments).await,
        "project_list" => handle_project_list(arguments).await,
        "project_show" => handle_project_show(arguments).await,
        "run" => handle_run(arguments).await,
        "status" => handle_status(arguments).await,
        "history" => handle_history(arguments).await,
        "tail" => handle_tail(arguments).await,
        "quick_prd" => handle_quick_prd(arguments).await,
        "config_show" => handle_config_show(arguments).await,
        _ => Err(format!("unknown tool: {name}")),
    }
}

// ---------------------------------------------------------------------------
// Argument extraction helpers
// ---------------------------------------------------------------------------

fn get_str(args: &Value, key: &str) -> Option<String> {
    args.get(key)
        .and_then(|v| v.as_str())
        .map(ToOwned::to_owned)
}

fn get_bool(args: &Value, key: &str) -> Option<bool> {
    args.get(key).and_then(|v| v.as_bool())
}

fn get_u32(args: &Value, key: &str) -> Option<u32> {
    args.get(key)
        .and_then(|v| v.as_u64())
        .and_then(|v| u32::try_from(v).ok())
}

fn get_usize(args: &Value, key: &str) -> Option<usize> {
    args.get(key)
        .and_then(|v| v.as_u64())
        .and_then(|v| usize::try_from(v).ok())
}

fn require_str(args: &Value, key: &str) -> Result<String, String> {
    get_str(args, key).ok_or_else(|| format!("missing required argument: {key}"))
}

/// Resolve a project ID from arguments or fall back to the active project.
fn resolve_project_id(args: &Value, workspace: &Workspace) -> Result<String, String> {
    if let Some(id) = get_str(args, "project") {
        return Ok(id);
    }
    workspace
        .index
        .active_project
        .clone()
        .ok_or_else(|| "no project specified and no active project is set".to_owned())
}

/// Map a `crate::Result` error into a tool-domain error string.
fn map_err(err: RalphError) -> String {
    err.to_string()
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

async fn handle_project_new(args: Value) -> Result<Value, String> {
    let id = require_str(&args, "id")?;
    let name = require_str(&args, "name")?;

    let prompt_file = get_str(&args, "prompt_file");
    let from_project = get_str(&args, "from_project");

    let source = match (prompt_file, from_project) {
        (Some(path), None) => PromptSource::File(PathBuf::from(path)),
        (None, Some(parent)) => PromptSource::ParentProject(parent),
        (Some(_), Some(_)) => {
            return Err("prompt_file and from_project are mutually exclusive".to_owned())
        }
        (None, None) => {
            return Err("exactly one of prompt_file or from_project is required".to_owned())
        }
    };

    if let Some(backend) = get_str(&args, "backend") {
        validate_backend_spec_name(&backend).map_err(map_err)?;
    }

    let mut workspace = Workspace::discover().map_err(map_err)?;

    create_project(
        &mut workspace,
        CreateProjectOptions {
            id: id.clone(),
            name: name.clone(),
            source,
            starting_backend: get_str(&args, "backend"),
        },
    )
    .map_err(map_err)?;

    Ok(CallToolResult::success_json(json!({
        "created": true,
        "project_id": id,
        "project_name": name,
    })))
}

async fn handle_project_list(_args: Value) -> Result<Value, String> {
    let workspace = Workspace::discover().map_err(map_err)?;

    let projects: Vec<Value> = workspace
        .index
        .projects
        .iter()
        .map(|p| {
            json!({
                "id": p.id,
                "name": p.name,
                "status": p.status,
                "total_feature_loops": p.total_feature_loops,
                "last_loop_number": p.last_loop_number,
                "active": workspace.index.active_project.as_deref() == Some(&p.id),
            })
        })
        .collect();

    Ok(CallToolResult::success_json(
        json!({ "projects": projects }),
    ))
}

async fn handle_project_show(args: Value) -> Result<Value, String> {
    let workspace = Workspace::discover().map_err(map_err)?;
    let project_id = resolve_project_id(&args, &workspace)?;

    let project_meta = workspace
        .index
        .get_project(&project_id)
        .ok_or_else(|| format!("project not found: {project_id}"))?;
    let project_dir = workspace.project_dir(&project_id);
    let state = load_project_state(&project_dir).map_err(map_err)?;

    Ok(CallToolResult::success_json(json!({
        "project": project_meta,
        "state": state,
    })))
}

async fn handle_run(args: Value) -> Result<Value, String> {
    let on_prompt_change = match get_str(&args, "on_prompt_change").as_deref() {
        Some("continue") => Some(PromptChangeAction::Continue),
        Some("restart-loop") => Some(PromptChangeAction::RestartLoop),
        Some("abort") => Some(PromptChangeAction::Abort),
        Some(other) => {
            return Err(format!(
                "invalid on_prompt_change value: {other} (expected: continue, restart-loop, abort)"
            ))
        }
        None => None,
    };

    let workspace = Workspace::discover().map_err(map_err)?;
    let mut orchestrator = Orchestrator::new(workspace);

    let result = orchestrator
        .run(RunOptions {
            project: get_str(&args, "project"),
            loops: get_u32(&args, "loops"),
            until_review: get_bool(&args, "until_review").unwrap_or(false),
            until_complete: get_bool(&args, "until_complete").unwrap_or(false),
            dry_run: get_bool(&args, "dry_run").unwrap_or(false),
            backend: get_str(&args, "backend"),
            planner_backend: get_str(&args, "planner_backend"),
            implementer_backend: get_str(&args, "implementer_backend"),
            reviewer_backend: get_str(&args, "reviewer_backend"),
            qa_backend: get_str(&args, "qa_backend"),
            completer_backend: get_str(&args, "completer_backend"),
            on_prompt_change,
            skip_commit: get_bool(&args, "skip_commit").unwrap_or(false),
            tmux: None, // tmux is not available via MCP
        })
        .await
        .map_err(map_err)?;

    Ok(CallToolResult::success_json(json!({
        "summary": result.summary,
        "loop_number": result.loop_number,
    })))
}

async fn handle_status(args: Value) -> Result<Value, String> {
    let workspace = Workspace::discover().map_err(map_err)?;
    let project_id = resolve_project_id(&args, &workspace)?;

    let _project_ref = workspace
        .index
        .get_project(&project_id)
        .ok_or_else(|| format!("project not found: {project_id}"))?;
    let project_dir = workspace.project_dir(&project_id);
    let state = load_project_state(&project_dir).map_err(map_err)?;

    let mut result = json!({
        "project_id": state.project_id,
        "project_name": state.project_name,
        "status": state.status,
        "current_loop": state.current_loop,
        "current_phase": state.current_phase,
        "phase_iteration": state.phase_iteration,
    });

    let obj = result.as_object_mut().unwrap();

    if let Some(loop_state) = state.current_feature_loop() {
        obj.insert("feature_name".to_owned(), json!(loop_state.feature_name));
        obj.insert(
            "backends".to_owned(),
            json!({
                "planner": loop_state.backends.planner,
                "implementer": loop_state.backends.implementer,
                "reviewer": loop_state.backends.reviewer,
                "qa": loop_state.backends.qa,
            }),
        );
    } else if let Some(attempt) = state.current_completion_attempt() {
        obj.insert("completion_loop".to_owned(), json!(attempt.loop_number));
        obj.insert(
            "backends".to_owned(),
            json!({
                "planner": attempt.backends.planner,
                "completer": attempt.backends.completer,
            }),
        );
        obj.insert(
            "verdict".to_owned(),
            match &attempt.verdict {
                Some(v) => serde_json::to_value(v).unwrap_or(Value::Null),
                None => Value::Null,
            },
        );
    }

    Ok(CallToolResult::success_json(result))
}

async fn handle_history(args: Value) -> Result<Value, String> {
    let workspace = Workspace::discover().map_err(map_err)?;
    let project_id = resolve_project_id(&args, &workspace)?;

    let _project_ref = workspace
        .index
        .get_project(&project_id)
        .ok_or_else(|| format!("project not found: {project_id}"))?;
    let project_dir = workspace.project_dir(&project_id);
    let state = load_project_state(&project_dir).map_err(map_err)?;

    let mut entries: Vec<Value> = Vec::new();
    for loop_state in &state.loops {
        entries.push(serde_json::to_value(loop_state).map_err(|e| e.to_string())?);
    }
    for completion in &state.completion_attempts {
        entries.push(serde_json::to_value(completion).map_err(|e| e.to_string())?);
    }
    entries.sort_by_key(|v| v.get("loop_number").and_then(|n| n.as_u64()).unwrap_or(0));

    Ok(CallToolResult::success_json(json!({
        "project_id": project_id,
        "loops": entries,
    })))
}

async fn handle_tail(args: Value) -> Result<Value, String> {
    let workspace = Workspace::discover().map_err(map_err)?;
    let project_id = resolve_project_id(&args, &workspace)?;

    let project_dir = workspace.project_dir(&project_id);
    if !project_dir.exists() {
        return Err(format!("project not found: {project_id}"));
    }

    let last = get_usize(&args, "last");
    let events = collect_tail_events(&project_dir, &project_id, last).map_err(map_err)?;

    Ok(CallToolResult::success_json(json!({
        "project_id": project_id,
        "events": events,
    })))
}

async fn handle_quick_prd(args: Value) -> Result<Value, String> {
    let idea = require_str(&args, "idea")?;
    let idea = idea.trim().to_owned();
    if idea.is_empty() {
        return Err("idea must not be empty".to_owned());
    }

    let dry_run = get_bool(&args, "dry_run").unwrap_or(false);

    if dry_run {
        let prompt = render_prompt(DRAFT_PROMPT, &[("{{idea}}", &idea)]);
        return Ok(CallToolResult::success_json(json!({
            "dry_run": true,
            "prompt": prompt,
        })));
    }

    let workspace = Workspace::discover().map_err(map_err)?;

    let writer_spec = get_str(&args, "writer_backend").unwrap_or_else(|| "claude".to_owned());
    let reviewer_spec = get_str(&args, "reviewer_backend").unwrap_or_else(|| "codex".to_owned());
    let max_revisions = get_u32(&args, "max_revisions").unwrap_or(1);

    crate::cli::backend_spec::validate_backend_spec(&writer_spec, &workspace.config)
        .map_err(map_err)?;
    crate::cli::backend_spec::validate_backend_spec(&reviewer_spec, &workspace.config)
        .map_err(map_err)?;

    let mut registry = crate::backend::BackendRegistry::new(
        &workspace.config,
        crate::backend::BackendRegistryTmuxConfig {
            enabled: false,
            session_name: workspace.config.workspace.tmux_session.clone(),
            window_keep_seconds: workspace.config.workspace.tmux_window_keep_seconds,
        },
    );

    let writer = registry
        .get_or_create_for_spec(&writer_spec)
        .map_err(map_err)?;
    let reviewer = registry
        .get_or_create_for_spec(&reviewer_spec)
        .map_err(map_err)?;

    writer.health_check().await.map_err(map_err)?;
    reviewer.health_check().await.map_err(map_err)?;

    let options = QuickPrdOptions {
        idea,
        writer_spec,
        reviewer_spec,
        max_revisions,
        dry_run: false,
    };

    let pipeline = QuickPrdPipeline::new(writer, reviewer, options);
    let result = pipeline.run().await.map_err(map_err)?;

    Ok(CallToolResult::success_json(json!({
        "spec_path": result.spec_path.to_string_lossy(),
        "summary": result.summary,
        "revision_count": result.revision_count,
        "approved": result.approved,
    })))
}

async fn handle_config_show(args: Value) -> Result<Value, String> {
    let global_flag = get_bool(&args, "global").unwrap_or(false);
    let project_arg = get_str(&args, "project");

    if global_flag && project_arg.is_some() {
        return Err("project and global are mutually exclusive".to_owned());
    }

    let workspace = Workspace::discover().map_err(map_err)?;

    if global_flag {
        let value = serde_json::to_value(&workspace.config).map_err(|e| e.to_string())?;
        return Ok(CallToolResult::success_json(json!({
            "scope": "global",
            "config": value,
        })));
    }

    // Try to resolve project scope
    let project_id = if let Some(id) = project_arg {
        Some(id)
    } else {
        workspace.index.active_project.clone()
    };

    if let Some(project_id) = project_id {
        if workspace.index.get_project(&project_id).is_none() {
            return Err(format!("project not found: {project_id}"));
        }

        let project_dir = workspace.project_dir(&project_id);
        let project_config = load_project_config_if_exists(&project_dir).map_err(map_err)?;
        let effective = resolve_effective_config(
            &workspace.root,
            &project_dir,
            workspace.config.clone(),
            project_config.clone(),
            RunWorkflowOverrides::default(),
        )
        .map_err(map_err)?;

        let value = json!({
            "scope": {
                "type": "project",
                "project": project_id,
            },
            "workspace": effective.global.workspace,
            "backends": effective.global.backends,
            "workflow": {
                "starting_backend": effective.workflow.starting_backend,
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
                "completer": effective.templates.completer,
                "qa": effective.templates.qa,
            },
            "git": effective.global.git,
            "project_overrides": project_config,
        });
        return Ok(CallToolResult::success_json(value));
    }

    // Fall back to global scope
    let value = serde_json::to_value(&workspace.config).map_err(|e| e.to_string())?;
    Ok(CallToolResult::success_json(json!({
        "scope": "global",
        "config": value,
    })))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::mcp::protocol::CallToolResult as CTR;

    #[tokio::test]
    async fn unknown_tool_returns_error() {
        let result = handle_tool_call("bogus", json!({})).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("unknown tool"));
    }

    #[tokio::test]
    async fn project_new_requires_id_and_name() {
        let result = handle_tool_call("project_new", json!({})).await;
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .contains("missing required argument: id"));
    }

    #[tokio::test]
    async fn project_new_requires_prompt_source() {
        let result = handle_tool_call("project_new", json!({ "id": "test", "name": "Test" })).await;
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .contains("exactly one of prompt_file or from_project"));
    }

    #[tokio::test]
    async fn project_new_rejects_both_prompt_sources() {
        let result = handle_tool_call(
            "project_new",
            json!({
                "id": "test",
                "name": "Test",
                "prompt_file": "/tmp/prompt.md",
                "from_project": "parent"
            }),
        )
        .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("mutually exclusive"));
    }

    #[tokio::test]
    async fn project_new_validates_backend() {
        let result = handle_tool_call(
            "project_new",
            json!({
                "id": "test",
                "name": "Test",
                "prompt_file": "/tmp/prompt.md",
                "backend": "unknown(foo)"
            }),
        )
        .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("unknown backend"));
    }

    #[tokio::test]
    async fn run_rejects_invalid_on_prompt_change() {
        let result = handle_tool_call("run", json!({ "on_prompt_change": "invalid-value" })).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("invalid on_prompt_change"));
    }

    #[tokio::test]
    async fn quick_prd_requires_idea() {
        let result = handle_tool_call("quick_prd", json!({})).await;
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .contains("missing required argument: idea"));
    }

    #[tokio::test]
    async fn quick_prd_rejects_empty_idea() {
        let result = handle_tool_call("quick_prd", json!({ "idea": "  " })).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("must not be empty"));
    }

    #[tokio::test]
    async fn quick_prd_dry_run_returns_prompt() {
        let result = handle_tool_call(
            "quick_prd",
            json!({ "idea": "add retry logic", "dry_run": true }),
        )
        .await;
        assert!(result.is_ok());
        let val = result.unwrap();
        let parsed: CTR = serde_json::from_value(val).unwrap();
        assert!(!parsed.is_error);
        let text = &parsed.content[0].text;
        let inner: Value = serde_json::from_str(text).unwrap();
        assert_eq!(inner["dry_run"], true);
        assert!(inner["prompt"]
            .as_str()
            .unwrap()
            .contains("add retry logic"));
    }

    #[tokio::test]
    async fn config_show_rejects_both_global_and_project() {
        let result = handle_tool_call(
            "config_show",
            json!({ "global": true, "project": "my-project" }),
        )
        .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("mutually exclusive"));
    }

    #[tokio::test]
    async fn extra_arguments_are_ignored() {
        // quick_prd dry_run should succeed even with extra unknown arguments
        let result = handle_tool_call(
            "quick_prd",
            json!({
                "idea": "test idea",
                "dry_run": true,
                "unknown_extra_arg": "should be ignored",
                "another_unknown": 42
            }),
        )
        .await;
        assert!(result.is_ok());
    }
}
