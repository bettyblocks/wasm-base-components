use std::collections::HashMap;

use serde::Deserialize;

use crate::wasi::cli::environment::get_environment;
use crate::wasi::config::store::get;

const WEBHOOKS_KEY: &str = "webhooks";
const ACTIONS_KEY: &str = "actions";
const BETTY_CONFIG_KEY: &str = "betty_config";
const WASMCLOUD_HOST_ENV: &str = "WASMCLOUD_HOST";
const APPLICATION_ID_ENV: &str = "APPLICATION_ID";

pub struct Config {
    pub wasmcloud_host: String,
    pub application_id: String,
    pub configurations: String,
    webhook_actions: HashMap<String, String>,
    action_auth: HashMap<String, ActionAuth>,
}

#[derive(Debug, Deserialize, PartialEq)]
struct ActionAuth {
    #[serde(rename = "authentication-profile-id", default)]
    authentication_profile_id: Option<String>,
}

impl Config {
    pub fn load() -> Result<Self, String> {
        let env: HashMap<String, String> = get_environment().into_iter().collect();
        let env_var = |name: &str| {
            env.get(name)
                .cloned()
                .ok_or_else(|| format!("Environment variable '{name}' not set"))
        };

        Ok(Self {
            wasmcloud_host: env_var(WASMCLOUD_HOST_ENV)?,
            application_id: env_var(APPLICATION_ID_ENV)?,
            configurations: parse_betty_config(load(BETTY_CONFIG_KEY)?)?,
            webhook_actions: parse(WEBHOOKS_KEY, &load(WEBHOOKS_KEY)?)?,
            action_auth: parse(ACTIONS_KEY, &load(ACTIONS_KEY)?)?,
        })
    }

    pub fn action_id(&self, webhook_name: &str) -> Option<&str> {
        self.webhook_actions.get(webhook_name).map(String::as_str)
    }

    pub fn is_public(&self, action_id: &str) -> Result<bool, String> {
        self.action_auth
            .get(action_id)
            .map(|action| action.authentication_profile_id.is_none())
            .ok_or_else(|| format!("Action '{action_id}' not found in {ACTIONS_KEY} config"))
    }
}

fn load(key: &str) -> Result<String, String> {
    get(key)
        .map_err(|e| format!("Failed to get wasi config: {e:?}"))?
        .ok_or_else(|| format!("{key} key not found in runtime configuration"))
}

fn parse<T: for<'de> Deserialize<'de>>(key: &str, raw: &str) -> Result<T, String> {
    serde_json::from_str(raw).map_err(|e| format!("Failed to parse {key} config: {e}"))
}

fn parse_betty_config(raw: String) -> Result<String, String> {
    parse::<serde::de::IgnoredAny>(BETTY_CONFIG_KEY, &raw)?;
    Ok(raw)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(actions: &str) -> Config {
        Config {
            wasmcloud_host: String::new(),
            application_id: String::new(),
            configurations: "{}".to_string(),
            webhook_actions: parse(
                WEBHOOKS_KEY,
                r#"{"send-invoice": "action-1", "ping": "action-2"}"#,
            )
            .unwrap(),
            action_auth: parse(ACTIONS_KEY, actions).unwrap(),
        }
    }

    #[test]
    fn test_action_id() {
        let config = config("{}");
        assert_eq!(config.action_id("send-invoice"), Some("action-1"));
        assert_eq!(config.action_id("action-1"), None);
    }

    #[test]
    fn test_is_public() {
        let config = config(
            r#"{
                "action-1": {"authentication-profile-id": "profile-1", "kind": "api-key"},
                "action-2": {"authentication-profile-id": null},
                "action-3": {}
            }"#,
        );
        assert_eq!(config.is_public("action-1"), Ok(false));
        assert_eq!(config.is_public("action-2"), Ok(true));
        assert_eq!(config.is_public("action-3"), Ok(true));
        assert!(config.is_public("unknown").is_err());
    }

    #[test]
    fn test_invalid_config() {
        assert!(parse::<HashMap<String, String>>(WEBHOOKS_KEY, "not json").is_err());
        assert!(parse::<HashMap<String, String>>(WEBHOOKS_KEY, r#"{"ping": {}}"#).is_err());
        assert!(parse_betty_config("{".to_string()).is_err());
        assert_eq!(parse_betty_config("{}".to_string()), Ok("{}".to_string()));
    }
}
