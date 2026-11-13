use std::collections::HashMap;

use serde::Deserialize;

use crate::wasi::cli::environment::get_environment;
use crate::wasi::config::store::get;

const WEBHOOKS_KEY: &str = "webhooks";
const BETTY_CONFIG_KEY: &str = "betty_config";
const WASMCLOUD_HOST_ENV: &str = "WASMCLOUD_HOST";
const APPLICATION_ID_ENV: &str = "APPLICATION_ID";

pub struct Config {
    pub wasmcloud_host: String,
    pub application_id: String,
    pub configurations: String,
    webhook_actions: HashMap<String, String>,
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
        })
    }

    pub fn action_id(&self, webhook_name: &str) -> Option<&str> {
        self.webhook_actions.get(webhook_name).map(String::as_str)
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

    fn config() -> Config {
        Config {
            wasmcloud_host: String::new(),
            application_id: String::new(),
            configurations: "{}".to_string(),
            webhook_actions: parse(
                WEBHOOKS_KEY,
                r#"{"send-invoice": "action-1", "ping": "action-2"}"#,
            )
            .unwrap(),
        }
    }

    #[test]
    fn test_action_id() {
        let config = config();
        assert_eq!(config.action_id("send-invoice"), Some("action-1"));
        assert_eq!(config.action_id("action-1"), None);
    }

    #[test]
    fn test_invalid_config() {
        assert!(parse::<HashMap<String, String>>(WEBHOOKS_KEY, "not json").is_err());
        assert!(parse::<HashMap<String, String>>(WEBHOOKS_KEY, r#"{"ping": {}}"#).is_err());
        assert!(parse_betty_config("{".to_string()).is_err());
        assert_eq!(parse_betty_config("{}".to_string()), Ok("{}".to_string()));
    }
}
