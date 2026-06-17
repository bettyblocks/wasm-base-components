use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{collections::HashMap, env, time::Duration};
use tracing::error;

use crate::exports::betty_blocks_utilities::logs_writer::debug_logging::Guest;

wit_bindgen::generate!({ generate_all });

// tracing doesn't bubble up to the wasmcloud host yet, but eprintln does.
// Use this function to ensure errors are visible in both tracing subscribers and host stderr.
fn log_error(message: &str) {
    error!("{message}");
    eprintln!("ERROR: {message}");
}

struct LogsWriter;

#[derive(Debug)]
struct Config {
    application_id: String,
    scheme: String,
    host: String,
    port: u16,
    jaws_issuer: String,
    jaws_secret_key: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LogsWriterMessage {
    pub application_id: String,

    #[serde(flatten)]
    pub other: HashMap<String, Value>,
}

pub type VariablesPayload = HashMap<String, HashMap<String, Value>>;
pub type VariablesOutput = HashMap<String, Vec<Value>>;

impl Config {
    fn from_env() -> Result<Self, String> {
        let application_id = env::var("APPLICATION_ID")
            .map_err(|_| "APPLICATION_ID must be set to use the logs-writer".to_string())?;
        let scheme = env::var("LOGS_WRITER_SCHEME").unwrap_or_else(|_| "http".to_string());
        let host = env::var("LOGS_WRITER_HOST").unwrap_or_else(|_| "logs-writer".to_string());
        let port = env::var("LOGS_WRITER_PORT")
            .ok()
            .and_then(|val| val.parse().ok())
            .unwrap_or(80);
        let jaws_issuer = env::var("JAWS_ISSUER").unwrap_or_else(|_| "actions-wasm".to_string());
        let jaws_secret_key = env::var("ACTIONS_WASM_LOGS_WRITER_SECRET")
            .map_err(|_| "ACTIONS_WASM_LOGS_WRITER_SECRET must be set".to_string())?;

        Ok(Self {
            application_id,
            scheme,
            host,
            port,
            jaws_issuer,
            jaws_secret_key,
        })
    }

    fn address(&self) -> String {
        format!("{}://{}:{}/internal", self.scheme, self.host, self.port)
    }

    fn logs_address(&self) -> String {
        format!("{}/bulk_logs", self.address())
    }

    fn variables_address(&self) -> String {
        format!("{}/bulk_variables", self.address())
    }
}

impl Guest for LogsWriter {
    fn send_log_and_variables(messages: Vec<String>, variables: String) {
        let config = match Config::from_env() {
            Ok(config) => config,
            Err(err) => {
                log_error(&format!("logs writer config failed: {err}"));
                return;
            }
        };

        let jwt = match generate_jaws(&config) {
            Ok(jwt) => jwt,
            Err(err) => {
                log_error(&format!("logs writer jwt generation failed: {err}"));
                return;
            }
        };

        let logs_data = match parse_messages_json(&messages) {
            Ok(data) => filter_messages_by_application_id(data, &config.application_id),
            Err(err) => {
                log_error(&format!("logs writer messages parse failed: {err}"));
                return;
            }
        };

        let variables_data = match parse_variables_json(&variables) {
            Ok(data) => filter_variables_by_application_id(data, &config.application_id),
            Err(err) => {
                log_error(&format!("logs writer variables parse failed: {err}"));
                return;
            }
        };

        let client = waki::Client::new();
        let logs_payload = serde_json::json!({ "data": logs_data });
        let variables_payload = serde_json::json!({ "data": variables_data });

        send_request(&client, &config.logs_address(), &jwt, logs_payload);
        send_request(
            &client,
            &config.variables_address(),
            &jwt,
            variables_payload,
        );
    }
}

pub fn parse_messages_json(
    messages: &[String],
) -> Result<Vec<LogsWriterMessage>, serde_json::Error> {
    messages
        .iter()
        .map(|s| serde_json::from_str::<LogsWriterMessage>(s))
        .collect()
}

fn filter_messages_by_application_id(
    messages: Vec<LogsWriterMessage>,
    application_id: &str,
) -> Vec<LogsWriterMessage> {
    messages
        .into_iter()
        .filter(|message| message.application_id == application_id)
        .collect()
}

fn filter_variables_by_application_id(
    variables: VariablesOutput,
    application_id: &str,
) -> VariablesOutput {
    variables
        .into_iter()
        .filter(|(app_id, _)| app_id == application_id)
        .collect()
}

pub fn parse_variables_json(json: &str) -> Result<VariablesOutput, serde_json::Error> {
    let variable_payload: VariablesPayload = serde_json::from_str(json)?;

    Ok(variable_payload
        .into_iter()
        .map(|(application_id, app_vars)| {
            let variables: Vec<Value> = app_vars
                .into_iter()
                .map(|(hash, value)| serde_json::json!({ "hash": hash, "value": value }))
                .collect();
            (application_id, variables)
        })
        .collect())
}

fn send_request(client: &waki::Client, url: &str, jwt: &str, payload: serde_json::Value) {
    let body = match serde_json::to_vec(&payload) {
        Ok(body) => body,
        Err(err) => {
            log_error(&format!(
                "logs writer payload encode failed | url={url} | error={err}"
            ));
            return;
        }
    };

    let response = client
        .post(url)
        .header("Content-Type", "application/json")
        .header("Authorization", format!("Bearer {jwt}"))
        .connect_timeout(Duration::from_secs(5))
        .body(body)
        .send();

    match response {
        Ok(response) => {
            let status_code = response.status_code();
            if status_code != 201 {
                match response.body() {
                    Ok(body) => {
                        let body_text = String::from_utf8_lossy(&body);
                        log_error(&format!(
                            "logs writer request returned unexpected status \
                             | status_code={status_code} | url={url} | response_body={body_text}"
                        ));
                    }
                    Err(err) => {
                        log_error(&format!(
                            "logs writer response body read failed \
                             | error={err} | status_code={status_code} | url={url}"
                        ));
                    }
                }
            }
        }
        Err(err) => {
            log_error(&format!(
                "logs writer request failed | error={err} | url={url}"
            ));
        }
    }
}

fn generate_jaws(config: &Config) -> Result<String, String> {
    let issued_at = jaws_rs::jsonwebtoken::get_current_timestamp();

    let claims = jaws_rs::Claims::new(
        config.jaws_issuer.clone(),
        config.application_id.to_string(),
        issued_at,
        uuid::Uuid::new_v4().to_string(),
    );

    jaws_rs::encode(&claims, &config.jaws_secret_key)
        .map_err(|e| format!("Failed to encode JAWS token: {e:#}"))
}

export!(LogsWriter);

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn when_parsing_multiple_messages_it_should_return_all_entries() {
        let expected_result = vec![
            LogsWriterMessage {
                application_id: "app-1".to_string(),
                other: HashMap::from([("k".to_string(), serde_json::json!(1))]),
            },
            LogsWriterMessage {
                application_id: "app-2".to_string(),
                other: HashMap::from([("k".to_string(), serde_json::json!(2))]),
            },
        ];
        let messages = vec![
            r#"{"application_id":"app-1","k":1}"#.to_string(),
            r#"{"application_id":"app-2","k":2}"#.to_string(),
        ];
        let parsed = parse_messages_json(&messages).expect("messages parse failed");

        assert_eq!(parsed, expected_result);
    }

    #[test]
    fn when_parsing_messages_with_invalid_json_it_should_return_error() {
        let messages = vec![
            r#"{"application_id":"app-1"}"#.to_string(),
            r#"{"application_id": "bad""#.to_string(),
        ];

        let result = parse_messages_json(&messages);
        assert!(result.is_err());
    }

    #[test]
    fn when_parsing_variables_json_it_should_parse_successfully() {
        let json = r#"{"app-1":{"h1":{"k":1},"h2":"v"}}"#;
        let vars = parse_variables_json(json).expect("variables parse failed");

        assert_eq!(vars.len(), 1);
        let app_vars = vars.get("app-1").expect("app-1 should exist");
        assert_eq!(app_vars.len(), 2);
        assert!(app_vars.contains(&serde_json::json!({"hash": "h1", "value": {"k": 1}})));
        assert!(app_vars.contains(&serde_json::json!({"hash": "h2", "value": "v"})));
    }

    #[test]
    fn when_parsing_variables_json_it_should_serialize_as_grouped_map() {
        let json = r#"{"app-1":{"h1":"v1"}}"#;
        let vars = parse_variables_json(json).expect("variables parse failed");
        let serialized = serde_json::to_value(&vars).expect("serialize failed");

        assert_eq!(
            serialized,
            serde_json::json!({"app-1": [{"hash": "h1", "value": "v1"}]})
        );
    }

    #[test]
    fn when_parsing_variables_json_with_multiple_apps_it_should_group_by_app() {
        let json = r#"{"app-1":{"h1":"v1"},"app-2":{"h2":"v2"}}"#;
        let vars = parse_variables_json(json).expect("variables parse failed");

        assert_eq!(vars.len(), 2);
        let app1_vars = vars.get("app-1").expect("app-1 should exist");
        assert_eq!(
            app1_vars,
            &vec![serde_json::json!({"hash": "h1", "value": "v1"})]
        );
        let app2_vars = vars.get("app-2").expect("app-2 should exist");
        assert_eq!(
            app2_vars,
            &vec![serde_json::json!({"hash": "h2", "value": "v2"})]
        );
    }

    #[test]
    fn when_parsing_variables_with_invalid_json_it_should_return_error() {
        let json = r#"{"app-1":{"h1":1},"#;
        let result = parse_variables_json(json);
        assert!(result.is_err());
    }

    #[test]
    fn when_filtering_messages_it_should_only_keep_matching_application_id() {
        let expected_result = vec![LogsWriterMessage {
            application_id: "app-1".to_string(),
            other: HashMap::from([("k".to_string(), serde_json::json!(1))]),
        }];

        let messages = vec![
            LogsWriterMessage {
                application_id: "app-1".to_string(),
                other: HashMap::from([("k".to_string(), serde_json::json!(1))]),
            },
            LogsWriterMessage {
                application_id: "app-2".to_string(),
                other: HashMap::from([("k".to_string(), serde_json::json!(2))]),
            },
        ];

        let filtered = filter_messages_by_application_id(messages, "app-1");

        assert_eq!(filtered, expected_result);
    }

    #[test]
    fn when_filtering_variables_it_should_only_keep_matching_application_id() {
        let variables = HashMap::from([
            (
                "app-1".to_string(),
                vec![serde_json::json!({"hash": "h1", "value": "v1"})],
            ),
            (
                "app-2".to_string(),
                vec![serde_json::json!({"hash": "h2", "value": "v2"})],
            ),
        ]);

        let filtered = filter_variables_by_application_id(variables, "app-1");

        assert_eq!(filtered.len(), 1);
        assert!(filtered.contains_key("app-1"));
        assert!(!filtered.contains_key("app-2"));
    }

    #[test]
    fn when_generating_jaws_token_it_should_return_token() {
        let config = Config {
            application_id: "app-1".to_string(),
            scheme: "http".to_string(),
            host: "logs-writer".to_string(),
            port: 80,
            jaws_issuer: "issuer".to_string(),
            jaws_secret_key: "secret".to_string(),
        };

        let token = generate_jaws(&config).expect("token should be generated");
        assert!(!token.is_empty());
    }
}
