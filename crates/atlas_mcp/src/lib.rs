#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

use std::sync::Arc;

use atlas_client::AtlasClient;
use rmcp::{
    ServerHandler,
    handler::server::wrapper::Parameters,
    model::{Implementation, ServerCapabilities, ServerInfo},
    tool, tool_handler, tool_router,
};
use schemars::JsonSchema;
use serde::Deserialize;

const ATLAS_INSTRUCTIONS: &str = "\
Atlas is a personal knowledge base for notes and tasks. \
Use `search_resources` to retrieve content by keyword or structured filters \
(status:open, tag:rust, etc.) before acting on it. \
Prefer narrow queries over broad ones; follow up with targeted reads rather than \
enumerating all results.";

/// MCP server backed by an Atlas HTTP API endpoint.
///
/// Holds a single shared client, built once on construction and reused across
/// all tool calls. Cloning the handler shares the same underlying client.
#[derive(Clone)]
pub struct AtlasMcp {
    client: Arc<AtlasClient>,
}

impl AtlasMcp {
    /// Returns a reference to the underlying HTTP client, for pre-serve diagnostics.
    pub fn client(&self) -> &AtlasClient {
        &self.client
    }

    /// Constructs an `AtlasMcp` with the given base URL and required API token.
    ///
    /// Returns an error if either argument is empty.
    pub fn new(base_url: impl Into<String>, token: impl Into<String>) -> anyhow::Result<Self> {
        let base_url = base_url.into();
        let token = token.into();

        if base_url.is_empty() {
            anyhow::bail!("base_url must not be empty");
        }
        if token.is_empty() {
            anyhow::bail!("ATLAS_TOKEN must not be empty");
        }

        let mut client = AtlasClient::new(base_url);
        client.set_token(token);

        Ok(Self {
            client: Arc::new(client),
        })
    }
}

/// Parameters accepted by the `search_resources` tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct SearchResourcesParams {
    /// Workspace slug to search in.
    pub workspace: String,
    /// Query string. Supports token filters like `status:open`, `tag:rust`.
    pub query: String,
    /// Kind filter: `all` (default), `note`, or `task`.
    #[serde(default)]
    pub type_filter: Option<String>,
    /// Sort order: `relevance` (default) or `updated`.
    #[serde(default)]
    pub sort: Option<String>,
    /// Maximum number of results (default 50, clamped to [1, 200]).
    #[serde(default)]
    pub limit: Option<u32>,
}

#[tool_router]
impl AtlasMcp {
    #[tool(description = "Ping the Atlas MCP server")]
    fn ping(&self) -> String {
        "pong".to_string()
    }

    #[tool(description = "Search documents and tasks in an Atlas workspace")]
    async fn search_resources(
        &self,
        Parameters(params): Parameters<SearchResourcesParams>,
    ) -> Result<String, String> {
        let page = self
            .client
            .search(
                &params.workspace,
                &params.query,
                params.type_filter.as_deref(),
                params.sort.as_deref(),
                None,
                params.limit,
            )
            .await
            .map_err(|e| e.to_string())?;

        let hits: Vec<serde_json::Value> = page
            .items
            .into_iter()
            .map(|hit| {
                serde_json::json!({
                    "id": hit.id,
                    "kind": format!("{:?}", hit.kind).to_lowercase(),
                    "readable_id": hit.readable_id,
                    "title": hit.title,
                    "snippet": hit.snippet,
                    "score": hit.score,
                    "updated_at": hit.updated_at,
                    "project_slug": hit.project_slug,
                })
            })
            .collect();

        serde_json::to_string(&hits).map_err(|e| e.to_string())
    }
}

#[tool_handler]
impl ServerHandler for AtlasMcp {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("atlas-mcp", env!("CARGO_PKG_VERSION")))
            .with_instructions(ATLAS_INSTRUCTIONS)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_missing_token() {
        let result = AtlasMcp::new("http://localhost:8080", "");
        assert!(result.is_err(), "empty token must be rejected");
    }

    #[test]
    fn rejects_missing_base_url() {
        let result = AtlasMcp::new("", "some-token");
        assert!(result.is_err(), "empty base_url must be rejected");
    }

    #[test]
    fn constructs_with_valid_args() {
        let server = AtlasMcp::new("http://localhost:8080", "test-token");
        assert!(server.is_ok());
    }

    #[test]
    fn clone_shares_client_arc() {
        let server = AtlasMcp::new("http://localhost:8080", "test-token").unwrap();
        let cloned = server.clone();
        assert!(std::ptr::eq(
            server.client() as *const AtlasClient,
            cloned.client() as *const AtlasClient
        ));
    }

    #[test]
    fn get_info_returns_correct_name_and_version() {
        let server = AtlasMcp::new("http://localhost:8080", "test-token").unwrap();
        let info = server.get_info();
        assert_eq!(info.server_info.name, "atlas-mcp");
        assert_eq!(info.server_info.version, env!("CARGO_PKG_VERSION"));
        assert!(
            info.instructions.as_deref().is_some_and(|s| !s.is_empty()),
            "instructions must be Some and non-empty"
        );
    }

    #[test]
    fn ping_returns_pong() {
        let server = AtlasMcp::new("http://localhost:8080", "test-token").unwrap();
        assert_eq!(server.ping(), "pong");
    }

    #[test]
    fn search_resources_params_deserializes() {
        let json = r#"{"workspace":"my-ws","query":"hello"}"#;
        let params: SearchResourcesParams = serde_json::from_str(json).unwrap();
        assert_eq!(params.workspace, "my-ws");
        assert_eq!(params.query, "hello");
        assert!(params.type_filter.is_none());
        assert!(params.sort.is_none());
        assert!(params.limit.is_none());
    }

    #[test]
    fn search_resources_params_deserializes_with_optionals() {
        let json =
            r#"{"workspace":"ws","query":"q","type_filter":"task","sort":"updated","limit":25}"#;
        let params: SearchResourcesParams = serde_json::from_str(json).unwrap();
        assert_eq!(params.type_filter.as_deref(), Some("task"));
        assert_eq!(params.sort.as_deref(), Some("updated"));
        assert_eq!(params.limit, Some(25));
    }
}
