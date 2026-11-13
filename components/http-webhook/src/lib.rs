use action_client::ActionRequest;
use serde_json::{json, Map, Value};
use wstd::http::{Body, HeaderMap, Method, Request, Response, StatusCode};

use crate::config::Config;

wit_bindgen::generate!({
    world: "webhook",
    generate_all,
});

mod config;

const PATH_PREFIX: &str = "/api/webhook/";
const REQUEST_KEY: &str = "__request";
const MAX_REQUEST_BODY_SIZE: u64 = 5 * 1024 * 1024;

type Failure = (StatusCode, String);

#[wstd::http_server]
async fn main(request: Request<Body>) -> Result<Response<Body>, wstd::http::Error> {
    Ok(match handle(request).await {
        Ok(output) => json_response(StatusCode::OK, &output),
        Err((status, message)) => error_response(status, &message),
    })
}

async fn handle(request: Request<Body>) -> Result<Value, Failure> {
    let name = route(&request)?;
    let config = Config::load().map_err(with_status(StatusCode::INTERNAL_SERVER_ERROR))?;
    let action_id = config
        .action_id(&name)
        .ok_or_else(|| (StatusCode::NOT_FOUND, format!("Webhook '{name}' not found")))?;
    let input = read_input(request).await?;

    let action_request = ActionRequest {
        wasmcloud_host: &config.wasmcloud_host,
        application_id: &config.application_id,
        action_id,
        input: &input,
        configurations: &config.configurations,
        jwt: None,
    };
    action_client::call_action(&action_request)
        .await
        .map_err(|e| (StatusCode::BAD_GATEWAY, e.to_string()))
}

fn route(request: &Request<Body>) -> Result<String, Failure> {
    webhook_name(request.uri().path())
        .map(str::to_string)
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                format!("Not found. Expected {PATH_PREFIX}{{webhook-name}}"),
            )
        })
}

async fn read_input(request: Request<Body>) -> Result<Value, Failure> {
    let method = request.method().clone();
    if method != Method::GET && method != Method::POST {
        return Err((
            StatusCode::METHOD_NOT_ALLOWED,
            "Method not allowed. Expected GET or POST".to_string(),
        ));
    }
    let headers = header_object(request.headers());
    let body = if method == Method::POST {
        read_body(request).await?
    } else {
        String::new()
    };
    Ok(action_input(method.as_str(), headers, body))
}

async fn read_body(request: Request<Body>) -> Result<String, Failure> {
    if let Some(content_length) = content_length(request.headers()) {
        reject_oversized(content_length)?;
    }
    let mut body = request.into_body();
    let body = body.str_contents().await.map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            format!("Failed to read request body: {e}"),
        )
    })?;
    reject_oversized(body.len() as u64)?;
    Ok(body.to_string())
}

fn with_status(status: StatusCode) -> impl Fn(String) -> Failure {
    move |message| (status, message)
}

fn webhook_name(path: &str) -> Option<&str> {
    path.strip_prefix(PATH_PREFIX)
        .filter(|name| !name.is_empty() && !name.contains('/'))
}

fn action_input(method: &str, headers: Map<String, Value>, body: String) -> Value {
    let mut input = match serde_json::from_str::<Value>(&body) {
        Ok(Value::Object(fields)) => fields,
        _ => Map::new(),
    };
    input.insert(
        REQUEST_KEY.to_string(),
        json!({ "method": method, "headers": headers, "body": body }),
    );
    Value::Object(input)
}

fn header_object(headers: &HeaderMap) -> Map<String, Value> {
    headers
        .keys()
        .map(|name| {
            let value = headers
                .get_all(name)
                .iter()
                .map(|value| String::from_utf8_lossy(value.as_bytes()))
                .collect::<Vec<_>>()
                .join(", ");
            (name.as_str().to_string(), Value::String(value))
        })
        .collect()
}

fn content_length(headers: &HeaderMap) -> Option<u64> {
    headers
        .get("content-length")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse().ok())
}

fn reject_oversized(size: u64) -> Result<(), Failure> {
    if size > MAX_REQUEST_BODY_SIZE {
        return Err((
            StatusCode::PAYLOAD_TOO_LARGE,
            format!(
                "Request body too large: {size} bytes exceeds {MAX_REQUEST_BODY_SIZE} byte limit"
            ),
        ));
    }
    Ok(())
}

fn json_response(status: StatusCode, body: &Value) -> Response<Body> {
    Response::builder()
        .status(status)
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap_or_else(|_| Response::new(Body::from("Internal Server Error")))
}

fn error_response(status: StatusCode, message: &str) -> Response<Body> {
    json_response(status, &json!({ "error": message }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_webhook_name() {
        assert_eq!(
            webhook_name("/api/webhook/send-invoice"),
            Some("send-invoice")
        );
        assert_eq!(webhook_name("/api/webhook/"), None);
        assert_eq!(webhook_name("/api/webhook"), None);
        assert_eq!(webhook_name("/api/webhook/a/b"), None);
        assert_eq!(webhook_name("/api/mcp/send-invoice"), None);
    }

    fn request(method: &str, body: &str) -> Value {
        json!({ "method": method, "headers": {}, "body": body })
    }

    #[test]
    fn test_action_input() {
        let input = |body: &str| action_input("POST", Map::new(), body.to_string());

        assert_eq!(
            input(r#"{"amount": 12}"#),
            json!({ "amount": 12, "__request": request("POST", r#"{"amount": 12}"#) })
        );
        assert_eq!(
            input(r#"{"__request": "spoofed"}"#),
            json!({ "__request": request("POST", r#"{"__request": "spoofed"}"#) })
        );
        assert_eq!(
            input("token=abc&command=%2Fdeploy"),
            json!({ "__request": request("POST", "token=abc&command=%2Fdeploy") })
        );
        assert_eq!(
            input("[1, 2]"),
            json!({ "__request": request("POST", "[1, 2]") })
        );
        assert_eq!(input(""), json!({ "__request": request("POST", "") }));
    }

    #[test]
    fn test_header_object() {
        let mut headers = HeaderMap::new();
        headers.append("X-Slack-Signature", "v0=abc".parse().unwrap());
        headers.append("accept", "text/html".parse().unwrap());
        headers.append("accept", "application/json".parse().unwrap());

        assert_eq!(
            Value::Object(header_object(&headers)),
            json!({ "x-slack-signature": "v0=abc", "accept": "text/html, application/json" })
        );
    }
}
