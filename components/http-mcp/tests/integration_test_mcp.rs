//! Integration tests for the MCP component.
//!
//! Tests the full flow:
//! 1. JWT token validation via jwt-auth-component (HS256)
//! 2. Config store loading for MCP server and auth configuration
//! 3. JSON-RPC request handling (initialize, tools/list, tools/call)
//! 4. Action execution via HTTP request to a mock action server
//!
//! Fixtures are automatically built by build.rs before tests run.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use anyhow::{Context, Result};
use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
use serde::Serialize;
use serde_json::json;
use std::{collections::HashMap, net::SocketAddr, sync::Arc};
use tokio::time::{timeout, Duration};
use uuid::Uuid;
use wash_runtime::{
    engine::Engine,
    host::{
        http::{DevRouter, HttpServer},
        Host, HostApi, HostBuilder,
    },
    plugin::wasi_config::DynamicConfig,
    types::{Component, LocalResources, Workload, WorkloadStartRequest},
    wit::WitInterface,
};

mod common;
use common::find_available_port;

const BETTY_MCP_COMPONENT_WASM: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/betty_mcp_component.wasm"
));
const JWT_AUTH_COMPONENT_WASM: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/jwt_auth_component.wasm"
));

const DATA_API_COMPONENT_WASM: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/data_api_component.wasm"
));

const TEST_SECRET: &str = "test-hs256-secret-key-for-integration-tests";
const TEST_PROFILE_ID: &str = "test-profile-001";
const TEST_ACTION_ID: &str = "action-weather-get";
const TEST_SERVER_ID: &str = "weather-server-001";
/// an mcp server whose auth profile is validated by the data-api
const USER_SERVER_ID: &str = "user-server-001";
const USER_ACTION_ID: &str = "action-user-get";
const USER_PROFILE_ID: &str = "user-profile-001";
const RESOURCE_METADATA_URL: &str = "https://test.betty.app/.well-known/oauth-protected-resource/api/runtime/test-app-id/authentication_profile/user-profile-001";
/// the mock data-api accepts these tokens, anything else is rejected as expired
const USER_TOKEN: &str = "user-token-of-user-profile";
const OTHER_PROFILE_USER_TOKEN: &str = "user-token-of-other-profile";
const WRONG_SECRET: &str = "wrong-secret-that-will-fail-hs256-validation";

#[derive(Serialize)]
struct JwtClaims {
    auth_profile_id: String,
    exp: u64,
    nbf: u64,
    iat: u64,
}

fn make_token(secret: &str, profile_id: &str, exp_offset_secs: i64) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let claims = JwtClaims {
        auth_profile_id: profile_id.to_string(),
        exp: (now as i64 + exp_offset_secs) as u64,
        nbf: now,
        iat: now,
    };
    encode(
        &Header::new(Algorithm::HS256),
        &claims,
        &EncodingKey::from_secret(secret.as_bytes()),
    )
    .expect("failed to encode JWT")
}

fn valid_token() -> String {
    make_token(TEST_SECRET, TEST_PROFILE_ID, 3600)
}

fn auth_component_config() -> HashMap<String, String> {
    HashMap::from([
        (
            "authentication_profiles".to_string(),
            json!({
                TEST_PROFILE_ID: TEST_SECRET
            })
            .to_string(),
        ),
        (
            "actions".to_string(),
            json!({
                TEST_ACTION_ID: { "authentication-profile-id": TEST_PROFILE_ID, "kind": "api-key" },
                USER_ACTION_ID: { "authentication-profile-id": USER_PROFILE_ID, "kind": "data-api" }
            })
            .to_string(),
        ),
        (
            "mcps".to_string(),
            json!({
                TEST_SERVER_ID: { "authentication-profile-id": TEST_PROFILE_ID, "kind": "api-key" },
                USER_SERVER_ID: {
                    "authentication-profile-id": USER_PROFILE_ID,
                    "kind": "data-api",
                    "resource-metadata-url": RESOURCE_METADATA_URL
                }
            })
            .to_string(),
        ),
    ])
}

fn mcp_component_config() -> HashMap<String, String> {
    HashMap::from([
        (
            "mcp_servers".to_string(),
            json!({
                "actions": {
                    TEST_ACTION_ID: { "authentication-profile-id": TEST_PROFILE_ID, "kind": "api-key" },
                    USER_ACTION_ID: { "authentication-profile-id": USER_PROFILE_ID, "kind": "data-api" }
                },
                "mcp-servers": [
                    {
                        "id": TEST_SERVER_ID,
                        "tools": [
                            {
                                "action-id": TEST_ACTION_ID,
                                "name": "get_weather",
                                "description": "Gets weather for a location",
                                "inputSchema": {
                                    "type": "object",
                                    "properties": {
                                        "location": { "type": "string" }
                                    },
                                    "required": ["location"]
                                }
                            }
                        ]
                    },
                    {
                        "id": USER_SERVER_ID,
                        "tools": [
                            {
                                "action-id": USER_ACTION_ID,
                                "name": "get_user",
                                "description": "Gets the current user",
                                "inputSchema": { "type": "object", "properties": {} }
                            }
                        ]
                    }
                ],
                "mcps": {
                    TEST_SERVER_ID: { "authentication-profile-id": TEST_PROFILE_ID, "kind": "api-key" },
                    USER_SERVER_ID: {
                        "authentication-profile-id": USER_PROFILE_ID,
                        "kind": "data-api",
                        "resource-metadata-url": RESOURCE_METADATA_URL
                    }
                }
            })
            .to_string(),
        ),
        (
            "meta_info".to_string(),
            json!({
                "protocolVersion": "2024-11-05",
                "capabilities": { "tools": {} },
                "serverInfo": {
                    "name": "test-mcp-server",
                    "version": "1.0.0"
                }
            })
            .to_string(),
        ),
    ])
}

async fn start_mock_action_server() -> Result<SocketAddr> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;

    let app = axum::Router::new().route(
        "/",
        axum::routing::post(|| async {
            axum::Json(json!({"output": "mock action executed successfully"}))
        }),
    );

    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    Ok(addr)
}

/// A protobuf `ValidateJWTResult`, framed as a unary gRPC message.
fn grpc_validate_jwt_result(ok: bool, result: &str) -> Vec<u8> {
    assert!(result.len() < 128, "the length is encoded as a single byte");

    let mut message = Vec::new();
    if !ok {
        message.extend([0x08, 0x01]); // status = ERROR
    }
    message.extend([0x12, result.len() as u8]); // result
    message.extend(result.as_bytes());

    let mut frame = vec![0];
    frame.extend((message.len() as u32).to_be_bytes());
    frame.extend(message);
    frame
}

fn contains(haystack: &[u8], needle: &str) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle.as_bytes())
}

/// Plays the data-api: knows the tokens of two users, rejects every other token.
async fn start_mock_data_api() -> Result<SocketAddr> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;

    let app = axum::Router::new().route(
        "/data_grpc.DataAPI/ValidateJWT",
        axum::routing::post(|body: bytes::Bytes| async move {
            let frame = if contains(&body, OTHER_PROFILE_USER_TOKEN) {
                grpc_validate_jwt_result(true, r#"{"user_id":2,"auth_profile":"other-profile"}"#)
            } else if contains(&body, USER_TOKEN) {
                grpc_validate_jwt_result(
                    true,
                    &json!({"user_id": 1, "auth_profile": USER_PROFILE_ID}).to_string(),
                )
            } else {
                grpc_validate_jwt_result(false, r#"{"code":"EXPIRED","message":"Token expired"}"#)
            };

            (
                [(axum::http::header::CONTENT_TYPE, "application/grpc")],
                frame,
            )
        }),
    );

    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    Ok(addr)
}

async fn setup() -> Result<(Arc<Host>, SocketAddr)> {
    let port = find_available_port().await?;
    let addr: SocketAddr = format!("127.0.0.1:{port}").parse().unwrap();

    let mock_action_addr = start_mock_action_server().await?;
    let mock_data_api_addr = start_mock_data_api().await?;

    let engine = Engine::builder().build()?;
    let http_plugin = HttpServer::new(DevRouter::default(), addr).await?;
    let host = HostBuilder::new()
        .with_engine(engine)
        .with_http_handler(Arc::new(http_plugin))
        .with_plugin(Arc::new(DynamicConfig::default()))?
        .build()?
        .start()
        .await
        .context("failed to start host")?;

    let req = WorkloadStartRequest {
        workload_id: Uuid::new_v4().to_string(),
        workload: Workload {
            namespace: "test".to_string(),
            name: "mcp-test-workload".to_string(),
            annotations: HashMap::new(),
            service: None,
            components: vec![
                Component {
                    name: "betty-mcp".to_string(),
                    bytes: bytes::Bytes::from_static(BETTY_MCP_COMPONENT_WASM),
                    digest: None,
                    local_resources: LocalResources {
                        memory_limit_mb: 256,
                        cpu_limit: 1,
                        config: mcp_component_config(),
                        environment: HashMap::from([
                            (
                                "WASMCLOUD_HOST".to_string(),
                                format!("http://{mock_action_addr}"),
                            ),
                            ("APPLICATION_ID".to_string(), "test-app-id".to_string()),
                        ]),
                        volume_mounts: vec![],
                        allowed_hosts: vec![mock_action_addr.to_string()].into(),
                    },
                    pool_size: 1,
                    max_invocations: 100,
                },
                Component {
                    name: "jwt-auth".to_string(),
                    bytes: bytes::Bytes::from_static(JWT_AUTH_COMPONENT_WASM),
                    digest: None,
                    local_resources: LocalResources {
                        memory_limit_mb: 128,
                        cpu_limit: 1,
                        config: auth_component_config(),
                        environment: HashMap::new(),
                        volume_mounts: vec![],
                        allowed_hosts: vec![].into(),
                    },
                    pool_size: 1,
                    max_invocations: 100,
                },
                Component {
                    name: "data-api".to_string(),
                    bytes: bytes::Bytes::from_static(DATA_API_COMPONENT_WASM),
                    digest: None,
                    local_resources: LocalResources {
                        memory_limit_mb: 128,
                        cpu_limit: 1,
                        config: HashMap::new(),
                        environment: HashMap::from([
                            (
                                "GRPC_SERVER_URI".to_string(),
                                format!("http://{mock_data_api_addr}"),
                            ),
                            ("JAWS_SECRET_KEY".to_string(), "jaws-secret".to_string()),
                        ]),
                        volume_mounts: vec![],
                        allowed_hosts: vec![mock_data_api_addr.to_string()].into(),
                    },
                    pool_size: 1,
                    max_invocations: 100,
                },
            ],
            host_interfaces: vec![
                WitInterface {
                    namespace: "wasi".to_string(),
                    package: "http".to_string(),
                    interfaces: [
                        "incoming-handler".to_string(),
                        "outgoing-handler".to_string(),
                    ]
                    .into_iter()
                    .collect(),
                    version: Some(semver::Version::parse("0.2.2").unwrap()),
                    config: HashMap::from([("host".to_string(), addr.to_string())]),
                    name: None,
                },
                WitInterface {
                    namespace: "wasi".to_string(),
                    package: "config".to_string(),
                    interfaces: ["store".to_string()].into_iter().collect(),
                    version: Some(semver::Version::parse("0.2.0-rc.1").unwrap()),
                    config: {
                        let mut config = mcp_component_config();
                        config.extend(auth_component_config());
                        config
                    },
                    name: None,
                },
                WitInterface {
                    namespace: "wasi".to_string(),
                    package: "cli".to_string(),
                    interfaces: ["environment".to_string()].into_iter().collect(),
                    version: Some(semver::Version::parse("0.2.2").unwrap()),
                    config: HashMap::new(),
                    name: None,
                },
            ],
            volumes: vec![],
        },
    };

    host.workload_start(req)
        .await
        .context("failed to start MCP workload")?;

    Ok((host, addr))
}

fn client() -> reqwest::Client {
    reqwest::Client::new()
}

async fn rpc_response(
    addr: SocketAddr,
    server_id: &str,
    token: Option<&str>,
    body: serde_json::Value,
) -> Result<reqwest::Response> {
    let mut req = client()
        .post(format!("http://{addr}/api/mcp/{server_id}"))
        .header("Content-Type", "application/json");
    if let Some(t) = token {
        req = req.header("Authorization", format!("Bearer {t}"));
    }
    timeout(Duration::from_secs(10), req.json(&body).send())
        .await
        .context("request timed out")?
        .context("request failed")
}

async fn rpc(
    addr: SocketAddr,
    token: Option<&str>,
    body: serde_json::Value,
) -> Result<serde_json::Value> {
    rpc_response(addr, TEST_SERVER_ID, token, body)
        .await?
        .json()
        .await
        .context("failed to parse response as JSON")
}

#[tokio::test]
async fn test_happy_path() -> Result<()> {
    let (_host, addr) = setup().await?;
    let token = valid_token();

    let body = rpc(
        addr,
        Some(&token),
        json!({
            "jsonrpc": "2.0",
            "method": "initialize",
            "params": { "protocolVersion": "2024-11-05", "capabilities": {} },
            "id": 1
        }),
    )
    .await?;
    assert_eq!(body["jsonrpc"], "2.0");
    assert!(body["result"]["serverInfo"].is_object(), "body: {body}");

    let body = rpc(
        addr,
        Some(&token),
        json!({"jsonrpc": "2.0", "method": "tools/list", "params": {}, "id": 2}),
    )
    .await?;
    let tools = body["result"]["tools"]
        .as_array()
        .expect("tools must be array");
    assert!(!tools.is_empty(), "body: {body}");
    let names: Vec<&str> = tools.iter().filter_map(|t| t["name"].as_str()).collect();
    assert!(names.contains(&"get_weather"), "body: {body}");

    let body = rpc(
        addr,
        Some(&token),
        json!({
            "jsonrpc": "2.0",
            "method": "tools/call",
            "params": { "name": "get_weather", "arguments": { "location": "Amsterdam, NH" } },
            "id": 3
        }),
    )
    .await?;
    assert!(body["result"]["content"].is_array(), "body: {body}");
    assert_ne!(body["result"]["isError"], true, "body: {body}");

    Ok(())
}

#[tokio::test]
async fn test_auth_failures() -> Result<()> {
    let (_host, addr) = setup().await?;

    let list_req = || json!({"jsonrpc": "2.0", "method": "tools/list", "params": {}, "id": 1});

    let body = rpc(
        addr,
        None,
        json!({"jsonrpc": "2.0", "method": "initialize", "params": {}, "id": 1}),
    )
    .await?;
    assert!(
        body["error"].is_object(),
        "expected error for initialize without token, body: {body}"
    );

    let body = rpc(addr, None, list_req()).await?;
    assert!(
        body["error"].is_object(),
        "expected error for missing token, body: {body}"
    );

    let wrong_token = make_token(WRONG_SECRET, TEST_PROFILE_ID, 3600);
    let body = rpc(addr, Some(&wrong_token), list_req()).await?;
    assert!(
        body["error"].is_object(),
        "expected error for wrong secret, body: {body}"
    );

    let expired_token = make_token(TEST_SECRET, TEST_PROFILE_ID, -3600);
    let body = rpc(addr, Some(&expired_token), list_req()).await?;
    assert!(
        body["error"].is_object(),
        "expected error for expired token, body: {body}"
    );

    Ok(())
}

#[tokio::test]
async fn test_error_handling() -> Result<()> {
    let (_host, addr) = setup().await?;

    let body = rpc(addr, None, json!({"not_jsonrpc": "invalid"})).await?;
    assert!(body["error"].is_object(), "body: {body}");

    let body = rpc(
        addr,
        Some(&valid_token()),
        json!({"jsonrpc": "2.0", "method": "unknown/method", "params": {}, "id": 1}),
    )
    .await?;
    assert_eq!(body["error"]["code"], -32601, "body: {body}");
    assert!(
        body["error"]["message"]
            .as_str()
            .unwrap_or("")
            .contains("Method not found"),
        "body: {body}"
    );

    let response = timeout(
        Duration::from_secs(10),
        client()
            .post(format!("http://{addr}/api/mcp/nonexistent-server-999"))
            .header("Content-Type", "application/json")
            .json(&json!({"jsonrpc": "2.0", "method": "initialize", "params": {}, "id": 1}))
            .send(),
    )
    .await
    .context("timed out")?
    .context("failed")?;
    let body: serde_json::Value = response.json().await?;
    assert!(body["error"].is_object(), "body: {body}");

    let status = timeout(
        Duration::from_secs(10),
        client()
            .get(format!("http://{addr}/api/mcp/{TEST_SERVER_ID}"))
            .send(),
    )
    .await
    .context("timed out")?
    .context("failed")?
    .status();
    assert_eq!(status.as_u16(), 405);

    Ok(())
}

fn list_req() -> serde_json::Value {
    json!({"jsonrpc": "2.0", "method": "tools/list", "params": {}, "id": 1})
}

#[tokio::test]
async fn test_api_key_without_token_is_unauthorized() -> Result<()> {
    let (_host, addr) = setup().await?;

    let response = rpc_response(addr, TEST_SERVER_ID, None, list_req()).await?;

    assert_eq!(response.status().as_u16(), 401);
    assert_eq!(response.headers()["www-authenticate"], "Bearer");
    let body: serde_json::Value = response.json().await?;
    assert!(body["error"].is_object(), "body: {body}");

    Ok(())
}

#[tokio::test]
async fn test_data_api_without_token_points_to_the_resource_metadata() -> Result<()> {
    let (_host, addr) = setup().await?;

    let response = rpc_response(addr, USER_SERVER_ID, None, list_req()).await?;

    assert_eq!(response.status().as_u16(), 401);
    assert_eq!(
        response.headers()["www-authenticate"],
        format!("Bearer resource_metadata=\"{RESOURCE_METADATA_URL}\"").as_str()
    );

    Ok(())
}

#[tokio::test]
async fn test_data_api_rejected_token_is_unauthorized() -> Result<()> {
    let (_host, addr) = setup().await?;

    let response = rpc_response(addr, USER_SERVER_ID, Some("expired-token"), list_req()).await?;

    assert_eq!(response.status().as_u16(), 401);
    assert!(response.headers().contains_key("www-authenticate"));
    let body: serde_json::Value = response.json().await?;
    assert!(
        body["error"]["message"]
            .as_str()
            .unwrap_or("")
            .contains("Token expired"),
        "body: {body}"
    );

    Ok(())
}

#[tokio::test]
async fn test_data_api_token_of_another_profile_is_forbidden() -> Result<()> {
    let (_host, addr) = setup().await?;

    let response = rpc_response(
        addr,
        USER_SERVER_ID,
        Some(OTHER_PROFILE_USER_TOKEN),
        list_req(),
    )
    .await?;

    assert_eq!(response.status().as_u16(), 403);
    assert!(!response.headers().contains_key("www-authenticate"));

    Ok(())
}

#[tokio::test]
async fn test_data_api_valid_token() -> Result<()> {
    let (_host, addr) = setup().await?;

    let response = rpc_response(addr, USER_SERVER_ID, Some(USER_TOKEN), list_req()).await?;

    assert_eq!(response.status().as_u16(), 200);
    let body: serde_json::Value = response.json().await?;
    let names: Vec<&str> = body["result"]["tools"]
        .as_array()
        .expect("tools must be array")
        .iter()
        .filter_map(|t| t["name"].as_str())
        .collect();
    assert_eq!(names, ["get_user"], "body: {body}");

    Ok(())
}
