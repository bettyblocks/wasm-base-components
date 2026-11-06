use std::collections::HashMap;

use crate::types::{McpServerConfig, McpServersConfig, McpsEntry};
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

/// An action is public when it has no authentication profile.
/// The `actions` map sits inside the `mcp_servers` config, next to `mcps`.
pub fn is_public_action(action_id: &str) -> Result<bool, String> {
    let raw = get(WASI_CONFIG_KEY)
        .map_err(|e| format!("Failed to get wasi config: {e:?}"))?
        .ok_or_else(|| "mcp_servers key not found in runtime configuration".to_string())?;

    is_public_action_in(&raw, action_id)
}

fn is_public_action_in(raw: &str, action_id: &str) -> Result<bool, String> {
    #[derive(serde::Deserialize)]
    struct ActionAuth {
        #[serde(rename = "authentication-profile-id")]
        authentication_profile_id: Option<String>,
    }

    #[derive(serde::Deserialize)]
    struct ActionsConfig {
        #[serde(default)]
        actions: HashMap<String, ActionAuth>,
    }

    let mut config: ActionsConfig =
        serde_json::from_str(raw).map_err(|e| format!("Failed to parse actions config: {e}"))?;
    let action = config
        .actions
        .remove(action_id)
        .ok_or_else(|| format!("Action '{action_id}' not found in actions config"))?;

    Ok(action.authentication_profile_id.is_none())
}

pub fn load_betty_config() -> Result<String, String> {
    let betty_config_string = get(BETTY_CONFIG_KEY)
        .map_err(|e| format!("Failed to get betty config: {e:?}"))?
        .ok_or_else(|| "mcp_servers key not found in runtime configuration".to_string())?;

    let _: serde::de::IgnoredAny = serde_json::from_str(&betty_config_string)
        .map_err(|e| format!("unable to decode betty config {e}"))?;

    Ok(betty_config_string)
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
    let mut config: McpServersConfig = serde_json::from_str(raw)
        .map_err(|e| format!("Failed to parse mcp_servers config: {e}"))?;

    let mut server = config
        .mcp_servers
        .into_iter()
        .find(|s| s.id == server_id)
        .ok_or_else(|| format!("MCP server '{server_id}' not found in configuration"))?;
    // `remove` hands over the owned entry, so nothing has to be cloned out of the config
    server.auth = match config.mcps.remove(server_id) {
        Some(McpsEntry::Auth(auth)) => Some(auth),
        Some(McpsEntry::Public(_)) | None => None,
    };

    Ok(server)
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_ACTIONS_CONFIG: &str = r#"{
        "actions": {
            "pub": {"authentication-profile-id": null},
            "pub-no-field": {},
            "priv": {"authentication-profile-id": "profile-abc"}
        },
        "app": "x",
        "mcp-servers": [],
        "mcps": {}
    }"#;

    #[test]
    fn test_is_public_action() {
        assert_eq!(is_public_action_in(TEST_ACTIONS_CONFIG, "pub"), Ok(true));
        assert_eq!(
            is_public_action_in(TEST_ACTIONS_CONFIG, "pub-no-field"),
            Ok(true)
        );
        assert_eq!(is_public_action_in(TEST_ACTIONS_CONFIG, "priv"), Ok(false));
        assert!(is_public_action_in(TEST_ACTIONS_CONFIG, "nope").is_err());
    }
    use crate::types::McpAuthConfig;

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
    fn test_parse_server_config_has_no_auth_without_mcps() {
        let config = parse_server_config(TEST_MCP_CONFIG, "weather-server-001").unwrap();
        assert_eq!(config.auth, None);
    }

    #[test]
    fn test_parse_server_config_reads_resource_metadata_url_of_the_server() {
        let raw = TEST_MCP_CONFIG.replacen(
            "\"mcp-servers\"",
            r#""mcps": {
                "weather-server-001": {
                    "authentication-profile-id": "profile-1",
                    "kind": "data-api",
                    "resource-metadata-url": "https://app.betty.app/.well-known/oauth-protected-resource/api/runtime/app/authentication_profile/profile-1"
                },
                "calculator-server-001": {
                    "authentication-profile-id": "profile-2",
                    "kind": "api-key"
                }
            },
            "mcp-servers""#,
            1,
        );

        let weather = parse_server_config(&raw, "weather-server-001").unwrap();
        assert_eq!(
            weather.auth,
            Some(McpAuthConfig::DataApi {
                resource_metadata_url: "https://app.betty.app/.well-known/oauth-protected-resource/api/runtime/app/authentication_profile/profile-1".to_string()
            })
        );

        let calculator = parse_server_config(&raw, "calculator-server-001").unwrap();
        assert_eq!(calculator.auth, Some(McpAuthConfig::ApiKey));
    }

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
            auth: None,
        };
        let result = build_initialize_result(&config);

        assert_eq!(result.server_info.name, "empty-server");
        assert!(result.capabilities.tools.is_none());
    }
}
