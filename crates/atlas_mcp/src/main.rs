use anyhow::Result;
use atlas_mcp::AtlasMcp;
use rmcp::{ServiceExt, transport::stdio};

#[tokio::main]
async fn main() -> Result<()> {
    let server = AtlasMcp.serve(stdio()).await?;
    server.waiting().await?;
    Ok(())
}
