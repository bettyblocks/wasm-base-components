use std::collections::HashMap;

use crate::wasi::cli::environment::get_environment;
use crate::wasi::config::store::get;
use serde::Deserialize;
use serde_json::Value;

/// The config of the mcp servers of the application, shared with the http-mcp component.
const WASI_CONFIG_KEY: &str = "mcp_servers";
const APPLICATION_ID_ENV: &str = "APPLICATION_ID";

/// The part of the mcp config that is read by this component, the rest is for http-mcp.
#[derive(Debug, Deserialize)]
struct McpServersConfig {
    #[serde(rename = "well-known")]
    well_known: WellKnownConfig,
}

/// The discovery documents of one application.
#[derive(Debug, Deserialize)]
pub struct WellKnownConfig {
    /// One document per mcp server, keyed by mcp id.
    #[serde(rename = "oauth-protected-resource", default)]
    pub protected_resources: HashMap<String, Value>,
    /// One document per authentication profile, keyed by profile id.
    #[serde(rename = "oauth-authorization-server", default)]
    pub authorization_servers: HashMap<String, Value>,
}

pub fn load_application_id() -> Result<String, String> {
    let env: HashMap<String, String> = get_environment().into_iter().collect();

    env.get(APPLICATION_ID_ENV)
        .cloned()
        .ok_or_else(|| format!("Environment variable '{APPLICATION_ID_ENV}' not set"))
}

pub fn load_config() -> Result<WellKnownConfig, String> {
    let raw = get(WASI_CONFIG_KEY)
        .map_err(|e| format!("Failed to get wasi config: {e:?}"))?
        .ok_or_else(|| format!("{WASI_CONFIG_KEY} key not found in runtime configuration"))?;

    parse_config(&raw)
}

pub fn parse_config(raw: &str) -> Result<WellKnownConfig, String> {
    serde_json::from_str::<McpServersConfig>(raw)
        .map(|config| config.well_known)
        .map_err(|e| format!("Failed to parse {WASI_CONFIG_KEY} config: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_config_reads_documents_per_mcp_server_and_profile() {
        let config = parse_config(
            r#"{
                "app": "my-app",
                "mcp-servers": [],
                "well-known": {
                    "oauth-protected-resource": {
                        "server-1": { "resource": "https://app.betty.app/api/mcp/server-1" },
                        "server-2": { "resource": "https://app.betty.app/api/mcp/server-2" },
                        "server-3": { "resource": "https://app.betty.app/api/mcp/server-3" }
                    },
                    "oauth-authorization-server": {
                        "profile-1": { "issuer": "https://app.betty.app/profile-1" },
                        "profile-2": { "issuer": "https://app.betty.app/profile-2" }
                    }
                }
            }"#,
        )
        .unwrap();

        assert_eq!(config.protected_resources.len(), 3);
        for server in ["server-1", "server-2", "server-3"] {
            assert_eq!(
                config.protected_resources[server]["resource"],
                format!("https://app.betty.app/api/mcp/{server}")
            );
        }

        assert_eq!(config.authorization_servers.len(), 2);
        for profile in ["profile-1", "profile-2"] {
            assert_eq!(
                config.authorization_servers[profile]["issuer"],
                format!("https://app.betty.app/{profile}")
            );
        }
    }

    #[test]
    fn test_parse_config_documents_default_to_empty() {
        let config = parse_config(r#"{"well-known": {}}"#).unwrap();
        assert!(config.protected_resources.is_empty());
        assert!(config.authorization_servers.is_empty());
    }

    #[test]
    fn test_parse_config_invalid_json() {
        let result = parse_config("not json");
        assert!(result.unwrap_err().contains("Failed to parse"));
    }

    #[test]
    fn test_parse_config_missing_well_known_key() {
        let result = parse_config(r#"{"mcp-servers": []}"#);
        assert!(result.unwrap_err().contains("Failed to parse"));
    }
}
