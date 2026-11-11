//! Integration tests for the webhook component.
//!
//! Runs the webhook, auth and data-api components in a wash host and sends real HTTP requests
//! to `/api/webhook/{webhook-name}`. A mock action server answers with what it received, so the
//! tests can check which action ran with which input and token.
//!
//! Fixtures are automatically built by build.rs before tests run.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use anyhow::{Context, Result};
use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
use serde::Serialize;
use serde_json::{json, Value};
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

const WEBHOOK_COMPONENT_WASM: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/webhook_component.wasm"
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
const OTHER_SECRET: &str = "other-hs256-secret-key-for-integration-tests";
const OTHER_PROFILE_ID: &str = "other-profile-001";
const PRIVATE_ACTION_ID: &str = "action-send-invoice";
const PUBLIC_ACTION_ID: &str = "action-ping";
const BETTY_CONFIG: &str = r#"{"config-1":{"encrypted":false,"value":"x"}}"#;

#[derive(Serialize)]
struct JwtClaims {
    auth_profile_id: String,
    exp: u64,
    nbf: u64,
    iat: u64,
}

fn make_token(secret: &str, profile_id: &str) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let claims = JwtClaims {
        auth_profile_id: profile_id.to_string(),
        exp: now + 3600,
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

/// The `actions` key, which the webhook and the auth component both read.
fn actions_config() -> String {
    json!({
        PRIVATE_ACTION_ID: { "authentication-profile-id": TEST_PROFILE_ID, "kind": "api-key" },
        PUBLIC_ACTION_ID: { "authentication-profile-id": null }
    })
    .to_string()
}

fn auth_component_config() -> HashMap<String, String> {
    HashMap::from([
        (
            "authentication_profiles".to_string(),
            json!({ TEST_PROFILE_ID: TEST_SECRET, OTHER_PROFILE_ID: OTHER_SECRET }).to_string(),
        ),
        ("actions".to_string(), actions_config()),
    ])
}

fn webhook_component_config() -> HashMap<String, String> {
    HashMap::from([
        (
            "webhooks".to_string(),
            json!({ "send-invoice": PRIVATE_ACTION_ID, "ping": PUBLIC_ACTION_ID }).to_string(),
        ),
        ("actions".to_string(), actions_config()),
        ("betty_config".to_string(), BETTY_CONFIG.to_string()),
    ])
}

/// Answers every action call with the action id, input, configurations and token it received.
async fn start_mock_action_server() -> Result<SocketAddr> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;

    let app = axum::Router::new().route(
        "/",
        axum::routing::post(|axum::Json(body): axum::Json<Value>| async move {
            axum::Json(json!({
                "output": {
                    "action_id": body["action_id"],
                    "input": body["payload"]["input"],
                    "configurations": body["payload"]["configurations"],
                    "jwt": body["jwt"]
                }
            }))
        }),
    );

    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    Ok(addr)
}

fn component(
    name: &str,
    bytes: &'static [u8],
    config: HashMap<String, String>,
    environment: HashMap<String, String>,
    allowed_hosts: Vec<String>,
) -> Component {
    Component {
        name: name.to_string(),
        bytes: bytes::Bytes::from_static(bytes),
        digest: None,
        local_resources: LocalResources {
            memory_limit_mb: 256,
            cpu_limit: 1,
            config,
            environment,
            volume_mounts: vec![],
            allowed_hosts: allowed_hosts.into(),
        },
        pool_size: 1,
        max_invocations: 100,
    }
}

fn host_interface(
    package: &str,
    interfaces: &[&str],
    version: &str,
    config: HashMap<String, String>,
) -> WitInterface {
    WitInterface {
        namespace: "wasi".to_string(),
        package: package.to_string(),
        interfaces: interfaces.iter().map(|i| i.to_string()).collect(),
        version: Some(semver::Version::parse(version).unwrap()),
        config,
        name: None,
    }
}

async fn setup() -> Result<(Arc<Host>, SocketAddr)> {
    let port = find_available_port().await?;
    let addr: SocketAddr = format!("127.0.0.1:{port}").parse().unwrap();
    let mock_action_addr = start_mock_action_server().await?;

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

    let mut config = webhook_component_config();
    config.extend(auth_component_config());

    let req = WorkloadStartRequest {
        workload_id: Uuid::new_v4().to_string(),
        workload: Workload {
            namespace: "test".to_string(),
            name: "webhook-test-workload".to_string(),
            annotations: HashMap::new(),
            service: None,
            components: vec![
                component(
                    "webhook",
                    WEBHOOK_COMPONENT_WASM,
                    webhook_component_config(),
                    HashMap::from([
                        (
                            "WASMCLOUD_HOST".to_string(),
                            format!("http://{mock_action_addr}"),
                        ),
                        ("APPLICATION_ID".to_string(), "test-app-id".to_string()),
                    ]),
                    vec![mock_action_addr.to_string()],
                ),
                component(
                    "jwt-auth",
                    JWT_AUTH_COMPONENT_WASM,
                    auth_component_config(),
                    HashMap::new(),
                    vec![],
                ),
                // the auth component imports the validator of the data-api, these tests never call it
                component(
                    "data-api",
                    DATA_API_COMPONENT_WASM,
                    HashMap::new(),
                    HashMap::new(),
                    vec![],
                ),
            ],
            host_interfaces: vec![
                host_interface(
                    "http",
                    &["incoming-handler", "outgoing-handler"],
                    "0.2.2",
                    HashMap::from([("host".to_string(), addr.to_string())]),
                ),
                host_interface("config", &["store"], "0.2.0-rc.1", config),
                host_interface("cli", &["environment"], "0.2.2", HashMap::new()),
            ],
            volumes: vec![],
        },
    };

    host.workload_start(req)
        .await
        .context("failed to start webhook workload")?;

    Ok((host, addr))
}

async fn send(
    addr: SocketAddr,
    method: reqwest::Method,
    path: &str,
    token: Option<&str>,
    body: &str,
) -> Result<(u16, reqwest::header::HeaderMap, Value)> {
    let mut req = reqwest::Client::new()
        .request(method, format!("http://{addr}{path}"))
        .header("Content-Type", "application/json")
        .body(body.to_string());
    if let Some(t) = token {
        req = req.header("Authorization", format!("Bearer {t}"));
    }
    let response = timeout(Duration::from_secs(10), req.send())
        .await
        .context("request timed out")?
        .context("request failed")?;
    let status = response.status().as_u16();
    let headers = response.headers().clone();
    let body = response.json().await.context("response is not JSON")?;
    Ok((status, headers, body))
}

async fn post(
    addr: SocketAddr,
    webhook: &str,
    token: Option<&str>,
    body: &str,
) -> Result<(u16, reqwest::header::HeaderMap, Value)> {
    let path = format!("/api/webhook/{webhook}");
    send(addr, reqwest::Method::POST, &path, token, body).await
}

#[tokio::test]
async fn test_runs_the_action_of_the_webhook() -> Result<()> {
    let (_host, addr) = setup().await?;
    let token = make_token(TEST_SECRET, TEST_PROFILE_ID);

    let (status, _, body) = post(addr, "send-invoice", Some(&token), r#"{"amount": 12}"#).await?;

    assert_eq!(status, 200, "body: {body}");
    assert_eq!(
        body,
        json!({
            "action_id": PRIVATE_ACTION_ID,
            "input": r#"{"amount":12}"#,
            "configurations": BETTY_CONFIG,
            "jwt": token
        })
    );
    Ok(())
}

#[tokio::test]
async fn test_public_webhook_runs_without_the_token_of_the_caller() -> Result<()> {
    let (_host, addr) = setup().await?;
    let token = make_token(TEST_SECRET, TEST_PROFILE_ID);

    let (status, _, body) = post(addr, "ping", Some(&token), "").await?;

    assert_eq!(status, 200, "body: {body}");
    assert_eq!(
        body,
        json!({
            "action_id": PUBLIC_ACTION_ID,
            "input": "{}",
            "configurations": BETTY_CONFIG,
            "jwt": ""
        })
    );
    Ok(())
}

#[tokio::test]
async fn test_get_runs_the_action_with_empty_input() -> Result<()> {
    let (_host, addr) = setup().await?;
    let token = make_token(TEST_SECRET, TEST_PROFILE_ID);

    let path = "/api/webhook/send-invoice";
    let (status, _, body) = send(addr, reqwest::Method::GET, path, Some(&token), "").await?;

    assert_eq!(status, 200, "body: {body}");
    assert_eq!(body["action_id"], PRIVATE_ACTION_ID);
    assert_eq!(body["input"], "{}");

    let (status, _, body) = send(addr, reqwest::Method::GET, path, None, "").await?;
    assert_eq!(status, 401, "body: {body}");
    Ok(())
}

#[tokio::test]
async fn test_auth_failures() -> Result<()> {
    let (_host, addr) = setup().await?;

    let (status, headers, body) = post(addr, "send-invoice", None, "{}").await?;
    assert_eq!(status, 401, "body: {body}");
    assert_eq!(headers["www-authenticate"], "Bearer");

    let bad_signature = make_token(OTHER_SECRET, TEST_PROFILE_ID);
    let (status, headers, body) = post(addr, "send-invoice", Some(&bad_signature), "{}").await?;
    assert_eq!(status, 401, "body: {body}");
    assert_eq!(headers["www-authenticate"], "Bearer");

    let valid_token_for_other_profile = make_token(OTHER_SECRET, OTHER_PROFILE_ID);
    let (status, headers, body) = post(
        addr,
        "send-invoice",
        Some(&valid_token_for_other_profile),
        "{}",
    )
    .await?;
    assert_eq!(status, 403, "body: {body}");
    assert!(!headers.contains_key("www-authenticate"));
    Ok(())
}

#[tokio::test]
async fn test_request_errors() -> Result<()> {
    let (_host, addr) = setup().await?;
    let token = make_token(TEST_SECRET, TEST_PROFILE_ID);

    let (status, _, body) = post(addr, "unknown", Some(&token), "{}").await?;
    assert_eq!(status, 404, "body: {body}");
    assert!(body["error"].as_str().unwrap().contains("'unknown'"));

    let (status, _, _) = send(addr, reqwest::Method::PUT, "/api/webhook/ping", None, "").await?;
    assert_eq!(status, 405);

    let (status, _, _) = send(addr, reqwest::Method::POST, "/api/other", None, "").await?;
    assert_eq!(status, 404);

    let (status, _, body) = post(addr, "send-invoice", Some(&token), "[1, 2]").await?;
    assert_eq!(status, 400, "body: {body}");
    Ok(())
}
