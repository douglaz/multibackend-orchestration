use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct JsonRpcMessage {
    #[serde(default)]
    pub jsonrpc: Option<String>,
    #[serde(default)]
    pub id: Option<Value>,
    #[serde(default)]
    pub method: Option<String>,
    #[serde(default)]
    pub params: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct JsonRpcResponse {
    #[serde(default = "jsonrpc_version")]
    pub jsonrpc: String,
    pub id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct JsonRpcError {
    pub code: i32,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CallToolResult {
    pub content: Vec<ContentBlock>,
    #[serde(rename = "isError")]
    pub is_error: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ContentBlock {
    #[serde(rename = "type")]
    pub kind: String,
    pub text: String,
}

fn jsonrpc_version() -> String {
    "2.0".to_owned()
}

impl JsonRpcResponse {
    pub fn success(id: Value, result: Value) -> Self {
        Self {
            jsonrpc: jsonrpc_version(),
            id,
            result: Some(result),
            error: None,
        }
    }

    pub fn error(id: Value, code: i32, message: impl Into<String>, data: Option<Value>) -> Self {
        Self {
            jsonrpc: jsonrpc_version(),
            id,
            result: None,
            error: Some(JsonRpcError {
                code,
                message: message.into(),
                data,
            }),
        }
    }
}

impl ContentBlock {
    pub fn text(text: impl Into<String>) -> Self {
        Self {
            kind: "text".to_owned(),
            text: text.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{JsonRpcMessage, JsonRpcResponse};

    #[test]
    fn jsonrpc_message_round_trip() {
        let source = json!({
            "jsonrpc": "2.0",
            "id": 42,
            "method": "initialize",
            "params": {
                "clientInfo": {
                    "name": "tester"
                }
            }
        });

        let decoded: JsonRpcMessage =
            serde_json::from_value(source.clone()).expect("message must deserialize");
        let encoded = serde_json::to_value(decoded).expect("message must serialize");

        assert_eq!(encoded, source);
    }

    #[test]
    fn jsonrpc_response_round_trip() {
        let response = JsonRpcResponse::error(json!("abc"), -32601, "Method not found", None);
        let encoded = serde_json::to_value(&response).expect("response must serialize");
        let decoded: JsonRpcResponse =
            serde_json::from_value(encoded.clone()).expect("response must deserialize");

        assert_eq!(decoded, response);
        assert_eq!(encoded["jsonrpc"], "2.0");
    }
}
