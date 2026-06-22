use anyhow::anyhow;
use atlas_mcp::AtlasMcp;
use clap::Parser;
use rmcp::{ServiceExt, transport::stdio};
use tracing_subscriber::EnvFilter;

#[derive(Debug, clap::ValueEnum, Clone)]
enum Transport {
    Stdio,
    Http,
}

#[derive(Parser, Debug)]
#[command(about = "Atlas MCP server — stdio (default) or HTTP/Streamable transport")]
struct Cli {
    /// Transport mode: `stdio` (default) or `http`.
    ///
    /// Can also be set via the `ATLAS_MCP_TRANSPORT` environment variable.
    #[arg(long, value_enum, default_value = "stdio", env = "ATLAS_MCP_TRANSPORT")]
    transport: Transport,

    /// Bind address for HTTP mode.
    ///
    /// Ignored in stdio mode. Can also be set via `ATLAS_MCP_BIND`.
    #[arg(long, default_value = "127.0.0.1", env = "ATLAS_MCP_BIND")]
    bind: String,

    /// TCP port for HTTP mode.
    ///
    /// Ignored in stdio mode. Can also be set via `ATLAS_MCP_PORT`.
    #[arg(long, default_value_t = 3001, env = "ATLAS_MCP_PORT")]
    port: u16,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let cli = Cli::parse();

    let base_url =
        std::env::var("ATLAS_BASE_URL").unwrap_or_else(|_| "http://localhost:8080".to_string());

    match cli.transport {
        Transport::Stdio => run_stdio(base_url).await,
        Transport::Http => run_http(base_url, cli.bind, cli.port).await,
    }
}

async fn run_stdio(base_url: String) -> anyhow::Result<()> {
    let token =
        std::env::var("ATLAS_TOKEN").map_err(|_| anyhow!("ATLAS_TOKEN is required but not set"))?;

    let server = AtlasMcp::new(base_url, token)?;

    let me = server
        .client()
        .map_err(|e| anyhow!("{e}"))?
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

async fn run_http(base_url: String, bind: String, port: u16) -> anyhow::Result<()> {
    use rmcp::transport::{
        StreamableHttpServerConfig,
        streamable_http_server::{
            session::local::LocalSessionManager, tower::StreamableHttpService,
        },
    };
    use std::sync::Arc;

    let handler = AtlasMcp::new_http(base_url)?;

    let session_manager: Arc<LocalSessionManager> = Arc::default();

    let config = StreamableHttpServerConfig::default().with_allowed_hosts([
        bind.clone(),
        "127.0.0.1".to_string(),
        "::1".to_string(),
        "localhost".to_string(),
    ]);

    let service = StreamableHttpService::new(move || Ok(handler.clone()), session_manager, config);

    let router = axum::Router::new().nest_service("/mcp", service);
    let addr = (bind.as_str(), port);
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|e| anyhow!("failed to bind {}:{}: {e}", bind, port))?;

    tracing::info!(bind = %bind, port = %port, "HTTP/Streamable MCP server listening");

    axum::serve(listener, router)
        .await
        .map_err(|e| anyhow!("HTTP server error: {e}"))?;

    Ok(())
}
