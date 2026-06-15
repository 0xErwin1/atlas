#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

use atlas_client::AtlasClient;
use rmcp::{handler::server::wrapper::Parameters, tool, tool_router};
use schemars::JsonSchema;
use serde::Deserialize;

/// MCP server backed by an Atlas HTTP API endpoint.
#[derive(Clone)]
pub struct AtlasMcp {
    base_url: String,
    token: Option<String>,
}

impl AtlasMcp {
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
            token: None,
        }
    }

    pub fn with_token(mut self, token: impl Into<String>) -> Self {
        self.token = Some(token.into());
        self
    }

    fn client(&self) -> AtlasClient {
        let mut c = AtlasClient::new(&self.base_url);
        if let Some(t) = &self.token {
            c.set_token(t.clone());
        }
        c
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

#[tool_router(server_handler)]
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
            .client()
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atlas_mcp_constructs_with_base_url() {
        let _server = AtlasMcp::new("http://localhost:8080");
    }

    #[test]
    fn atlas_mcp_constructs_with_token() {
        let _server = AtlasMcp::new("http://localhost:8080").with_token("test-token");
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
        let json = r#"{"workspace":"ws","query":"q","type_filter":"task","sort":"updated","limit":25}"#;
        let params: SearchResourcesParams = serde_json::from_str(json).unwrap();
        assert_eq!(params.type_filter.as_deref(), Some("task"));
        assert_eq!(params.sort.as_deref(), Some("updated"));
        assert_eq!(params.limit, Some(25));
    }
}
