pub mod handlers;
pub mod protocol;
pub mod schema;
pub mod server;
pub mod tail_events;
pub mod transport;

pub use server::McpServer;

use crate::Result;

pub async fn serve() -> Result<()> {
    let mut server = McpServer::stdio();
    server.run().await
}
