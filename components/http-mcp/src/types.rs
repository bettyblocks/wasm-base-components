use rust_mcp_schema::Tool;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct McpServersConfig {
    pub mcp_servers: Vec<McpServerConfig>,
    /// the auth config per mcp server id, absent in configs of mcp servers without auth
    #[serde(default)]
    pub mcps: HashMap<String, McpAuthConfig>,
}

#[derive(Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct McpAuthConfig {
    /// where the client finds the authorization server, only set for data-api auth profiles
    pub resource_metadata_url: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "kebab-case")]
pub struct ToolWithAction {
    #[serde(flatten)]
    pub tool: Tool,
    pub action_id: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct McpServerConfig {
    pub id: String,
    pub tools: Vec<ToolWithAction>,
    /// filled from `mcps` when the config is loaded, not part of the server entry itself
    #[serde(skip)]
    pub resource_metadata_url: Option<String>,
}
