use anyhow::Result;
use atlas_mcp::AtlasMcp;
use rmcp::{ServiceExt, transport::stdio};

#[tokio::main]
async fn main() -> Result<()> {
    let base_url =
        std::env::var("ATLAS_BASE_URL").unwrap_or_else(|_| "http://localhost:8080".to_string());

    let mut mcp = AtlasMcp::new(base_url);

    if let Ok(token) = std::env::var("ATLAS_TOKEN") {
        mcp = mcp.with_token(token);
    }

    let server = mcp.serve(stdio()).await?;
    server.waiting().await?;
    Ok(())
}
