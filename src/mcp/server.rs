use serde_json::{json, Value};
use tokio::io::{AsyncBufRead, AsyncWrite, BufReader};

use crate::mcp::handlers;
use crate::mcp::protocol::{CallToolResult, ContentBlock, JsonRpcResponse};
use crate::mcp::schema;
use crate::mcp::transport::StdioTransport;
use crate::Result;

pub struct McpServer<R: AsyncBufRead + Unpin, W: AsyncWrite + Unpin> {
    transport: StdioTransport<R, W>,
    initialized: bool,
}

impl McpServer<BufReader<tokio::io::Stdin>, tokio::io::Stdout> {
    pub fn stdio() -> Self {
        Self::new(StdioTransport::new(
            BufReader::new(tokio::io::stdin()),
            tokio::io::stdout(),
        ))
    }
}

impl<R: AsyncBufRead + Unpin, W: AsyncWrite + Unpin> McpServer<R, W> {
    pub fn new(transport: StdioTransport<R, W>) -> Self {
        Self {
            transport,
            initialized: false,
        }
    }

    pub async fn run(&mut self) -> Result<()> {
        while let Some(message) = self.transport.read_message().await? {
            let request_id = message.id.clone().unwrap_or(Value::Null);
            let Some(method) = message.method.as_deref() else {
                self.write_error(request_id, -32600, "Invalid Request", None)
                    .await?;
                continue;
            };

            if message.jsonrpc.as_deref() != Some("2.0") {
                self.write_error(request_id, -32600, "Invalid Request", None)
                    .await?;
                continue;
            }

            if method == "notifications/initialized" {
                continue;
            }
            if method.starts_with("notifications/") {
                continue;
            }

            match method {
                "initialize" => {
                    let Some(id) = message.id else {
                        self.write_error(Value::Null, -32600, "Invalid Request", None)
                            .await?;
                        continue;
                    };

                    self.initialized = true;
                    self.write_success(
                        id,
                        json!({
                            "protocolVersion": "2025-06-18",
                            "serverInfo": {
                                "name": "ralph",
                                "version": env!("CARGO_PKG_VERSION"),
                            },
                            "capabilities": {
                                "tools": {}
                            }
                        }),
                    )
                    .await?;
                }
                "ping" => {
                    let Some(id) = message.id else {
                        self.write_error(Value::Null, -32600, "Invalid Request", None)
                            .await?;
                        continue;
                    };

                    self.write_success(id, json!({})).await?;
                }
                "tools/list" => {
                    let Some(id) = message.id else {
                        self.write_error(Value::Null, -32600, "Invalid Request", None)
                            .await?;
                        continue;
                    };

                    self.write_success(
                        id,
                        json!({
                            "tools": schema::tool_definitions()
                        }),
                    )
                    .await?;
                }
                "tools/call" => {
                    let Some(id) = message.id else {
                        self.write_error(Value::Null, -32600, "Invalid Request", None)
                            .await?;
                        continue;
                    };

                    let Some((name, arguments)) = extract_tool_call_params(message.params) else {
                        self.write_error(id, -32600, "Invalid Request", None)
                            .await?;
                        continue;
                    };

                    let result = match handlers::handle_tool_call(&name, arguments).await {
                        Ok(value) => value,
                        Err(message) => {
                            match serde_json::to_value(CallToolResult {
                                content: vec![ContentBlock::text(message)],
                                is_error: true,
                            }) {
                                Ok(value) => value,
                                Err(err) => {
                                    self.write_error(
                                        id,
                                        -32603,
                                        "Internal error",
                                        Some(json!({ "detail": err.to_string() })),
                                    )
                                    .await?;
                                    continue;
                                }
                            }
                        }
                    };

                    self.write_success(id, result).await?;
                }
                _ => {
                    if let Some(id) = message.id {
                        self.write_error(id, -32601, "Method not found", None)
                            .await?;
                    }
                }
            }
        }

        Ok(())
    }

    async fn write_success(&mut self, id: Value, result: Value) -> Result<()> {
        self.transport
            .write_message(&JsonRpcResponse::success(id, result))
            .await
    }

    async fn write_error(
        &mut self,
        id: Value,
        code: i32,
        message: &str,
        data: Option<Value>,
    ) -> Result<()> {
        self.transport
            .write_message(&JsonRpcResponse::error(id, code, message, data))
            .await
    }
}

fn extract_tool_call_params(params: Option<Value>) -> Option<(String, Value)> {
    let params = params?;
    let obj = params.as_object()?;

    let name = obj.get("name")?.as_str()?.to_owned();
    let arguments = obj.get("arguments").cloned().unwrap_or_else(|| json!({}));

    Some((name, arguments))
}

#[cfg(test)]
mod tests {
    use serde_json::Value;
    use tokio::io::{AsyncReadExt, AsyncWriteExt, BufReader};

    use super::McpServer;
    use crate::mcp::transport::StdioTransport;
    use crate::Result;

    #[tokio::test]
    async fn server_dispatches_initialize_tools_list_ping_and_unknown_method() -> Result<()> {
        let (mut request_client, request_server) = tokio::io::duplex(16 * 1024);
        let (response_server, mut response_client) = tokio::io::duplex(16 * 1024);

        let input = concat!(
            "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{}}\n",
            "{\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}\n",
            "{\"jsonrpc\":\"2.0\",\"method\":\"notifications/custom\",\"params\":{\"x\":1}}\n",
            "{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/list\"}\n",
            "{\"jsonrpc\":\"2.0\",\"id\":3,\"method\":\"ping\"}\n",
            "{\"jsonrpc\":\"2.0\",\"id\":4,\"method\":\"bogus/method\"}\n",
        );

        request_client.write_all(input.as_bytes()).await?;
        request_client.shutdown().await?;

        let transport = StdioTransport::new(BufReader::new(request_server), response_server);
        let mut server = McpServer::new(transport);
        server.run().await?;
        drop(server);

        let mut raw_output = String::new();
        response_client.read_to_string(&mut raw_output).await?;

        let responses: Vec<Value> = raw_output
            .lines()
            .map(|line| serde_json::from_str(line).expect("response line should be valid JSON"))
            .collect();

        assert_eq!(responses.len(), 4);

        assert_eq!(responses[0]["id"], 1);
        assert_eq!(responses[0]["result"]["protocolVersion"], "2025-06-18");
        assert_eq!(responses[0]["result"]["serverInfo"]["name"], "ralph");
        assert_eq!(
            responses[0]["result"]["capabilities"]["tools"],
            serde_json::json!({})
        );

        assert_eq!(responses[1]["id"], 2);
        assert!(responses[1]["result"]["tools"].is_array());
        assert_eq!(
            responses[1]["result"]["tools"].as_array().map(Vec::len),
            Some(9)
        );

        assert_eq!(responses[2]["id"], 3);
        assert_eq!(responses[2]["result"], serde_json::json!({}));

        assert_eq!(responses[3]["id"], 4);
        assert_eq!(responses[3]["error"]["code"], -32601);

        Ok(())
    }
}
