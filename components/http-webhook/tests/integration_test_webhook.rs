//! Integration tests for the webhook component.
//!
//! Runs the webhook component in a wash host and sends real HTTP requests to
//! `/api/webhook/{webhook-name}`. A mock action server answers with what it received, so the
//! tests can check which action ran with which input and token.
//!
//! Fixtures are automatically built by build.rs before tests run.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use anyhow::{Context, Result};
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
const SEND_INVOICE_ACTION_ID: &str = "action-send-invoice";
const PING_ACTION_ID: &str = "action-ping";
const BETTY_CONFIG: &str = r#"{"config-1":{"encrypted":false,"value":"x"}}"#;

fn webhook_component_config() -> HashMap<String, String> {
    HashMap::from([
        (
            "webhooks".to_string(),
            json!({ "send-invoice": SEND_INVOICE_ACTION_ID, "ping": PING_ACTION_ID }).to_string(),
        ),
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

    let config = webhook_component_config();

    let req = WorkloadStartRequest {
        workload_id: Uuid::new_v4().to_string(),
        workload: Workload {
            namespace: "test".to_string(),
            name: "webhook-test-workload".to_string(),
            annotations: HashMap::new(),
            service: None,
            components: vec![component(
                "webhook",
                WEBHOOK_COMPONENT_WASM,
                config.clone(),
                HashMap::from([
                    (
                        "WASMCLOUD_HOST".to_string(),
                        format!("http://{mock_action_addr}"),
                    ),
                    ("APPLICATION_ID".to_string(), "test-app-id".to_string()),
                ]),
                vec![mock_action_addr.to_string()],
            )],
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
) -> Result<(u16, Value)> {
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
    let body = response.json().await.context("response is not JSON")?;
    Ok((status, body))
}

async fn post(
    addr: SocketAddr,
    webhook: &str,
    token: Option<&str>,
    body: &str,
) -> Result<(u16, Value)> {
    let path = format!("/api/webhook/{webhook}");
    send(addr, reqwest::Method::POST, &path, token, body).await
}

#[tokio::test]
async fn test_runs_the_action_of_the_webhook() -> Result<()> {
    let (_host, addr) = setup().await?;

    let (status, body) = post(addr, "send-invoice", None, r#"{"amount": 12}"#).await?;

    assert_eq!(status, 200, "body: {body}");
    assert_eq!(
        body,
        json!({
            "action_id": SEND_INVOICE_ACTION_ID,
            "input": r#"{"amount":12}"#,
            "configurations": BETTY_CONFIG,
            "jwt": ""
        })
    );
    Ok(())
}

#[tokio::test]
async fn test_does_not_forward_the_token_of_the_caller() -> Result<()> {
    let (_host, addr) = setup().await?;

    let (status, body) = post(addr, "ping", Some("token-of-the-caller"), "").await?;

    assert_eq!(status, 200, "body: {body}");
    assert_eq!(
        body,
        json!({
            "action_id": PING_ACTION_ID,
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

    let path = "/api/webhook/send-invoice";
    let (status, body) = send(addr, reqwest::Method::GET, path, None, "").await?;

    assert_eq!(status, 200, "body: {body}");
    assert_eq!(body["action_id"], SEND_INVOICE_ACTION_ID);
    assert_eq!(body["input"], "{}");
    Ok(())
}

#[tokio::test]
async fn test_request_errors() -> Result<()> {
    let (_host, addr) = setup().await?;

    let (status, body) = post(addr, "unknown", None, "{}").await?;
    assert_eq!(status, 404, "body: {body}");
    assert!(body["error"].as_str().unwrap().contains("'unknown'"));

    let (status, _) = send(addr, reqwest::Method::PUT, "/api/webhook/ping", None, "").await?;
    assert_eq!(status, 405);

    let (status, _) = send(addr, reqwest::Method::POST, "/api/other", None, "").await?;
    assert_eq!(status, 404);

    let (status, body) = post(addr, "send-invoice", None, "[1, 2]").await?;
    assert_eq!(status, 400, "body: {body}");
    Ok(())
}
