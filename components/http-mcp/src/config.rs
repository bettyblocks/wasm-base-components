use std::collections::HashMap;

use crate::types::{McpServerConfig, McpServersConfig};
use crate::wasi::cli::environment::get_environment;
use crate::wasi::config::store::get;
use rust_mcp_schema::{
    Implementation, InitializeResult, ServerCapabilities, ServerCapabilitiesTools,
    LATEST_PROTOCOL_VERSION,
};

const WASI_CONFIG_KEY: &str = "mcp_servers";
const BETTY_CONFIG_KEY: &str = "betty_config";
const WASMCLOUD_HOST_ENV: &str = "WASMCLOUD_HOST";
const APPLICATION_ID_ENV: &str = "APPLICATION_ID";

pub struct EnvConfig {
    pub wasmcloud_host: String,
    pub application_id: String,
}

impl EnvConfig {
    pub fn from_env() -> Result<Self, String> {
        let env: HashMap<String, String> = get_environment().into_iter().collect();

        let get = |name: &str| -> Result<String, String> {
            env.get(name)
                .cloned()
                .ok_or_else(|| format!("Environment variable '{name}' not set"))
        };

        Ok(Self {
            wasmcloud_host: get(WASMCLOUD_HOST_ENV)?,
            application_id: get(APPLICATION_ID_ENV)?,
        })
    }
}

pub fn load_server_config(server_id: &str) -> Result<McpServerConfig, String> {
    let raw = get(WASI_CONFIG_KEY)
        .map_err(|e| format!("Failed to get wasi config: {e:?}"))?
        .ok_or_else(|| "mcp_servers key not found in runtime configuration".to_string())?;

    parse_server_config(&raw, server_id)
}

pub fn load_betty_config() -> Result<serde_json::Value, String> {
    let raw = get(BETTY_CONFIG_KEY)
        .map_err(|e| format!("Failed to get betty config: {e:?}"))?
        .ok_or_else(|| "mcp_servers key not found in runtime configuration".to_string())?;

    Ok(serde_json::from_str(&raw).map_err(|e| format!("unable to decode betty config {e}"))?)
}

pub fn build_initialize_result(server_config: &McpServerConfig) -> InitializeResult {
    InitializeResult {
        protocol_version: LATEST_PROTOCOL_VERSION.to_string(),
        capabilities: ServerCapabilities {
            tools: (!server_config.tools.is_empty()).then_some(ServerCapabilitiesTools::default()),
            ..Default::default()
        },
        server_info: Implementation {
            name: server_config.id.clone(),
            version: "0.1.0".to_string(),
            description: None,
            icons: vec![],
            title: None,
            website_url: None,
        },
        instructions: None,
        meta: None,
    }
}

fn parse_server_config(raw: &str, server_id: &str) -> Result<McpServerConfig, String> {
    let config: McpServersConfig = serde_json::from_str(raw)
        .map_err(|e| format!("Failed to parse mcp_servers config: {e}"))?;

    config
        .mcp_servers
        .into_iter()
        .find(|s| s.id == server_id)
        .ok_or_else(|| format!("MCP server '{server_id}' not found in configuration"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_MCP_CONFIG: &str = r#"{
        "mcp-servers": [
            {
                "id": "weather-server-001",
                "tools": [
                    {
                        "name": "get_weather",
                        "description": "Get weather for a location",
                        "inputSchema": {
                            "type": "object",
                            "properties": {
                                "location": { "type": "string" }
                            },
                            "required": ["location"]
                        },
                        "action-id": "action-weather-001"
                    }
                ]
            },
            {
                "id": "calculator-server-001",
                "tools": [
                    {
                        "name": "add",
                        "description": "Add two numbers",
                        "inputSchema": {
                            "type": "object",
                            "properties": {
                                "a": { "type": "number" },
                                "b": { "type": "number" }
                            },
                            "required": ["a", "b"]
                        },
                        "action-id": "action-calc-001"
                    }
                ]
            }
        ]
    }"#;

    #[test]
    fn test_parse_server_config_finds_existing_server() {
        let config = parse_server_config(TEST_MCP_CONFIG, "weather-server-001").unwrap();
        assert_eq!(config.id, "weather-server-001");
        assert_eq!(config.tools.len(), 1);
        assert_eq!(config.tools[0].tool.name, "get_weather");
        assert_eq!(config.tools[0].action_id, "action-weather-001");
    }

    #[test]
    fn test_parse_server_config_finds_second_server() {
        let config = parse_server_config(TEST_MCP_CONFIG, "calculator-server-001").unwrap();
        assert_eq!(config.id, "calculator-server-001");
        assert_eq!(config.tools[0].tool.name, "add");
        assert_eq!(config.tools[0].action_id, "action-calc-001");
    }

    #[test]
    fn test_parse_server_config_not_found() {
        let result = parse_server_config(TEST_MCP_CONFIG, "nonexistent-server");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not found in configuration"));
    }

    #[test]
    fn test_parse_server_config_invalid_json() {
        let result = parse_server_config("not json", "any-id");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Failed to parse"));
    }

    #[test]
    fn test_parse_server_config_missing_mcp_servers_key() {
        let result = parse_server_config(r#"{"other": "data"}"#, "any-id");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Failed to parse"));
    }

    #[test]
    fn test_parse_server_config_empty_servers_array() {
        let result = parse_server_config(r#"{"mcp-servers": []}"#, "any-id");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not found in configuration"));
    }

    #[test]
    fn test_build_initialize_result_with_tools() {
        let config = parse_server_config(TEST_MCP_CONFIG, "weather-server-001").unwrap();
        let result = build_initialize_result(&config);

        assert_eq!(result.protocol_version, LATEST_PROTOCOL_VERSION);
        assert_eq!(result.server_info.name, "weather-server-001");
        assert_eq!(result.server_info.version, "0.1.0");
        assert!(result.capabilities.tools.is_some());
        assert!(result.instructions.is_none());
    }

    #[test]
    fn test_build_initialize_result_without_tools() {
        let config = McpServerConfig {
            id: "empty-server".to_string(),
            tools: vec![],
        };
        let result = build_initialize_result(&config);

        assert_eq!(result.server_info.name, "empty-server");
        assert!(result.capabilities.tools.is_none());
    }
}
