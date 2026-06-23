use rust_mcp_schema::Tool;
use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct McpServersConfig {
    pub mcp_servers: Vec<McpServerConfig>,
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
}
