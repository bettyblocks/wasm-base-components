use rust_mcp_schema::ContentBlock;
use serde_json::{json, Value};
use wstd::http::{Body, Client, Method, Request};
use wstd::time::Duration;

const MAX_ACTION_RESPONSE_SIZE: u64 = 10 * 1024 * 1024; // 10 MB

pub async fn execute_mapped_action(
    action_id: &str,
    arguments: &Value,
    configurations: &str,
    wasmcloud_host: &str,
    application_id: &str,
) -> Result<(bool, Vec<ContentBlock>), String> {
    let input_json = serde_json::to_string(arguments)
        .map_err(|e| format!("Failed to serialize arguments: {e}"))?;

    let request_body = json!({
        "action_id": action_id,
        "payload": {
            "input": input_json,
            "configurations": configurations
        },
        // The HTTP wrapper we call does not validate or use the JWT;
        // it is included only because the endpoint schema requires it.
        // TODO: Should we pass through the caller's JWT instead of an empty string?
        "jwt": ""
    });

    let body_bytes = serde_json::to_vec(&request_body)
        .map_err(|e| format!("Failed to serialize request body: {e}"))?;

    let response_body = send_http_request(wasmcloud_host, application_id, &body_bytes).await?;

    let data: Value = match serde_json::from_str(&response_body) {
        Ok(v) => v,
        Err(_) => {
            return Ok((
                true,
                vec![ContentBlock::text_content(format!(
                    "Failed to parse action response: {response_body}"
                ))],
            ))
        }
    };
    let output_value = data.get("output").cloned().unwrap_or(Value::Null);
    Ok((false, parse_action_output(&output_value)))
}

async fn send_http_request(
    wasmcloud_host: &str,
    application_id: &str,
    body_bytes: &[u8],
) -> Result<String, String> {
    let uri = format!("{wasmcloud_host}/");
    let request = Request::builder()
        .method(Method::POST)
        .uri(&uri)
        // NOTE: WASI prohibits to set the host header so as a workaround
        // we set this header. The runtime-gateway then sets the host header
        // with this value
        .header("x-route-host", application_id)
        .header("content-type", "application/json")
        .body(Body::from(body_bytes.to_vec()))
        .map_err(|e| format!("Failed to build request: {e}"))?;

    let timeout = Duration::from_secs(10 * 60); // 10 minutes
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

    crate::reject_oversized_body("Action response", &response_text, MAX_ACTION_RESPONSE_SIZE)?;

    if !status.is_success() {
        return Err(format!(
            "Action HTTP request failed with status {status}: {response_text}"
        ));
    }
    Ok(response_text)
}

/// Best-effort early rejection based on response content-length (may be absent).
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

pub fn parse_action_output(output: &Value) -> Vec<ContentBlock> {
    match output {
        Value::Null => vec![],
        Value::String(s) => vec![ContentBlock::text_content(s.clone())],
        other => vec![ContentBlock::text_content(
            serde_json::to_string_pretty(other).unwrap_or_else(|_| other.to_string()),
        )],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_parse_action_output_null() {
        let content = parse_action_output(&Value::Null);
        assert!(
            content.is_empty(),
            "null output should produce no content blocks"
        );
    }

    #[test]
    fn test_parse_action_output_string() {
        let content = parse_action_output(&json!("Hello, world!"));
        assert_eq!(content.len(), 1);
        match &content[0] {
            ContentBlock::TextContent(t) => assert_eq!(t.text, "Hello, world!"),
            _ => panic!("expected TextContent"),
        }
    }

    #[test]
    fn test_parse_action_output_object() {
        let content = parse_action_output(&json!({"key": "value", "num": 42}));
        assert_eq!(content.len(), 1);
        match &content[0] {
            ContentBlock::TextContent(t) => {
                assert!(t.text.contains("key"));
                assert!(t.text.contains("value"));
            }
            _ => panic!("expected TextContent"),
        }
    }

    #[test]
    fn test_parse_action_output_number() {
        let content = parse_action_output(&json!(42));
        assert_eq!(content.len(), 1);
        match &content[0] {
            ContentBlock::TextContent(t) => assert_eq!(t.text, "42"),
            _ => panic!("expected TextContent"),
        }
    }

    #[test]
    fn test_parse_action_output_array() {
        let content = parse_action_output(&json!([1, 2, 3]));
        assert_eq!(content.len(), 1);
        match &content[0] {
            ContentBlock::TextContent(t) => {
                assert!(t.text.contains('1'));
                assert!(t.text.contains('3'));
            }
            _ => panic!("expected TextContent"),
        }
    }
}
