use serde_json::Value;
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWrite, AsyncWriteExt};

use crate::mcp::protocol::{JsonRpcMessage, JsonRpcResponse};
use crate::Result;

pub struct StdioTransport<R: AsyncBufRead + Unpin, W: AsyncWrite + Unpin> {
    reader: R,
    writer: W,
}

impl<R: AsyncBufRead + Unpin, W: AsyncWrite + Unpin> StdioTransport<R, W> {
    pub fn new(reader: R, writer: W) -> Self {
        Self { reader, writer }
    }

    pub async fn read_message(&mut self) -> Result<Option<JsonRpcMessage>> {
        let mut line = String::new();

        loop {
            line.clear();
            let read = self.reader.read_line(&mut line).await?;
            if read == 0 {
                return Ok(None);
            }

            let payload = line.trim_end_matches(['\r', '\n']);
            if payload.trim().is_empty() {
                continue;
            }

            match serde_json::from_str::<JsonRpcMessage>(payload) {
                Ok(message) => return Ok(Some(message)),
                Err(_) => {
                    let response = JsonRpcResponse::error(Value::Null, -32700, "Parse error", None);
                    self.write_message(&response).await?;
                }
            }
        }
    }

    pub async fn write_message(&mut self, message: &JsonRpcResponse) -> Result<()> {
        let encoded = serde_json::to_string(message)?;
        self.writer.write_all(encoded.as_bytes()).await?;
        self.writer.write_all(b"\n").await?;
        self.writer.flush().await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use serde_json::{json, Value};
    use tokio::io::{AsyncReadExt, AsyncWriteExt, BufReader};

    use super::StdioTransport;
    use crate::mcp::protocol::JsonRpcResponse;
    use crate::Result;

    #[tokio::test]
    async fn reads_valid_message_and_writes_response() -> Result<()> {
        let (mut request_client, request_server) = tokio::io::duplex(4096);
        let (response_server, mut response_client) = tokio::io::duplex(4096);

        let mut transport = StdioTransport::new(BufReader::new(request_server), response_server);

        request_client
            .write_all(b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"ping\"}\n")
            .await?;
        request_client.shutdown().await?;

        let message = transport
            .read_message()
            .await?
            .expect("expected one JSON-RPC message");
        assert_eq!(message.method.as_deref(), Some("ping"));
        assert_eq!(message.id, Some(json!(1)));

        let response = JsonRpcResponse::success(json!(1), json!({ "ok": true }));
        transport.write_message(&response).await?;
        drop(transport);

        let mut raw = String::new();
        response_client.read_to_string(&mut raw).await?;
        assert!(raw.ends_with('\n'));

        let parsed: Value =
            serde_json::from_str(raw.trim_end()).expect("response must be valid JSON");
        assert_eq!(parsed["jsonrpc"], "2.0");
        assert_eq!(parsed["result"]["ok"], true);

        Ok(())
    }

    #[tokio::test]
    async fn malformed_json_emits_parse_error_and_continues() -> Result<()> {
        let (mut request_client, request_server) = tokio::io::duplex(4096);
        let (response_server, mut response_client) = tokio::io::duplex(4096);

        let mut transport = StdioTransport::new(BufReader::new(request_server), response_server);

        request_client
            .write_all(
                b"{\"jsonrpc\":\"2.0\",\"id\":\"bad\",\"method\":}\n{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"ping\"}\n",
            )
            .await?;
        request_client.shutdown().await?;

        let message = transport
            .read_message()
            .await?
            .expect("expected valid message after parse error");
        assert_eq!(message.id, Some(json!(2)));
        assert_eq!(message.method.as_deref(), Some("ping"));

        drop(transport);

        let mut raw = String::new();
        response_client.read_to_string(&mut raw).await?;
        let first_line = raw.lines().next().expect("expected parse-error response");
        let response: Value =
            serde_json::from_str(first_line).expect("parse-error response is JSON");

        assert_eq!(response["id"], Value::Null);
        assert_eq!(response["error"]["code"], -32700);

        Ok(())
    }
}
