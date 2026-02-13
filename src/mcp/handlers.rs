use serde_json::Value;

pub async fn handle_tool_call(name: &str, _arguments: Value) -> Result<Value, String> {
    match name {
        "project_new" | "project_list" | "project_show" | "run" | "status" | "history" | "tail"
        | "quick_prd" | "config_show" => Err("not yet implemented".to_owned()),
        _ => Err(format!("unknown tool: {name}")),
    }
}
