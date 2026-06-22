use anyhow::anyhow;
use atlas_mcp::AtlasMcp;
use rmcp::{ServiceExt, transport::stdio};
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // stdout is reserved for MCP JSON-RPC framing; tracing must write to stderr only.
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let base_url =
        std::env::var("ATLAS_BASE_URL").unwrap_or_else(|_| "http://localhost:8080".to_string());

    let token =
        std::env::var("ATLAS_TOKEN").map_err(|_| anyhow!("ATLAS_TOKEN is required but not set"))?;

    let server = AtlasMcp::new(base_url, token)?;

    let me = server
        .client()
        .me()
        .await
        .map_err(|e| anyhow!("startup identity probe failed: {e}"))?;

    if me.principal_type == "api_key" {
        tracing::info!("authenticated as api_key agent");
    } else {
        tracing::warn!(
            principal_type = %me.principal_type,
            "token is not an API key; attribution will be a user, not an agent"
        );
    }

    let mcp_server = server.serve(stdio()).await?;
    mcp_server.waiting().await?;

    Ok(())
}
