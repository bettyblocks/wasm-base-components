use std::fmt;

use serde_json::{json, Value};
use wstd::http::{Body, Client, Method, Request};
use wstd::time::Duration;

const MAX_ACTION_RESPONSE_SIZE: u64 = 10 * 1024 * 1024;

pub type BearerToken = String;

pub struct ActionRequest<'a> {
    pub wasmcloud_host: &'a str,
    pub application_id: &'a str,
    pub action_id: &'a str,
    pub input: &'a Value,
    pub configurations: &'a str,
    pub jwt: Option<&'a str>,
}

#[derive(Debug, PartialEq)]
pub enum ActionError {
    Failed(String),
    InvalidResponse(String),
}

impl fmt::Display for ActionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Failed(message) => f.write_str(message),
            Self::InvalidResponse(response) => {
                write!(f, "Failed to parse action response: {response}")
            }
        }
    }
}

pub async fn call_action(action_request: &ActionRequest<'_>) -> Result<Value, ActionError> {
    let response = send(action_request).await.map_err(ActionError::Failed)?;
    parse_output(&response)
}

async fn send(action_request: &ActionRequest<'_>) -> Result<String, String> {
    let body = json!({
        "action_id": action_request.action_id,
        "payload": {
            "input": action_request.input.to_string(),
            "configurations": action_request.configurations
        },
        "jwt": action_request.jwt.unwrap_or_default()
    });
    let body_bytes =
        serde_json::to_vec(&body).map_err(|e| format!("Failed to serialize request body: {e}"))?;

    let request = Request::builder()
        .method(Method::POST)
        .uri(format!("{}/", action_request.wasmcloud_host))
        // NOTE: WASI prohibits to set the host header so as a workaround
        // we set this header. The runtime-gateway then sets the host header
        // with this value
        .header("x-route-host", action_request.application_id)
        .header("content-type", "application/json")
        .body(Body::from(body_bytes))
        .map_err(|e| format!("Failed to build request: {e}"))?;

    let timeout = Duration::from_secs(10 * 60);
    let mut client = Client::new();
    client.set_connect_timeout(timeout);
    client.set_first_byte_timeout(timeout);
    client.set_between_bytes_timeout(timeout);

    let response = client
        .send(request)
        .await
        .map_err(|e| format!("HTTP request failed (possibly timed out after 10 minutes): {e}"))?;

    let status = response.status();
    let mut body = response.into_body();

    reject_oversized_response_hint(&body, MAX_ACTION_RESPONSE_SIZE)?;

    let response_text = body
        .str_contents()
        .await
        .map_err(|e| format!("Failed to read response body: {e}"))?
        .to_string();

    reject_oversized_body(&response_text, MAX_ACTION_RESPONSE_SIZE)?;

    if !status.is_success() {
        return Err(format!(
            "Action HTTP request failed with status {status}: {response_text}"
        ));
    }
    Ok(response_text)
}

fn parse_output(response: &str) -> Result<Value, ActionError> {
    let mut data: Value = serde_json::from_str(response)
        .map_err(|_| ActionError::InvalidResponse(response.to_string()))?;
    Ok(data
        .get_mut("output")
        .map(Value::take)
        .unwrap_or(Value::Null))
}

pub fn extract_bearer_token(headers: &[(String, Vec<u8>)]) -> Option<BearerToken> {
    headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("authorization"))
        .and_then(|(_, value)| std::str::from_utf8(value).ok())
        .and_then(|value| {
            let (scheme, token) = value.trim().split_once(' ')?;
            scheme.eq_ignore_ascii_case("bearer").then(|| token.trim())
        })
        .map(str::to_string)
}

fn reject_oversized_response_hint(body: &Body, max_size: u64) -> Result<(), String> {
    if let Some(content_length) = body.content_length() {
        if content_length > max_size {
            return Err(format!(
                "Action response too large: {content_length} bytes exceeds {max_size} byte limit"
            ));
        }
    }
    Ok(())
}

fn reject_oversized_body(body: &str, max_size: u64) -> Result<(), String> {
    if body.len() as u64 > max_size {
        return Err(format!(
            "Action response too large: {} bytes exceeds {max_size} byte limit",
            body.len()
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_output() {
        assert_eq!(parse_output(r#"{"output": {"a": 1}}"#), Ok(json!({"a": 1})));
        assert_eq!(parse_output(r#"{"output": "pong"}"#), Ok(json!("pong")));
        assert_eq!(parse_output("{}"), Ok(Value::Null));
        assert_eq!(
            parse_output("oops"),
            Err(ActionError::InvalidResponse("oops".to_string()))
        );
    }

    #[test]
    fn test_extract_bearer_token() {
        let h = vec![("Authorization".to_string(), b"Bearer abc.def".to_vec())];
        assert_eq!(extract_bearer_token(&h), Some("abc.def".to_string()));
    }

    #[test]
    fn test_extract_bearer_token_missing_or_other_scheme() {
        assert_eq!(extract_bearer_token(&[]), None);
        let h = vec![("authorization".to_string(), b"Basic xyz".to_vec())];
        assert_eq!(extract_bearer_token(&h), None);
    }
}
