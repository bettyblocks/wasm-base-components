use serde_json::json;
use wstd::http::{Body, HeaderMap, Method, Request, Response, StatusCode};

wit_bindgen::generate!({
    world: "mcp",
    generate_all,
});

mod actions;
mod config;
mod mcp;
mod types;
mod validation;

const PATH_PREFIX: &str = "/api/mcp/";
const MAX_REQUEST_BODY_SIZE: u64 = 10 * 1024 * 1024; // 10 MB

#[wstd::http_server]
async fn main(request: Request<Body>) -> Result<Response<Body>, wstd::http::Error> {
    Ok(inner_handle(request).await)
}

async fn inner_handle(request: Request<Body>) -> Response<Body> {
    let path = request
        .uri()
        .path_and_query()
        .map(|pq| pq.to_string())
        .unwrap_or_default();
    match (request.method(), path.starts_with(PATH_PREFIX)) {
        (&Method::POST, true) => handle_mcp_request(request, &path).await,
        _ => json_response(
            StatusCode::METHOD_NOT_ALLOWED,
            json!({
                "jsonrpc": "2.0",
                "error": {
                    "code": -32000,
                    "message": format!("Method Not Allowed. Expected POST to {}{{server-id}}", PATH_PREFIX)
                },
                "id": null
            })
            .to_string(),
        ),
    }
}

async fn handle_mcp_request(request: Request<Body>, path: &str) -> Response<Body> {
    if let Err(e) = validate_content_type(request.headers()) {
        return json_rpc_error_response(StatusCode::BAD_REQUEST, -32600, &e);
    }

    let server_id = match extract_server_id_from_path(path) {
        Ok(id) => id,
        Err(e) => return json_rpc_error_response(StatusCode::BAD_REQUEST, -32600, &e),
    };

    let env_config = match config::EnvConfig::from_env() {
        Ok(c) => c,
        Err(e) => return json_rpc_error_response(StatusCode::INTERNAL_SERVER_ERROR, -32603, &e),
    };

    if let Err(e) = reject_oversized_request_hint(request.headers(), MAX_REQUEST_BODY_SIZE) {
        return json_rpc_error_response(StatusCode::PAYLOAD_TOO_LARGE, -32600, &e);
    }

    let headers = header_map_to_entries(request.headers());

    let mut req_body = request.into_body();
    let body = match req_body.str_contents().await {
        Ok(s) => {
            if let Err(e) = reject_oversized_body("Request body", s, MAX_REQUEST_BODY_SIZE) {
                return json_rpc_error_response(StatusCode::PAYLOAD_TOO_LARGE, -32600, &e);
            }
            s.to_string()
        }
        Err(e) => {
            return json_rpc_error_response(
                StatusCode::BAD_REQUEST,
                -32700,
                &format!("Failed to read request body: {e}"),
            )
        }
    };

    match crate::mcp::process_rpc(
        &server_id,
        &body,
        &headers,
        &env_config.wasmcloud_host,
        &env_config.application_id,
    )
    .await
    {
        Ok(Some(result)) => serialize_to_json_response(&result),
        Ok(None) => Response::builder()
            .status(StatusCode::NO_CONTENT)
            .body(Body::from(String::new()))
            .unwrap_or_else(|_| Response::new(Body::from("Internal Server Error"))),
        Err(error_response) => serialize_to_json_response(&error_response),
    }
}

fn validate_content_type(headers: &HeaderMap) -> Result<(), String> {
    validate_content_type_from_headers(&header_map_to_entries(headers))
}

fn validate_content_type_from_headers(headers: &[(String, Vec<u8>)]) -> Result<(), String> {
    headers
        .iter()
        .find(|(key, value)| {
            key.eq_ignore_ascii_case("content-type")
                && String::from_utf8_lossy(value)
                    .trim()
                    .starts_with("application/json")
        })
        .map(|_| ())
        .ok_or_else(|| "Content-Type must be application/json".to_string())
}

fn header_map_to_entries(headers: &HeaderMap) -> Vec<(String, Vec<u8>)> {
    headers
        .iter()
        .map(|(k, v)| (k.as_str().to_string(), v.as_bytes().to_vec()))
        .collect()
}

fn extract_server_id_from_path(path: &str) -> Result<String, String> {
    let remainder = path
        .strip_prefix(PATH_PREFIX)
        .ok_or_else(|| format!("Invalid path format. Expected {PATH_PREFIX}{{server-id}}"))?;

    let server_id = remainder.split(['/', '?']).next().unwrap_or("");
    if server_id.is_empty() {
        Err(format!(
            "Server ID cannot be empty. Expected {PATH_PREFIX}{{server-id}}"
        ))
    } else {
        Ok(server_id.to_string())
    }
}

/// Best-effort early rejection based on content-length header (may be absent or inaccurate).
fn reject_oversized_request_hint(headers: &HeaderMap, max_size: u64) -> Result<(), String> {
    if let Some(content_length) = headers
        .get("content-length")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok())
    {
        if content_length > max_size {
            return Err(format!(
                "Request body too large: {content_length} bytes exceeds {max_size} byte limit"
            ));
        }
    }
    Ok(())
}

/// Guaranteed size enforcement after reading the body.
pub(crate) fn reject_oversized_body(
    context: &str,
    body: &str,
    max_size: u64,
) -> Result<(), String> {
    if body.len() as u64 > max_size {
        return Err(format!(
            "{} too large: {} bytes exceeds {} byte limit",
            context,
            body.len(),
            max_size
        ));
    }
    Ok(())
}

fn serialize_to_json_response(value: &impl serde::Serialize) -> Response<Body> {
    match serde_json::to_string(value) {
        Ok(body_str) => json_response(StatusCode::OK, body_str),
        Err(_) => json_rpc_error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            -32603,
            "Internal server error",
        ),
    }
}

fn json_response(status: StatusCode, body: String) -> Response<Body> {
    Response::builder()
        .status(status)
        .header("content-type", "application/json")
        .body(Body::from(body))
        .unwrap_or_else(|_| Response::new(Body::from("Internal Server Error")))
}

fn json_rpc_error_response(status: StatusCode, code: i32, message: &str) -> Response<Body> {
    let error_body = json!({
        "jsonrpc": "2.0",
        "error": {
            "code": code,
            "message": message
        },
        "id": null
    })
    .to_string();
    json_response(status, error_body)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_server_id_valid() {
        let result = extract_server_id_from_path("/api/mcp/weather-server-001");
        assert_eq!(result.unwrap(), "weather-server-001");
    }

    #[test]
    fn test_extract_server_id_with_query_params() {
        let result = extract_server_id_from_path("/api/mcp/server-123?key=value");
        assert_eq!(result.unwrap(), "server-123");
    }

    #[test]
    fn test_extract_server_id_with_trailing_path() {
        let result = extract_server_id_from_path("/api/mcp/server-123/extra/path");
        assert_eq!(result.unwrap(), "server-123");
    }

    #[test]
    fn test_extract_server_id_empty() {
        let result = extract_server_id_from_path("/api/mcp/");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Server ID cannot be empty"));
    }

    #[test]
    fn test_extract_server_id_too_short() {
        let result = extract_server_id_from_path("/api/mcp");
        assert!(result.is_err());
    }

    #[test]
    fn test_extract_server_id_root_path() {
        let result = extract_server_id_from_path("/");
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_content_type_valid() {
        let headers = vec![("content-type".to_string(), b"application/json".to_vec())];
        assert!(validate_content_type_from_headers(&headers).is_ok());
    }

    #[test]
    fn test_validate_content_type_with_charset() {
        let headers = vec![(
            "Content-Type".to_string(),
            b"application/json; charset=utf-8".to_vec(),
        )];
        assert!(validate_content_type_from_headers(&headers).is_ok());
    }

    #[test]
    fn test_validate_content_type_case_insensitive_key() {
        let headers = vec![("CONTENT-TYPE".to_string(), b"application/json".to_vec())];
        assert!(validate_content_type_from_headers(&headers).is_ok());
    }

    #[test]
    fn test_validate_content_type_missing() {
        let headers: Vec<(String, Vec<u8>)> = vec![];
        let result = validate_content_type_from_headers(&headers);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .contains("Content-Type must be application/json"));
    }

    #[test]
    fn test_validate_content_type_wrong_type() {
        let headers = vec![("content-type".to_string(), b"text/plain".to_vec())];
        assert!(validate_content_type_from_headers(&headers).is_err());
    }

    #[test]
    fn test_validate_content_type_among_other_headers() {
        let headers = vec![
            ("authorization".to_string(), b"Bearer token".to_vec()),
            ("content-type".to_string(), b"application/json".to_vec()),
            ("accept".to_string(), b"*/*".to_vec()),
        ];
        assert!(validate_content_type_from_headers(&headers).is_ok());
    }
}
