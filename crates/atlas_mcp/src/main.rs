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

    let base_url = std::env::var("ATLAS_BASE_URL")
        .ok()
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| "http://localhost:8080".to_string());

    match cli.transport {
        Transport::Stdio => run_stdio(base_url).await,
        Transport::Http => run_http(base_url, cli.bind, cli.port).await,
    }
}

async fn run_stdio(base_url: String) -> anyhow::Result<()> {
    let token =
        std::env::var("ATLAS_TOKEN").map_err(|_| anyhow!("ATLAS_TOKEN is required but not set"))?;

    let server = AtlasMcp::new(base_url, token)?;

    log_startup_identity(&server).await;

    let mcp_server = server.serve(stdio()).await?;
    mcp_server.waiting().await?;

    Ok(())
}

/// Best-effort identity probe for stdio mode.
///
/// Logs the authenticated principal to stderr but never aborts startup: a backend
/// that is unreachable at launch or a rejected token must not break the MCP
/// handshake (the client would only see an opaque connection error). Individual
/// tool calls surface auth and connection failures with actionable messages.
async fn log_startup_identity(server: &AtlasMcp) {
    let client = match server.client() {
        Ok(client) => client,
        Err(e) => {
            tracing::warn!("skipping startup identity probe: {e}");
            return;
        }
    };

    match client.me().await {
        Ok(me) if me.principal_type == "api_key" => {
            tracing::info!("authenticated as api_key agent");
        }
        Ok(me) => {
            tracing::warn!(
                principal_type = %me.principal_type,
                "token is not an API key; attribution will be a user, not an agent"
            );
        }
        Err(e) => {
            tracing::warn!(
                "startup identity probe failed; continuing (tool calls will report auth/connection errors): {e}"
            );
        }
    }
}

/// Axum middleware that enforces `Authorization: Bearer atlas_<token>` on all requests.
///
/// Returns HTTP 401 when the header is absent or does not carry a valid `atlas_`-prefixed
/// Bearer token. Passes through to the next handler when the header is present and valid.
/// This provides early rejection at the HTTP boundary before rmcp processes the request.
async fn bearer_auth_middleware(
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    use axum::response::IntoResponse;

    let auth_result = request
        .headers()
        .get(http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .map(atlas_mcp::parse_bearer_atlas_token);

    match auth_result {
        Some(Ok(_)) => next.run(request).await,
        Some(Err(reason)) => {
            tracing::warn!(reason = %reason, "rejected request with invalid Bearer token");
            (
                http::StatusCode::UNAUTHORIZED,
                [("WWW-Authenticate", "Bearer realm=\"atlas-mcp\"")],
                reason,
            )
                .into_response()
        }
        None => {
            tracing::warn!("rejected request with missing Authorization header");
            (
                http::StatusCode::UNAUTHORIZED,
                [("WWW-Authenticate", "Bearer realm=\"atlas-mcp\"")],
                "Authorization header required: provide 'Authorization: Bearer atlas_<token>'",
            )
                .into_response()
        }
    }
}

fn build_http_router(base_url: String, bind: String) -> anyhow::Result<axum::Router> {
    use rmcp::transport::{
        StreamableHttpServerConfig,
        streamable_http_server::{
            session::never::NeverSessionManager, tower::StreamableHttpService,
        },
    };
    use std::sync::Arc;

    let handler = AtlasMcp::new_http(base_url)?;

    let session_manager: Arc<NeverSessionManager> = Arc::default();

    let config = StreamableHttpServerConfig::default()
        .with_stateful_mode(false)
        .with_json_response(true)
        .with_allowed_hosts([
            bind,
            "127.0.0.1".to_string(),
            "::1".to_string(),
            "localhost".to_string(),
        ]);

    let service = StreamableHttpService::new(move || Ok(handler.clone()), session_manager, config);

    Ok(axum::Router::new()
        .nest_service("/mcp", service)
        .layer(axum::middleware::from_fn(bearer_auth_middleware)))
}

async fn run_http(base_url: String, bind: String, port: u16) -> anyhow::Result<()> {
    let router = build_http_router(base_url, bind.clone())?;
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

#[cfg(test)]
mod tests {
    use axum::{Json, Router, http::HeaderMap, routing::get};

    use super::*;

    const INITIALIZE_REQUEST: &str = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"test","version":"1.0"}}}"#;
    const TOOL_CALL_REQUEST: &str = r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"get_agent_identity","arguments":{}}}"#;

    async fn spawn_router(
        router: Router,
    ) -> anyhow::Result<(String, tokio::task::JoinHandle<anyhow::Result<()>>)> {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let address = listener.local_addr()?;
        let server = tokio::spawn(async move {
            axum::serve(listener, router).await?;

            Ok(())
        });

        Ok((format!("http://{address}"), server))
    }

    async fn mock_identity(headers: HeaderMap) -> Json<serde_json::Value> {
        let name = match headers
            .get(http::header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
        {
            Some("Bearer atlas_first") => "first-agent",
            Some("Bearer atlas_second") => "second-agent",
            _ => "unexpected-agent",
        };

        Json(serde_json::json!({
            "principal_type": "api_key",
            "username": name,
            "email": null,
            "id": null,
            "display_name": null,
            "is_root": false,
            "is_system_admin": false,
            "agent": {
                "id": "0197f3f5-70be-7000-8000-000000000001",
                "name": name,
                "scopes": []
            }
        }))
    }

    async fn post_mcp(
        client: &reqwest::Client,
        url: &str,
        token: &str,
        body: &'static str,
    ) -> anyhow::Result<reqwest::Response> {
        Ok(client
            .post(url)
            .header(http::header::AUTHORIZATION, format!("Bearer {token}"))
            .header(http::header::CONTENT_TYPE, "application/json")
            .header(http::header::ACCEPT, "application/json, text/event-stream")
            .body(body)
            .send()
            .await?)
    }

    #[tokio::test]
    async fn stateless_http_initialize_does_not_issue_session_id() -> anyhow::Result<()> {
        let router = build_http_router("http://127.0.0.1:1".to_string(), "127.0.0.1".to_string())?;
        let (base_url, server) = spawn_router(router).await?;
        let client = reqwest::Client::new();

        let response = post_mcp(
            &client,
            &format!("{base_url}/mcp"),
            "atlas_test",
            INITIALIZE_REQUEST,
        )
        .await?;

        assert_eq!(response.status(), http::StatusCode::OK);
        assert!(response.headers().get("mcp-session-id").is_none());
        assert_eq!(
            response
                .headers()
                .get(http::header::CONTENT_TYPE)
                .and_then(|value| value.to_str().ok()),
            Some("application/json")
        );

        server.abort();

        Ok(())
    }

    #[tokio::test]
    async fn stateless_http_ignores_stale_session_and_resolves_bearer_per_request()
    -> anyhow::Result<()> {
        let backend = Router::new().route("/api/auth/me", get(mock_identity));
        let (backend_url, backend_server) = spawn_router(backend).await?;
        let router = build_http_router(backend_url, "127.0.0.1".to_string())?;
        let (base_url, mcp_server) = spawn_router(router).await?;
        let client = reqwest::Client::new();
        let mcp_url = format!("{base_url}/mcp");

        for (token, expected_name) in [
            ("atlas_first", "first-agent"),
            ("atlas_second", "second-agent"),
        ] {
            let response = client
                .post(&mcp_url)
                .header(http::header::AUTHORIZATION, format!("Bearer {token}"))
                .header(http::header::CONTENT_TYPE, "application/json")
                .header(http::header::ACCEPT, "application/json, text/event-stream")
                .header("mcp-session-id", "stale-session")
                .body(TOOL_CALL_REQUEST)
                .send()
                .await?;

            assert_eq!(response.status(), http::StatusCode::OK);
            let body = response.text().await?;
            assert!(body.contains(expected_name), "response was {body}");
        }

        mcp_server.abort();
        backend_server.abort();

        Ok(())
    }
}
