use rust_mcp_schema::Tool;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct McpServersConfig {
    pub mcp_servers: Vec<McpServerConfig>,
    /// the auth config per mcp server id, absent in configs of mcp servers without auth
    #[serde(default)]
    pub mcps: HashMap<String, McpsEntry>,
}

/// An entry of `mcps`: the compiler leaves out `kind` for public mcp servers.
#[derive(Deserialize)]
#[serde(untagged)]
pub enum McpsEntry {
    Auth(McpAuthConfig),
    Public(PublicMcp),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct PublicMcp {
    #[allow(dead_code)]
    authentication_profile_id: Option<String>,
}

/// How the token of an mcp server is validated.
#[derive(Debug, Deserialize, Clone, PartialEq)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum McpAuthConfig {
    ApiKey,
    DataApi {
        /// where the client finds the authorization server (RFC 9728)
        #[serde(rename = "resource-metadata-url")]
        resource_metadata_url: String,
    },
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
    pub auth: Option<McpAuthConfig>,
}
