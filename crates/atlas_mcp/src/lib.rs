#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

use rmcp::{tool, tool_router};

#[derive(Clone)]
pub struct AtlasMcp;

#[tool_router(server_handler)]
impl AtlasMcp {
    #[tool(description = "Ping the Atlas MCP server")]
    fn ping(&self) -> String {
        "pong".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atlas_mcp_constructs() {
        let _server = AtlasMcp;
    }
}
