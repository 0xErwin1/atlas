#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

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

/// Browser origins allowed to reach `/mcp`.
///
/// Atlas MCP has no browser client: every caller is an agent or the CLI, and
/// those send no `Origin`, which rmcp lets through untouched. Listing only the
/// addresses the server itself answers on therefore rejects any cross-origin
/// browser request outright without affecting a real caller. Leaving the list
/// empty would disable the check entirely, which is what rmcp does by default.
fn allowed_origins(bind: &str, port: u16) -> Vec<String> {
    let mut hosts = vec![
        "127.0.0.1".to_string(),
        "[::1]".to_string(),
        "localhost".to_string(),
    ];

    if !hosts.iter().any(|host| host == bind) {
        hosts.push(bind.to_string());
    }

    hosts
        .into_iter()
        .map(|host| format!("http://{host}:{port}"))
        .collect()
}

fn build_http_router(base_url: String, bind: String, port: u16) -> anyhow::Result<axum::Router> {
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
        .with_legacy_session_mode(false)
        .with_json_response(true)
        .with_allowed_hosts([
            bind.clone(),
            "127.0.0.1".to_string(),
            "::1".to_string(),
            "localhost".to_string(),
        ])
        .with_allowed_origins(allowed_origins(&bind, port));

    let service = StreamableHttpService::new(move || Ok(handler.clone()), session_manager, config);

    Ok(axum::Router::new()
        .nest_service("/mcp", service)
        .layer(axum::middleware::from_fn(bearer_auth_middleware)))
}

async fn run_http(base_url: String, bind: String, port: u16) -> anyhow::Result<()> {
    let router = build_http_router(base_url, bind.clone(), port)?;
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
    const TOOL_CALL_REQUEST: &str = r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"identity","arguments":{"resource":"agent","params":{}}}}"#;
    const TOOLS_LIST_REQUEST: &str =
        r#"{"jsonrpc":"2.0","id":3,"method":"tools/list","params":{}}"#;
    const TEMPLATES_LIST_REQUEST: &str =
        r#"{"jsonrpc":"2.0","id":4,"method":"resources/templates/list","params":{}}"#;
    /// Port the routers under test are configured for. The listener binds an
    /// ephemeral port instead, which only matters for `Origin` validation: no
    /// test client sends an `Origin`, and the one that does sends a foreign one.
    const TEST_PORT: u16 = 3001;
    const MODERN_VERSION: &str = "2026-07-28";
    const LEGACY_VERSION: &str = "2025-11-25";

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

    async fn mock_meta(headers: HeaderMap) -> Json<serde_json::Value> {
        let enabled = headers
            .get(http::header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            != Some("Bearer atlas_disabled");

        Json(serde_json::json!({
            "version": "1",
            "build": null,
            "semantic_search_enabled": enabled
        }))
    }

    async fn mock_document() -> Json<serde_json::Value> {
        Json(serde_json::json!({
            "id": "0197f3f5-70be-7000-8000-000000000010",
            "workspace_id": "0197f3f5-70be-7000-8000-000000000011",
            "slug": "notes",
            "title": "Notes",
            "content": "# Notes\n",
            "head_revision_id": "0197f3f5-70be-7000-8000-000000000012",
            "head_seq": 1,
            "frontmatter": {},
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-01T00:00:00Z"
        }))
    }

    async fn post_mcp(
        client: &reqwest::Client,
        url: &str,
        token: &str,
        body: impl Into<reqwest::Body>,
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

    /// Builds a `2026-07-28` request body carrying the per-request protocol
    /// metadata SEP-2575 makes mandatory on that revision.
    fn modern_request(id: u32, method: &str, params: serde_json::Value) -> String {
        let mut params = match params {
            serde_json::Value::Object(map) => map,
            other => panic!("request params must be a JSON object, got {other}"),
        };

        params.insert(
            "_meta".to_string(),
            serde_json::json!({
                "io.modelcontextprotocol/protocolVersion": MODERN_VERSION,
                "io.modelcontextprotocol/clientCapabilities": {}
            }),
        );

        serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params
        })
        .to_string()
    }

    /// Mirrors the SEP-2243 `Mcp-Name` source for the methods that carry one.
    fn mcp_name_for(method: &str, params: &serde_json::Value) -> Option<String> {
        let key = match method {
            "tools/call" | "prompts/get" => "name",
            "resources/read" => "uri",
            _ => return None,
        };

        params
            .get(key)
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned)
    }

    async fn post_modern(
        client: &reqwest::Client,
        url: &str,
        token: &str,
        id: u32,
        method: &str,
        params: serde_json::Value,
    ) -> anyhow::Result<reqwest::Response> {
        let mut request = client
            .post(url)
            .header(http::header::AUTHORIZATION, format!("Bearer {token}"))
            .header(http::header::CONTENT_TYPE, "application/json")
            .header(http::header::ACCEPT, "application/json, text/event-stream")
            .header("MCP-Protocol-Version", MODERN_VERSION)
            .header("Mcp-Method", method);

        if let Some(name) = mcp_name_for(method, &params) {
            request = request.header("Mcp-Name", name);
        }

        Ok(request
            .body(modern_request(id, method, params))
            .send()
            .await?)
    }

    #[tokio::test]
    async fn stateless_http_initialize_does_not_issue_session_id() -> anyhow::Result<()> {
        let router = build_http_router(
            "http://127.0.0.1:1".to_string(),
            "127.0.0.1".to_string(),
            TEST_PORT,
        )?;
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
        let backend = Router::new().route("/api/v2/custos/auth/me", get(mock_identity));
        let (backend_url, backend_server) = spawn_router(backend).await?;
        let router = build_http_router(backend_url, "127.0.0.1".to_string(), TEST_PORT)?;
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

    #[tokio::test]
    async fn stateless_http_tools_list_offers_the_verb_shaped_catalog() -> anyhow::Result<()> {
        let backend = Router::new().route("/api/v2/platform/meta", get(mock_meta));
        let (backend_url, backend_server) = spawn_router(backend).await?;
        let router = build_http_router(backend_url, "127.0.0.1".to_string(), TEST_PORT)?;
        let (base_url, mcp_server) = spawn_router(router).await?;
        let client = reqwest::Client::new();
        let mcp_url = format!("{base_url}/mcp");

        for token in ["atlas_disabled", "atlas_enabled"] {
            let response = post_mcp(&client, &mcp_url, token, TOOLS_LIST_REQUEST).await?;

            assert_eq!(response.status(), http::StatusCode::OK);
            let body: serde_json::Value = response.json().await?;
            let tools = body
                .pointer("/result/tools")
                .and_then(serde_json::Value::as_array)
                .ok_or_else(|| anyhow!("tools/list response does not contain tools: {body}"))?;
            let names: Vec<&str> = tools
                .iter()
                .filter_map(|tool| tool.get("name").and_then(serde_json::Value::as_str))
                .collect();
            assert!(names.contains(&"find"));
            assert!(names.contains(&"help"));
            assert!(
                !names.contains(&"semantic_search"),
                "semantic_search was merged into search's mode parameter"
            );
            assert!(
                !names.contains(&"list_tasks"),
                "the catalog is verb-shaped: {names:?}"
            );
        }

        mcp_server.abort();
        backend_server.abort();

        Ok(())
    }

    #[tokio::test]
    async fn initialize_negotiates_the_2026_07_28_revision() -> anyhow::Result<()> {
        let router = build_http_router(
            "http://127.0.0.1:1".to_string(),
            "127.0.0.1".to_string(),
            TEST_PORT,
        )?;
        let (base_url, server) = spawn_router(router).await?;
        let client = reqwest::Client::new();

        let response = client
            .post(format!("{base_url}/mcp"))
            .header(http::header::AUTHORIZATION, "Bearer atlas_test")
            .header(http::header::CONTENT_TYPE, "application/json")
            .header(http::header::ACCEPT, "application/json, text/event-stream")
            .header("MCP-Protocol-Version", MODERN_VERSION)
            .body(format!(
                r#"{{"jsonrpc":"2.0","id":1,"method":"initialize","params":{{"protocolVersion":"{MODERN_VERSION}","capabilities":{{}},"clientInfo":{{"name":"test","version":"1.0"}}}}}}"#
            ))
            .send()
            .await?;

        assert_eq!(response.status(), http::StatusCode::OK);
        let body: serde_json::Value = response.json().await?;
        assert_eq!(
            body.pointer("/result/protocolVersion")
                .and_then(serde_json::Value::as_str),
            Some(MODERN_VERSION),
            "response was {body}"
        );

        server.abort();

        Ok(())
    }

    #[tokio::test]
    async fn initialize_still_negotiates_the_legacy_era() -> anyhow::Result<()> {
        let router = build_http_router(
            "http://127.0.0.1:1".to_string(),
            "127.0.0.1".to_string(),
            TEST_PORT,
        )?;
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
        let body: serde_json::Value = response.json().await?;
        assert_eq!(
            body.pointer("/result/protocolVersion")
                .and_then(serde_json::Value::as_str),
            Some("2025-03-26"),
            "response was {body}"
        );

        server.abort();

        Ok(())
    }

    #[tokio::test]
    async fn server_discover_advertises_both_protocol_eras() -> anyhow::Result<()> {
        let router = build_http_router(
            "http://127.0.0.1:1".to_string(),
            "127.0.0.1".to_string(),
            TEST_PORT,
        )?;
        let (base_url, server) = spawn_router(router).await?;
        let client = reqwest::Client::new();

        let response = post_modern(
            &client,
            &format!("{base_url}/mcp"),
            "atlas_test",
            1,
            "server/discover",
            serde_json::json!({}),
        )
        .await?;

        assert_eq!(response.status(), http::StatusCode::OK);
        let body: serde_json::Value = response.json().await?;
        let result = body
            .get("result")
            .ok_or_else(|| anyhow!("server/discover returned no result: {body}"))?;

        let versions: Vec<&str> = result
            .get("supportedVersions")
            .and_then(serde_json::Value::as_array)
            .ok_or_else(|| anyhow!("discover result has no supportedVersions: {body}"))?
            .iter()
            .filter_map(serde_json::Value::as_str)
            .collect();
        assert!(
            versions.contains(&MODERN_VERSION),
            "advertised {versions:?}"
        );
        assert!(
            versions.contains(&LEGACY_VERSION),
            "advertised {versions:?}"
        );

        assert_eq!(
            result.get("resultType").and_then(serde_json::Value::as_str),
            Some("complete")
        );
        assert!(
            result
                .get("ttlMs")
                .and_then(serde_json::Value::as_u64)
                .is_some(),
            "discover result must carry ttlMs: {body}"
        );
        assert!(
            result
                .get("cacheScope")
                .and_then(serde_json::Value::as_str)
                .is_some(),
            "discover result must carry cacheScope: {body}"
        );
        assert!(
            result
                .pointer("/capabilities/tools")
                .is_some_and(|tools| !tools.is_null()),
            "discover must advertise the tools capability: {body}"
        );
        assert!(
            result.pointer("/capabilities/prompts").is_none(),
            "Atlas does not implement prompts: {body}"
        );

        server.abort();

        Ok(())
    }

    #[tokio::test]
    async fn modern_resource_templates_list_carries_public_cache_hints() -> anyhow::Result<()> {
        let router = build_http_router(
            "http://127.0.0.1:1".to_string(),
            "127.0.0.1".to_string(),
            TEST_PORT,
        )?;
        let (base_url, server) = spawn_router(router).await?;
        let client = reqwest::Client::new();

        let response = post_modern(
            &client,
            &format!("{base_url}/mcp"),
            "atlas_test",
            1,
            "resources/templates/list",
            serde_json::json!({}),
        )
        .await?;

        assert_eq!(response.status(), http::StatusCode::OK);
        let body: serde_json::Value = response.json().await?;
        let result = body
            .get("result")
            .ok_or_else(|| anyhow!("templates/list returned no result: {body}"))?;

        assert_eq!(
            result.get("resultType").and_then(serde_json::Value::as_str),
            Some("complete"),
            "response was {body}"
        );
        assert!(
            result
                .get("ttlMs")
                .and_then(serde_json::Value::as_u64)
                .is_some(),
            "response was {body}"
        );
        assert_eq!(
            result.get("cacheScope").and_then(serde_json::Value::as_str),
            Some("public"),
            "the resource template catalog is identical for every caller: {body}"
        );

        server.abort();

        Ok(())
    }

    #[tokio::test]
    async fn legacy_resource_templates_list_omits_cache_hints() -> anyhow::Result<()> {
        let router = build_http_router(
            "http://127.0.0.1:1".to_string(),
            "127.0.0.1".to_string(),
            TEST_PORT,
        )?;
        let (base_url, server) = spawn_router(router).await?;
        let client = reqwest::Client::new();

        let response = post_mcp(
            &client,
            &format!("{base_url}/mcp"),
            "atlas_test",
            TEMPLATES_LIST_REQUEST,
        )
        .await?;

        assert_eq!(response.status(), http::StatusCode::OK);
        let body: serde_json::Value = response.json().await?;
        let result = body
            .get("result")
            .ok_or_else(|| anyhow!("templates/list returned no result: {body}"))?;

        assert!(result.get("resultType").is_none(), "response was {body}");
        assert!(result.get("ttlMs").is_none(), "response was {body}");
        assert!(result.get("cacheScope").is_none(), "response was {body}");

        server.abort();

        Ok(())
    }

    #[tokio::test]
    async fn modern_read_resource_is_scoped_private() -> anyhow::Result<()> {
        let backend = Router::new().route(
            "/api/v2/acta/workspaces/{workspace}/documents/{slug}",
            get(mock_document),
        );
        let (backend_url, backend_server) = spawn_router(backend).await?;
        let router = build_http_router(backend_url, "127.0.0.1".to_string(), TEST_PORT)?;
        let (base_url, mcp_server) = spawn_router(router).await?;
        let client = reqwest::Client::new();

        let response = post_modern(
            &client,
            &format!("{base_url}/mcp"),
            "atlas_test",
            1,
            "resources/read",
            serde_json::json!({ "uri": "atlas:///atlas/notes" }),
        )
        .await?;

        assert_eq!(response.status(), http::StatusCode::OK);
        let body: serde_json::Value = response.json().await?;
        let result = body
            .get("result")
            .ok_or_else(|| anyhow!("resources/read returned no result: {body}"))?;

        assert_eq!(
            result.get("resultType").and_then(serde_json::Value::as_str),
            Some("complete"),
            "response was {body}"
        );
        assert_eq!(
            result.get("cacheScope").and_then(serde_json::Value::as_str),
            Some("private"),
            "a document body is scoped to the calling principal: {body}"
        );
        assert_eq!(
            result.get("ttlMs").and_then(serde_json::Value::as_u64),
            Some(0),
            "document bodies are never served stale: {body}"
        );

        mcp_server.abort();
        backend_server.abort();

        Ok(())
    }

    // -----------------------------------------------------------------------
    // 2026-07-28 per-request protocol contract
    // -----------------------------------------------------------------------

    async fn missing_document() -> (http::StatusCode, Json<serde_json::Value>) {
        (
            http::StatusCode::NOT_FOUND,
            Json(serde_json::json!({
                "type": "urn:atlas:error:not_found",
                "title": "Not Found",
                "status": 404,
                "detail": "document not found"
            })),
        )
    }

    /// Reads the JSON-RPC error code out of a response body.
    fn error_code(body: &serde_json::Value) -> Option<i64> {
        body.pointer("/error/code")
            .and_then(serde_json::Value::as_i64)
    }

    #[tokio::test]
    async fn modern_request_without_client_capabilities_is_invalid_params() -> anyhow::Result<()> {
        let router = build_http_router(
            "http://127.0.0.1:1".to_string(),
            "127.0.0.1".to_string(),
            TEST_PORT,
        )?;
        let (base_url, server) = spawn_router(router).await?;
        let client = reqwest::Client::new();

        let response = client
            .post(format!("{base_url}/mcp"))
            .header(http::header::AUTHORIZATION, "Bearer atlas_test")
            .header(http::header::CONTENT_TYPE, "application/json")
            .header(http::header::ACCEPT, "application/json, text/event-stream")
            .header("MCP-Protocol-Version", MODERN_VERSION)
            .header("Mcp-Method", "tools/list")
            .body(format!(
                r#"{{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{{"_meta":{{"io.modelcontextprotocol/protocolVersion":"{MODERN_VERSION}"}}}}}}"#
            ))
            .send()
            .await?;

        assert_eq!(response.status(), http::StatusCode::BAD_REQUEST);
        let body: serde_json::Value = response.json().await?;
        assert_eq!(error_code(&body), Some(-32602), "response was {body}");
        assert!(
            body.pointer("/error/message")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|message| message.contains("clientCapabilities")),
            "the error must name the missing key: {body}"
        );

        server.abort();

        Ok(())
    }

    #[tokio::test]
    async fn modern_meta_without_the_protocol_version_header_is_a_header_mismatch()
    -> anyhow::Result<()> {
        let router = build_http_router(
            "http://127.0.0.1:1".to_string(),
            "127.0.0.1".to_string(),
            TEST_PORT,
        )?;
        let (base_url, server) = spawn_router(router).await?;
        let client = reqwest::Client::new();

        let response = post_mcp(
            &client,
            &format!("{base_url}/mcp"),
            "atlas_test",
            modern_request(1, "tools/list", serde_json::json!({})),
        )
        .await?;

        assert_eq!(response.status(), http::StatusCode::BAD_REQUEST);
        let body: serde_json::Value = response.json().await?;
        assert_eq!(error_code(&body), Some(-32020), "response was {body}");

        server.abort();

        Ok(())
    }

    #[tokio::test]
    async fn modern_mcp_method_header_must_match_the_body() -> anyhow::Result<()> {
        let router = build_http_router(
            "http://127.0.0.1:1".to_string(),
            "127.0.0.1".to_string(),
            TEST_PORT,
        )?;
        let (base_url, server) = spawn_router(router).await?;
        let client = reqwest::Client::new();

        let response = client
            .post(format!("{base_url}/mcp"))
            .header(http::header::AUTHORIZATION, "Bearer atlas_test")
            .header(http::header::CONTENT_TYPE, "application/json")
            .header(http::header::ACCEPT, "application/json, text/event-stream")
            .header("MCP-Protocol-Version", MODERN_VERSION)
            .header("Mcp-Method", "resources/templates/list")
            .body(modern_request(1, "tools/list", serde_json::json!({})))
            .send()
            .await?;

        assert_eq!(response.status(), http::StatusCode::BAD_REQUEST);
        let body: serde_json::Value = response.json().await?;
        assert_eq!(error_code(&body), Some(-32020), "response was {body}");

        server.abort();

        Ok(())
    }

    #[tokio::test]
    async fn modern_resources_read_requires_the_mcp_name_header() -> anyhow::Result<()> {
        let router = build_http_router(
            "http://127.0.0.1:1".to_string(),
            "127.0.0.1".to_string(),
            TEST_PORT,
        )?;
        let (base_url, server) = spawn_router(router).await?;
        let client = reqwest::Client::new();

        let response = client
            .post(format!("{base_url}/mcp"))
            .header(http::header::AUTHORIZATION, "Bearer atlas_test")
            .header(http::header::CONTENT_TYPE, "application/json")
            .header(http::header::ACCEPT, "application/json, text/event-stream")
            .header("MCP-Protocol-Version", MODERN_VERSION)
            .header("Mcp-Method", "resources/read")
            .body(modern_request(
                1,
                "resources/read",
                serde_json::json!({ "uri": "atlas:///atlas/notes" }),
            ))
            .send()
            .await?;

        assert_eq!(response.status(), http::StatusCode::BAD_REQUEST);
        let body: serde_json::Value = response.json().await?;
        assert_eq!(error_code(&body), Some(-32020), "response was {body}");

        server.abort();

        Ok(())
    }

    #[tokio::test]
    async fn modern_mcp_name_header_accepts_the_base64_sentinel() -> anyhow::Result<()> {
        use base64::Engine;

        let backend = Router::new().route(
            "/api/v2/acta/workspaces/{workspace}/documents/{slug}",
            get(mock_document),
        );
        let (backend_url, backend_server) = spawn_router(backend).await?;
        let router = build_http_router(backend_url, "127.0.0.1".to_string(), TEST_PORT)?;
        let (base_url, mcp_server) = spawn_router(router).await?;
        let client = reqwest::Client::new();

        let uri = "atlas:///atlas/café";
        let sentinel = format!(
            "=?base64?{}?=",
            base64::prelude::BASE64_STANDARD.encode(uri)
        );

        let response = client
            .post(format!("{base_url}/mcp"))
            .header(http::header::AUTHORIZATION, "Bearer atlas_test")
            .header(http::header::CONTENT_TYPE, "application/json")
            .header(http::header::ACCEPT, "application/json, text/event-stream")
            .header("MCP-Protocol-Version", MODERN_VERSION)
            .header("Mcp-Method", "resources/read")
            .header("Mcp-Name", sentinel)
            .body(modern_request(
                1,
                "resources/read",
                serde_json::json!({ "uri": uri }),
            ))
            .send()
            .await?;

        assert_eq!(response.status(), http::StatusCode::OK);
        let body: serde_json::Value = response.json().await?;
        assert!(
            body.get("result").is_some(),
            "the sentinel must decode to the body uri: {body}"
        );

        mcp_server.abort();
        backend_server.abort();

        Ok(())
    }

    #[tokio::test]
    async fn a_protocol_version_atlas_does_not_implement_is_rejected() -> anyhow::Result<()> {
        let router = build_http_router(
            "http://127.0.0.1:1".to_string(),
            "127.0.0.1".to_string(),
            TEST_PORT,
        )?;
        let (base_url, server) = spawn_router(router).await?;
        let client = reqwest::Client::new();

        let response = client
            .post(format!("{base_url}/mcp"))
            .header(http::header::AUTHORIZATION, "Bearer atlas_test")
            .header(http::header::CONTENT_TYPE, "application/json")
            .header(http::header::ACCEPT, "application/json, text/event-stream")
            .header("MCP-Protocol-Version", "2099-01-01")
            .header("Mcp-Method", "tools/list")
            .body(
                r#"{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{"_meta":{"io.modelcontextprotocol/protocolVersion":"2099-01-01","io.modelcontextprotocol/clientCapabilities":{}}}}"#,
            )
            .send()
            .await?;

        assert_eq!(response.status(), http::StatusCode::BAD_REQUEST);
        let body: serde_json::Value = response.json().await?;
        assert_eq!(error_code(&body), Some(-32022), "response was {body}");

        server.abort();

        Ok(())
    }

    #[tokio::test]
    async fn an_unknown_method_is_not_found() -> anyhow::Result<()> {
        let router = build_http_router(
            "http://127.0.0.1:1".to_string(),
            "127.0.0.1".to_string(),
            TEST_PORT,
        )?;
        let (base_url, server) = spawn_router(router).await?;
        let client = reqwest::Client::new();

        let response = post_modern(
            &client,
            &format!("{base_url}/mcp"),
            "atlas_test",
            1,
            "atlas/does-not-exist",
            serde_json::json!({}),
        )
        .await?;

        assert_eq!(response.status(), http::StatusCode::NOT_FOUND);
        let body: serde_json::Value = response.json().await?;
        assert_eq!(error_code(&body), Some(-32601), "response was {body}");

        server.abort();

        Ok(())
    }

    #[tokio::test]
    async fn modern_read_of_a_missing_document_is_invalid_params() -> anyhow::Result<()> {
        let backend = Router::new().route(
            "/api/v2/acta/workspaces/{workspace}/documents/{slug}",
            get(missing_document),
        );
        let (backend_url, backend_server) = spawn_router(backend).await?;
        let router = build_http_router(backend_url, "127.0.0.1".to_string(), TEST_PORT)?;
        let (base_url, mcp_server) = spawn_router(router).await?;
        let client = reqwest::Client::new();

        let response = post_modern(
            &client,
            &format!("{base_url}/mcp"),
            "atlas_test",
            1,
            "resources/read",
            serde_json::json!({ "uri": "atlas:///atlas/ghost" }),
        )
        .await?;

        assert_eq!(response.status(), http::StatusCode::BAD_REQUEST);
        let body: serde_json::Value = response.json().await?;
        assert_eq!(error_code(&body), Some(-32602), "response was {body}");

        mcp_server.abort();
        backend_server.abort();

        Ok(())
    }

    #[tokio::test]
    async fn legacy_read_of_a_missing_document_keeps_resource_not_found() -> anyhow::Result<()> {
        let backend = Router::new().route(
            "/api/v2/acta/workspaces/{workspace}/documents/{slug}",
            get(missing_document),
        );
        let (backend_url, backend_server) = spawn_router(backend).await?;
        let router = build_http_router(backend_url, "127.0.0.1".to_string(), TEST_PORT)?;
        let (base_url, mcp_server) = spawn_router(router).await?;
        let client = reqwest::Client::new();

        let response = post_mcp(
            &client,
            &format!("{base_url}/mcp"),
            "atlas_test",
            r#"{"jsonrpc":"2.0","id":1,"method":"resources/read","params":{"uri":"atlas:///atlas/ghost"}}"#,
        )
        .await?;

        assert_eq!(response.status(), http::StatusCode::OK);
        let body: serde_json::Value = response.json().await?;
        assert_eq!(error_code(&body), Some(-32002), "response was {body}");

        mcp_server.abort();
        backend_server.abort();

        Ok(())
    }

    // -----------------------------------------------------------------------
    // Multi round-trip confirmation of destructive deletes (SEP-2322)
    // -----------------------------------------------------------------------

    const DELETE_TASK_ARGUMENTS: &str = r#"{"resource":"task","params":{"workspace":"atlas","readable_id":"ATL-42","confirm":false}}"#;

    /// Backend that records every request it receives, so a test can assert that
    /// an unconfirmed delete never reached Atlas.
    fn spawn_counting_backend() -> (Router, std::sync::Arc<std::sync::atomic::AtomicUsize>) {
        use std::sync::{Arc, atomic::AtomicUsize};

        let deletes = Arc::new(AtomicUsize::new(0));
        let counter = deletes.clone();
        let router = Router::new().route(
            "/api/v2/acta/workspaces/{workspace}/tasks/{readable_id}",
            axum::routing::delete(move || {
                let counter = counter.clone();
                async move {
                    counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    http::StatusCode::NO_CONTENT
                }
            }),
        );

        (router, deletes)
    }

    fn delete_call_params(input_responses: Option<serde_json::Value>) -> serde_json::Value {
        let arguments: serde_json::Value =
            serde_json::from_str(DELETE_TASK_ARGUMENTS).expect("delete arguments are valid JSON");
        let mut params = serde_json::json!({ "name": "delete", "arguments": arguments });

        if let (Some(responses), Some(object)) = (input_responses, params.as_object_mut()) {
            object.insert("inputResponses".to_string(), responses);
        }

        params
    }

    /// `_meta` for a modern peer that can answer elicitation requests.
    fn elicitation_capable_meta() -> serde_json::Value {
        serde_json::json!({
            "io.modelcontextprotocol/protocolVersion": MODERN_VERSION,
            "io.modelcontextprotocol/clientCapabilities": { "elicitation": {} }
        })
    }

    async fn post_delete(
        client: &reqwest::Client,
        url: &str,
        meta: serde_json::Value,
        input_responses: Option<serde_json::Value>,
    ) -> anyhow::Result<serde_json::Value> {
        let mut params = delete_call_params(input_responses);
        match params.as_object_mut() {
            Some(object) => object.insert("_meta".to_string(), meta),
            None => panic!("call params are an object"),
        };

        let body = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": params
        })
        .to_string();

        let response = client
            .post(url)
            .header(http::header::AUTHORIZATION, "Bearer atlas_test")
            .header(http::header::CONTENT_TYPE, "application/json")
            .header(http::header::ACCEPT, "application/json, text/event-stream")
            .header("MCP-Protocol-Version", MODERN_VERSION)
            .header("Mcp-Method", "tools/call")
            .header("Mcp-Name", "delete")
            .body(body)
            .send()
            .await?;

        Ok(response.json().await?)
    }

    #[tokio::test]
    async fn modern_delete_without_confirmation_asks_the_client_for_input() -> anyhow::Result<()> {
        let (backend, deletes) = spawn_counting_backend();
        let (backend_url, backend_server) = spawn_router(backend).await?;
        let router = build_http_router(backend_url, "127.0.0.1".to_string(), TEST_PORT)?;
        let (base_url, mcp_server) = spawn_router(router).await?;
        let client = reqwest::Client::new();

        let body = post_delete(
            &client,
            &format!("{base_url}/mcp"),
            elicitation_capable_meta(),
            None,
        )
        .await?;

        assert_eq!(
            body.pointer("/result/resultType")
                .and_then(serde_json::Value::as_str),
            Some("input_required"),
            "response was {body}"
        );

        let requests = body
            .pointer("/result/inputRequests")
            .and_then(serde_json::Value::as_object)
            .ok_or_else(|| anyhow!("no inputRequests in {body}"))?;
        assert_eq!(requests.len(), 1, "response was {body}");
        let request = requests
            .values()
            .next()
            .ok_or_else(|| anyhow!("no inputRequest in {body}"))?;
        assert_eq!(
            request.get("method").and_then(serde_json::Value::as_str),
            Some("elicitation/create"),
            "response was {body}"
        );
        assert!(
            request
                .pointer("/params/message")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|message| message.contains("ATL-42")),
            "the prompt must name what is about to be deleted: {body}"
        );
        assert!(
            body.pointer("/result/requestState").is_none(),
            "Atlas keeps no authorization state in requestState: {body}"
        );
        assert_eq!(
            deletes.load(std::sync::atomic::Ordering::SeqCst),
            0,
            "nothing may be deleted before the client confirms"
        );

        mcp_server.abort();
        backend_server.abort();

        Ok(())
    }

    #[tokio::test]
    async fn modern_delete_runs_once_the_client_accepts() -> anyhow::Result<()> {
        let (backend, deletes) = spawn_counting_backend();
        let (backend_url, backend_server) = spawn_router(backend).await?;
        let router = build_http_router(backend_url, "127.0.0.1".to_string(), TEST_PORT)?;
        let (base_url, mcp_server) = spawn_router(router).await?;
        let client = reqwest::Client::new();
        let mcp_url = format!("{base_url}/mcp");

        let first = post_delete(&client, &mcp_url, elicitation_capable_meta(), None).await?;
        let key = first
            .pointer("/result/inputRequests")
            .and_then(serde_json::Value::as_object)
            .and_then(|requests| requests.keys().next().cloned())
            .ok_or_else(|| anyhow!("no inputRequests in {first}"))?;

        let body = post_delete(
            &client,
            &mcp_url,
            elicitation_capable_meta(),
            Some(serde_json::json!({
                key: { "action": "accept", "content": { "confirm": true } }
            })),
        )
        .await?;

        assert_eq!(
            body.pointer("/result/resultType")
                .and_then(serde_json::Value::as_str),
            Some("complete"),
            "response was {body}"
        );
        assert_ne!(
            body.pointer("/result/isError")
                .and_then(serde_json::Value::as_bool),
            Some(true),
            "response was {body}"
        );
        assert_eq!(
            deletes.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "the accepted confirmation must run exactly one delete"
        );

        mcp_server.abort();
        backend_server.abort();

        Ok(())
    }

    #[tokio::test]
    async fn modern_delete_declined_by_the_client_deletes_nothing() -> anyhow::Result<()> {
        let (backend, deletes) = spawn_counting_backend();
        let (backend_url, backend_server) = spawn_router(backend).await?;
        let router = build_http_router(backend_url, "127.0.0.1".to_string(), TEST_PORT)?;
        let (base_url, mcp_server) = spawn_router(router).await?;
        let client = reqwest::Client::new();

        let body = post_delete(
            &client,
            &format!("{base_url}/mcp"),
            elicitation_capable_meta(),
            Some(serde_json::json!({
                "confirm_deletion": { "action": "decline" }
            })),
        )
        .await?;

        assert_eq!(
            body.pointer("/result/isError")
                .and_then(serde_json::Value::as_bool),
            Some(true),
            "response was {body}"
        );
        assert!(
            body.pointer("/result/content/0/text")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|text| text.contains("declined")),
            "the answer, not the missing flag, is what stopped it: {body}"
        );
        assert_eq!(
            deletes.load(std::sync::atomic::Ordering::SeqCst),
            0,
            "a declined confirmation must delete nothing"
        );

        mcp_server.abort();
        backend_server.abort();

        Ok(())
    }

    #[tokio::test]
    async fn modern_delete_without_elicitation_keeps_the_error_string() -> anyhow::Result<()> {
        let (backend, deletes) = spawn_counting_backend();
        let (backend_url, backend_server) = spawn_router(backend).await?;
        let router = build_http_router(backend_url, "127.0.0.1".to_string(), TEST_PORT)?;
        let (base_url, mcp_server) = spawn_router(router).await?;
        let client = reqwest::Client::new();

        let body = post_delete(
            &client,
            &format!("{base_url}/mcp"),
            serde_json::json!({
                "io.modelcontextprotocol/protocolVersion": MODERN_VERSION,
                "io.modelcontextprotocol/clientCapabilities": {}
            }),
            None,
        )
        .await?;

        assert_eq!(
            body.pointer("/result/isError")
                .and_then(serde_json::Value::as_bool),
            Some(true),
            "response was {body}"
        );
        assert!(
            body.pointer("/result/content/0/text")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|text| text.contains("confirm: true")),
            "a client that cannot be asked keeps the actionable error: {body}"
        );
        assert_eq!(deletes.load(std::sync::atomic::Ordering::SeqCst), 0);

        mcp_server.abort();
        backend_server.abort();

        Ok(())
    }

    #[tokio::test]
    async fn legacy_delete_without_confirmation_keeps_the_error_string() -> anyhow::Result<()> {
        let (backend, deletes) = spawn_counting_backend();
        let (backend_url, backend_server) = spawn_router(backend).await?;
        let router = build_http_router(backend_url, "127.0.0.1".to_string(), TEST_PORT)?;
        let (base_url, mcp_server) = spawn_router(router).await?;
        let client = reqwest::Client::new();

        let response = post_mcp(
            &client,
            &format!("{base_url}/mcp"),
            "atlas_test",
            format!(
                r#"{{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{{"name":"delete","arguments":{DELETE_TASK_ARGUMENTS}}}}}"#
            ),
        )
        .await?;

        assert_eq!(response.status(), http::StatusCode::OK);
        let body: serde_json::Value = response.json().await?;
        assert_eq!(
            body.pointer("/result/isError")
                .and_then(serde_json::Value::as_bool),
            Some(true),
            "response was {body}"
        );
        assert!(
            body.pointer("/result/content/0/text")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|text| text.contains("confirm: true")),
            "response was {body}"
        );
        assert!(
            body.pointer("/result/resultType").is_none(),
            "a legacy peer gets no resultType discriminator: {body}"
        );
        assert_eq!(deletes.load(std::sync::atomic::Ordering::SeqCst), 0);

        mcp_server.abort();
        backend_server.abort();

        Ok(())
    }

    #[tokio::test]
    async fn a_browser_origin_is_rejected() -> anyhow::Result<()> {
        let router = build_http_router(
            "http://127.0.0.1:1".to_string(),
            "127.0.0.1".to_string(),
            TEST_PORT,
        )?;
        let (base_url, server) = spawn_router(router).await?;
        let client = reqwest::Client::new();

        let response = client
            .post(format!("{base_url}/mcp"))
            .header(http::header::AUTHORIZATION, "Bearer atlas_test")
            .header(http::header::CONTENT_TYPE, "application/json")
            .header(http::header::ACCEPT, "application/json, text/event-stream")
            .header(http::header::ORIGIN, "https://evil.example")
            .body(INITIALIZE_REQUEST)
            .send()
            .await?;

        assert_eq!(response.status(), http::StatusCode::FORBIDDEN);

        server.abort();

        Ok(())
    }

    #[tokio::test]
    async fn the_servers_own_origin_is_accepted() -> anyhow::Result<()> {
        let router = build_http_router(
            "http://127.0.0.1:1".to_string(),
            "127.0.0.1".to_string(),
            TEST_PORT,
        )?;
        let (base_url, server) = spawn_router(router).await?;
        let client = reqwest::Client::new();

        let response = client
            .post(format!("{base_url}/mcp"))
            .header(http::header::AUTHORIZATION, "Bearer atlas_test")
            .header(http::header::CONTENT_TYPE, "application/json")
            .header(http::header::ACCEPT, "application/json, text/event-stream")
            .header(
                http::header::ORIGIN,
                format!("http://localhost:{TEST_PORT}"),
            )
            .body(INITIALIZE_REQUEST)
            .send()
            .await?;

        assert_eq!(response.status(), http::StatusCode::OK);

        server.abort();

        Ok(())
    }

    #[tokio::test]
    async fn only_post_is_served() -> anyhow::Result<()> {
        let router = build_http_router(
            "http://127.0.0.1:1".to_string(),
            "127.0.0.1".to_string(),
            TEST_PORT,
        )?;
        let (base_url, server) = spawn_router(router).await?;
        let client = reqwest::Client::new();
        let mcp_url = format!("{base_url}/mcp");

        for method in [http::Method::GET, http::Method::DELETE] {
            let response = client
                .request(method.clone(), &mcp_url)
                .header(http::header::AUTHORIZATION, "Bearer atlas_test")
                .header(http::header::ACCEPT, "application/json, text/event-stream")
                .send()
                .await?;

            assert_eq!(
                response.status(),
                http::StatusCode::METHOD_NOT_ALLOWED,
                "{method} must not open a stream on a stateless server"
            );
        }

        server.abort();

        Ok(())
    }
}
