use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;
use std::{collections::HashMap, env, time::Duration};
use tracing::error;
use wstd::http::{Client, Request};

use crate::exports::betty_blocks_types::logs_writer::debug_logging::Guest;

wit_bindgen::generate!({ generate_all });

const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const FIRST_BYTE_TIMEOUT: Duration = Duration::from_secs(10);
const BETWEEN_BYTES_TIMEOUT: Duration = Duration::from_secs(10);

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

#[derive(Deserialize)]
struct MessageApplication {
    application_id: String,
}

#[derive(Serialize)]
struct LogsData<'a> {
    data: Vec<&'a RawValue>,
}

#[derive(Serialize)]
struct Variable<'a> {
    hash: &'a str,
    value: &'a RawValue,
}

#[derive(Serialize)]
struct VariablesData<'a> {
    data: HashMap<&'a str, Vec<Variable<'a>>>,
}

type VariablesPayload<'a> = HashMap<String, HashMap<String, &'a RawValue>>;

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

        let logs_body = match logs_body(&messages, &config.application_id) {
            Ok(body) => body,
            Err(err) => {
                log_error(&format!("logs writer messages parse failed: {err}"));
                return;
            }
        };

        let variables_body = match variables_body(&variables, &config.application_id) {
            Ok(body) => body,
            Err(err) => {
                log_error(&format!("logs writer variables parse failed: {err}"));
                return;
            }
        };

        let mut client = Client::new();
        client.set_connect_timeout(CONNECT_TIMEOUT);
        client.set_first_byte_timeout(FIRST_BYTE_TIMEOUT);
        client.set_between_bytes_timeout(BETWEEN_BYTES_TIMEOUT);

        wstd::runtime::block_on(futures_lite::future::zip(
            send_request(&client, &config, config.logs_address(), logs_body),
            send_request(&client, &config, config.variables_address(), variables_body),
        ));
    }
}

fn logs_body(messages: &[String], application_id: &str) -> Result<Vec<u8>, serde_json::Error> {
    let data = messages
        .iter()
        .filter_map(
            |message| match serde_json::from_str::<MessageApplication>(message) {
                Ok(m) if m.application_id == application_id => {
                    Some(serde_json::from_str::<&RawValue>(message))
                }
                Ok(_) => None,
                Err(err) => Some(Err(err)),
            },
        )
        .collect::<Result<Vec<_>, _>>()?;
    serde_json::to_vec(&LogsData { data })
}

fn variables_body(json: &str, application_id: &str) -> Result<Vec<u8>, serde_json::Error> {
    let payload: VariablesPayload = serde_json::from_str(json)?;
    let data = payload
        .iter()
        .filter(|(app_id, _)| app_id.as_str() == application_id)
        .map(|(app_id, app_vars)| {
            let variables = app_vars
                .iter()
                .map(|(hash, value)| Variable { hash, value })
                .collect();
            (app_id.as_str(), variables)
        })
        .collect();
    serde_json::to_vec(&VariablesData { data })
}

async fn send_request(client: &Client, config: &Config, url: String, body: Vec<u8>) {
    let jwt = match generate_jaws(config) {
        Ok(jwt) => jwt,
        Err(err) => {
            log_error(&format!(
                "logs writer jwt generation failed: {err} | url={url}"
            ));
            return;
        }
    };

    let request = match Request::post(&url)
        .header("Content-Type", "application/json")
        .header("Authorization", format!("Bearer {jwt}"))
        .body(body)
    {
        Ok(request) => request,
        Err(err) => {
            log_error(&format!(
                "logs writer request build failed | error={err} | url={url}"
            ));
            return;
        }
    };

    let mut response = match client.send(request).await {
        Ok(response) => response,
        Err(err) => {
            log_error(&format!(
                "logs writer request failed | error={err:?} | url={url}"
            ));
            return;
        }
    };

    let status_code = response.status().as_u16();
    if status_code != 201 {
        match response.body_mut().contents().await {
            Ok(body) => {
                let body_text = String::from_utf8_lossy(body);
                log_error(&format!(
                    "logs writer request returned unexpected status \
                     | status_code={status_code} | url={url} | response_body={body_text}"
                ));
            }
            Err(err) => {
                log_error(&format!(
                    "logs writer response body read failed \
                     | error={err:?} | status_code={status_code} | url={url}"
                ));
            }
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
    use serde_json::{Value, json};

    fn parse(body: Vec<u8>) -> Value {
        serde_json::from_slice(&body).expect("body is not valid JSON")
    }

    #[test]
    fn logs_body_keeps_only_messages_of_the_application() {
        let messages = vec![
            r#"{"application_id":"app-1","k":1,"nested":{"b":2,"a":1}}"#.to_string(),
            r#"{"application_id":"app-2","k":2}"#.to_string(),
            r#"{"application_id":"app-1","k":3}"#.to_string(),
        ];

        let body = parse(logs_body(&messages, "app-1").unwrap());

        assert_eq!(
            body,
            json!({"data": [
                {"application_id": "app-1", "k": 1, "nested": {"b": 2, "a": 1}},
                {"application_id": "app-1", "k": 3},
            ]})
        );
    }

    #[test]
    fn logs_body_is_empty_when_no_message_matches() {
        let messages = vec![r#"{"application_id":"app-2"}"#.to_string()];

        assert_eq!(
            parse(logs_body(&messages, "app-1").unwrap()),
            json!({"data": []})
        );
    }

    #[test]
    fn logs_body_rejects_invalid_json() {
        let messages = vec![
            r#"{"application_id":"app-1"}"#.to_string(),
            r#"{"application_id":"app-1"} trailing"#.to_string(),
        ];

        assert!(logs_body(&messages, "app-1").is_err());
    }

    #[test]
    fn variables_body_groups_the_variables_of_the_application() {
        let json = r#"{"app-1":{"h1":{"k":[1,2]},"h2":"v"},"app-2":{"h3":"other"}}"#;

        let body = parse(variables_body(json, "app-1").unwrap());

        let mut variables = body["data"]["app-1"].as_array().unwrap().clone();
        variables.sort_by_key(|v| v["hash"].as_str().unwrap().to_string());
        assert_eq!(
            variables,
            vec![
                json!({"hash": "h1", "value": {"k": [1, 2]}}),
                json!({"hash": "h2", "value": "v"}),
            ]
        );
        assert_eq!(body["data"].as_object().unwrap().len(), 1);
    }

    #[test]
    fn variables_body_rejects_invalid_json() {
        assert!(variables_body(r#"{"app-1":{"h1":1},"#, "app-1").is_err());
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
