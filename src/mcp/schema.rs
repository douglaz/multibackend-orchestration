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
            "description": "Show project details and state",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "project": { "type": "string", "description": "Project ID (defaults to active project)" },
                    "json": { "type": "boolean", "description": "Return machine-readable JSON output" }
                },
                "required": []
            }
        }),
        json!({
            "name": "run",
            "description": "Run orchestration loops for a project",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "project": { "type": "string" },
                    "loops": { "type": "integer", "minimum": 1 },
                    "until_review": { "type": "boolean" },
                    "until_complete": { "type": "boolean" },
                    "dry_run": { "type": "boolean" },
                    "backend": { "type": "string" },
                    "planner_backend": { "type": "string" },
                    "implementer_backend": { "type": "string" },
                    "reviewer_backend": { "type": "string" },
                    "qa_backend": { "type": "string" },
                    "completer_backend": { "type": "string" },
                    "on_prompt_change": { "type": "string" },
                    "skip_commit": { "type": "boolean" }
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
            "description": "Show feature loop history",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "project": { "type": "string" },
                    "verbose": { "type": "boolean" },
                    "json": { "type": "boolean" }
                },
                "required": []
            }
        }),
        json!({
            "name": "tail",
            "description": "Stream recent project artifact events",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "project": { "type": "string" },
                    "last": { "type": "integer", "minimum": 1 },
                    "follow": { "type": "boolean" },
                    "poll_interval_ms": { "type": "integer", "minimum": 1 },
                    "json": { "type": "boolean" },
                    "tmux": { "type": "boolean" }
                },
                "required": []
            }
        }),
        json!({
            "name": "quick_prd",
            "description": "Generate a quick product requirements draft",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "idea": { "type": "string" },
                    "writer_backend": { "type": "string" },
                    "reviewer_backend": { "type": "string" },
                    "max_revisions": { "type": "integer", "minimum": 1 },
                    "dry_run": { "type": "boolean" }
                },
                "required": ["idea"]
            }
        }),
        json!({
            "name": "config_show",
            "description": "Show effective config in project or global scope",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "project": { "type": "string" },
                    "global": { "type": "boolean" }
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
