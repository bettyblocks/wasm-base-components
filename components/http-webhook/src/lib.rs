use action_client::{ActionRequest, BearerToken};
use serde_json::{json, Value};
use wstd::http::{Body, HeaderMap, Method, Request, Response, StatusCode};

use crate::betty_blocks_types::auth::jwt::{allowed_to_call, AuthError};
use crate::config::Config;

wit_bindgen::generate!({
    world: "webhook",
    generate_all,
});

mod config;

const PATH_PREFIX: &str = "/api/webhooks/";
const MAX_REQUEST_BODY_SIZE: u64 = 10 * 1024 * 1024;

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
    let jwt = authorize(request.headers(), &config, action_id)?;
    let input = read_input(request).await?;

    let action_request = ActionRequest {
        wasmcloud_host: &config.wasmcloud_host,
        application_id: &config.application_id,
        action_id,
        input: &input,
        configurations: &config.configurations,
        jwt: jwt.as_deref(),
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

fn authorize(
    headers: &HeaderMap,
    config: &Config,
    action_id: &str,
) -> Result<Option<BearerToken>, Failure> {
    let headers = header_entries(headers);
    allowed_to_call(&headers, &config.application_id, action_id).map_err(auth_failure)?;

    let public = config
        .is_public(action_id)
        .map_err(with_status(StatusCode::INTERNAL_SERVER_ERROR))?;
    Ok(if public {
        None
    } else {
        action_client::extract_bearer_token(&headers)
    })
}

async fn read_input(request: Request<Body>) -> Result<Value, Failure> {
    match *request.method() {
        Method::GET => Ok(json!({})),
        Method::POST => read_body(request).await,
        _ => Err((
            StatusCode::METHOD_NOT_ALLOWED,
            "Method not allowed. Expected GET or POST".to_string(),
        )),
    }
}

async fn read_body(request: Request<Body>) -> Result<Value, Failure> {
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
    action_input(body).map_err(with_status(StatusCode::BAD_REQUEST))
}

fn with_status(status: StatusCode) -> impl Fn(String) -> Failure {
    move |message| (status, message)
}

fn auth_failure(error: AuthError) -> Failure {
    let (status, message) = match error {
        AuthError::MalformedToken => (StatusCode::UNAUTHORIZED, "malformed token".to_string()),
        AuthError::ValidationFailed(msg) => (StatusCode::UNAUTHORIZED, msg),
        AuthError::Forbidden(msg) => (StatusCode::FORBIDDEN, msg),
        AuthError::Unavailable(msg) => (StatusCode::SERVICE_UNAVAILABLE, msg),
        AuthError::MissingConfig(msg) => (StatusCode::INTERNAL_SERVER_ERROR, msg),
    };
    (status, format!("Auth error: {message}"))
}

fn webhook_name(path: &str) -> Option<&str> {
    path.strip_prefix(PATH_PREFIX)
        .filter(|name| !name.is_empty() && !name.contains('/'))
}

fn action_input(body: &str) -> Result<Value, String> {
    if body.trim().is_empty() {
        return Ok(json!({}));
    }
    match serde_json::from_str::<Value>(body) {
        Ok(input @ Value::Object(_)) => Ok(input),
        Ok(_) => Err("Request body must be a JSON object".to_string()),
        Err(e) => Err(format!("Request body is not valid JSON: {e}")),
    }
}

fn header_entries(headers: &HeaderMap) -> Vec<(String, Vec<u8>)> {
    headers
        .iter()
        .map(|(k, v)| (k.as_str().to_string(), v.as_bytes().to_vec()))
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
    let mut response = json_response(status, &json!({ "error": message }));
    if status == StatusCode::UNAUTHORIZED {
        response
            .headers_mut()
            .insert("www-authenticate", "Bearer".parse().expect("valid header"));
    }
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_webhook_name() {
        assert_eq!(
            webhook_name("/api/webhooks/send-invoice"),
            Some("send-invoice")
        );
        assert_eq!(webhook_name("/api/webhooks/"), None);
        assert_eq!(webhook_name("/api/webhooks"), None);
        assert_eq!(webhook_name("/api/webhooks/a/b"), None);
        assert_eq!(webhook_name("/api/mcp/send-invoice"), None);
    }

    #[test]
    fn test_action_input() {
        assert_eq!(action_input(""), Ok(json!({})));
        assert_eq!(action_input(" \n"), Ok(json!({})));
        assert_eq!(action_input(r#"{"amount": 12}"#), Ok(json!({"amount": 12})));
        assert!(action_input("[1, 2]").unwrap_err().contains("JSON object"));
        assert!(action_input("{").unwrap_err().contains("not valid JSON"));
    }

    #[test]
    fn test_auth_failure_status() {
        let status = |error| auth_failure(error).0;
        assert_eq!(status(AuthError::MalformedToken), StatusCode::UNAUTHORIZED);
        assert_eq!(
            status(AuthError::ValidationFailed("expired".into())),
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            status(AuthError::Forbidden("no".into())),
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            status(AuthError::Unavailable("down".into())),
            StatusCode::SERVICE_UNAVAILABLE
        );
        assert_eq!(
            status(AuthError::MissingConfig("missing".into())),
            StatusCode::INTERNAL_SERVER_ERROR
        );
    }
}
