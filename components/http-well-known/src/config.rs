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

/// The discovery documents of one application, keyed by authentication profile id and then by
/// document name (`oauth-protected-resource`, `oauth-authorization-server`).
#[derive(Debug, Deserialize)]
pub struct WellKnownConfig {
    pub profiles: HashMap<String, HashMap<String, Value>>,
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
    fn test_parse_config_reads_documents_per_profile() {
        let config = parse_config(
            r#"{
                "app": "my-app",
                "mcp-servers": [],
                "well-known": {
                    "profiles": {
                        "profile-1": {
                            "oauth-protected-resource": { "resource": "https://app.betty.app/profile-1" },
                            "oauth-authorization-server": { "issuer": "https://app.betty.app/profile-1" }
                        },
                        "profile-2": {
                            "oauth-protected-resource": { "resource": "https://app.betty.app/profile-2" },
                            "oauth-authorization-server": { "issuer": "https://app.betty.app/profile-2" }
                        },
                        "profile-3": {
                            "oauth-protected-resource": { "resource": "https://app.betty.app/profile-3" },
                            "oauth-authorization-server": { "issuer": "https://app.betty.app/profile-3" }
                        }
                    }
                }
            }"#,
        )
        .unwrap();

        assert_eq!(config.profiles.len(), 3);
        for profile in ["profile-1", "profile-2", "profile-3"] {
            let documents = &config.profiles[profile];
            assert_eq!(
                documents["oauth-protected-resource"]["resource"],
                format!("https://app.betty.app/{profile}")
            );
            assert_eq!(
                documents["oauth-authorization-server"]["issuer"],
                format!("https://app.betty.app/{profile}")
            );
        }
    }

    #[test]
    fn test_parse_config_empty_profiles() {
        let config = parse_config(r#"{"well-known": {"profiles": {}}}"#).unwrap();
        assert!(config.profiles.is_empty());
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

    #[test]
    fn test_parse_config_missing_profiles_key() {
        let result = parse_config(r#"{"well-known": {"other": {}}}"#);
        assert!(result.unwrap_err().contains("Failed to parse"));
    }
}
