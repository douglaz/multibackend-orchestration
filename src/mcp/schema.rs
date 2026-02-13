use serde_json::{json, Value};

pub fn tool_definitions() -> Vec<Value> {
    vec![
        json!({
            "name": "project_new",
            "description": "Create a new project from a prompt file or parent project",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "id": { "type": "string", "description": "Project identifier" },
                    "name": { "type": "string", "description": "Human-readable project name" },
                    "prompt_file": { "type": "string", "description": "Path to prompt markdown file" },
                    "from_project": { "type": "string", "description": "Existing parent project ID" },
                    "backend": { "type": "string", "description": "Starting backend override" }
                },
                "required": ["id", "name"],
                "oneOf": [
                    {
                        "required": ["prompt_file"],
                        "not": { "required": ["from_project"] }
                    },
                    {
                        "required": ["from_project"],
                        "not": { "required": ["prompt_file"] }
                    }
                ]
            }
        }),
        json!({
            "name": "project_list",
            "description": "List projects in the workspace",
            "inputSchema": {
                "type": "object",
                "properties": {},
                "required": []
            }
        }),
        json!({
            "name": "project_show",
            "description": "Show project details and state (always returns JSON)",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "project": { "type": "string", "description": "Project ID (defaults to active project)" }
                },
                "required": []
            }
        }),
        json!({
            "name": "run",
            "description": "Run orchestration loops for a project (tmux is not available via MCP)",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "project": { "type": "string", "description": "Project ID (defaults to active project)" },
                    "loops": { "type": "integer", "minimum": 1, "description": "Number of feature loops to run" },
                    "until_review": { "type": "boolean", "description": "Stop after first review" },
                    "until_complete": { "type": "boolean", "description": "Run until completion check" },
                    "dry_run": { "type": "boolean", "description": "Preview without executing" },
                    "backend": { "type": "string", "description": "Default backend for all roles" },
                    "planner_backend": { "type": "string", "description": "Backend for planner role" },
                    "implementer_backend": { "type": "string", "description": "Backend for implementer role" },
                    "reviewer_backend": { "type": "string", "description": "Backend for reviewer role" },
                    "qa_backend": { "type": "string", "description": "Backend for QA role" },
                    "completer_backend": { "type": "string", "description": "Backend for completer role" },
                    "on_prompt_change": { "type": "string", "description": "Action on prompt change: continue, restart-loop, abort", "enum": ["continue", "restart-loop", "abort"] },
                    "skip_commit": { "type": "boolean", "description": "Skip git commit after loop completion" }
                },
                "required": []
            }
        }),
        json!({
            "name": "status",
            "description": "Show current project status",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "project": { "type": "string" }
                },
                "required": []
            }
        }),
        json!({
            "name": "history",
            "description": "Show feature loop history (always returns JSON)",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "project": { "type": "string", "description": "Project ID (defaults to active project)" }
                },
                "required": []
            }
        }),
        json!({
            "name": "tail",
            "description": "Return recent project artifact events as JSON (follow/tmux not available via MCP)",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "project": { "type": "string", "description": "Project ID (defaults to active project)" },
                    "last": { "type": "integer", "minimum": 1, "description": "Return only the N most recent events" }
                },
                "required": []
            }
        }),
        json!({
            "name": "quick_prd",
            "description": "Generate a quick product requirements draft (always non-interactive via MCP)",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "idea": { "type": "string", "description": "Feature idea to generate a PRD for" },
                    "writer_backend": { "type": "string", "description": "Backend for writing the spec (default: claude)" },
                    "reviewer_backend": { "type": "string", "description": "Backend for reviewing the spec (default: codex)" },
                    "max_revisions": { "type": "integer", "minimum": 1, "description": "Maximum revision iterations" },
                    "dry_run": { "type": "boolean", "description": "Return rendered prompt without executing backends" }
                },
                "required": ["idea"]
            }
        }),
        json!({
            "name": "config_show",
            "description": "Show effective config (project and global are mutually exclusive; defaults to active project if set, otherwise global)",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "project": { "type": "string", "description": "Project ID to show config for" },
                    "global": { "type": "boolean", "description": "Show global config only" }
                },
                "required": [],
                "not": {
                    "required": ["project", "global"]
                }
            }
        }),
    ]
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use serde_json::Value;

    use super::tool_definitions;

    #[test]
    fn contains_all_expected_tools() {
        let tools = tool_definitions();
        assert_eq!(tools.len(), 9);

        let names: HashSet<&str> = tools
            .iter()
            .map(|tool| tool["name"].as_str().expect("tool name should be a string"))
            .collect();

        let expected = HashSet::from([
            "project_new",
            "project_list",
            "project_show",
            "run",
            "status",
            "history",
            "tail",
            "quick_prd",
            "config_show",
        ]);

        assert_eq!(names, expected);
    }

    #[test]
    fn each_tool_has_required_schema_shape() {
        for tool in tool_definitions() {
            assert!(tool["name"].is_string());
            assert!(tool["description"].is_string());

            let schema = &tool["inputSchema"];
            assert_eq!(schema["type"], Value::String("object".to_owned()));
            assert!(schema["properties"].is_object());
            assert!(schema["required"].is_array());
        }
    }

    #[test]
    fn run_excludes_tmux_and_includes_on_prompt_change_enum() {
        let tools = tool_definitions();
        let run = tools
            .iter()
            .find(|tool| tool["name"] == "run")
            .expect("run must exist");
        let props = &run["inputSchema"]["properties"];
        assert!(props.get("tmux").is_none());
        assert!(props.get("no_tmux").is_none());
        assert!(props["on_prompt_change"]["enum"].is_array());
    }

    #[test]
    fn tail_excludes_cli_only_controls() {
        let tools = tool_definitions();
        let tail = tools
            .iter()
            .find(|tool| tool["name"] == "tail")
            .expect("tail must exist");
        let props = &tail["inputSchema"]["properties"];
        assert!(props.get("follow").is_none());
        assert!(props.get("poll_interval_ms").is_none());
        assert!(props.get("json").is_none());
        assert!(props.get("tmux").is_none());
    }

    #[test]
    fn quick_prd_excludes_interactive_flags() {
        let tools = tool_definitions();
        let quick_prd = tools
            .iter()
            .find(|tool| tool["name"] == "quick_prd")
            .expect("quick_prd must exist");
        let props = &quick_prd["inputSchema"]["properties"];
        assert!(props.get("interactive").is_none());
        assert!(props.get("non_interactive").is_none());
    }

    #[test]
    fn project_new_and_config_show_include_mutual_exclusion_constraints() {
        let tools = tool_definitions();

        let project_new = tools
            .iter()
            .find(|tool| tool["name"] == "project_new")
            .expect("project_new must exist");
        assert!(project_new["inputSchema"]["oneOf"].is_array());

        let config_show = tools
            .iter()
            .find(|tool| tool["name"] == "config_show")
            .expect("config_show must exist");
        assert_eq!(
            config_show["inputSchema"]["not"]["required"],
            serde_json::json!(["project", "global"])
        );
    }
}
