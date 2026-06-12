use anyhow::Result;

#[tokio::main]
async fn main() -> Result<()> {
    p4mcp_server_rs::server::run_from_cli().await
}
