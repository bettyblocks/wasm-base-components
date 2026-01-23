use anyhow::Context;
use jaws_rs::Keys;
use reqwest::{Client, StatusCode};
use serde_json::json;
use serial_test::serial;
use std::collections::HashMap;
use std::{
    net::SocketAddr,
    sync::{Arc, Once},
};
use tokio::task;
use tonic::metadata::MetadataMap;
use tonic::{transport::Server, Request, Response};
use tracing::info;
use uuid::Uuid;
use wash_runtime::host::http::{DevRouter, HttpServer};
use wash_runtime::host::HostApi;
use wash_runtime::types::{Component, LocalResources, Workload, WorkloadStartRequest};
use wash_runtime::wit::WitInterface;
use wash_runtime::{
    engine::Engine,
    host::{Host, HostBuilder},
};
pub mod data_grpc {
    tonic::include_proto!("data_grpc");
}

use data_grpc::DataApiRequest;
use data_grpc::{
    data_api_result::Status,
    data_api_server::{DataApi, DataApiServer},
    DataApiResult,
};

#[path = "common/mod.rs"]
mod common;
use common::find_available_port;

const DATA_API_COMPONENT: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/target/wasm32-wasip2/release/deps/data_api_component.wasm"
));
const TEST_COMPONENT: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/test-component/target/wasm32-wasip2/release/deps/test_component.wasm"
));

const GRPC_HOST_HEADER: &str = "grpc";
const APPLICATION_ID: &str = "06caae5da8234837a330c14a7350ed75";
const JAWS_SECRET: &str = "SUPER_SECRET";
const JAWS_DEFAULT_ISSUER: &str = "actions-wasm";

static TRACING: Once = Once::new();

/// Inits tracing for tests
fn init_tracing_for_test() {
    TRACING.call_once(|| {
        tracing_subscriber::fmt()
            .with_env_filter(
                tracing_subscriber::EnvFilter::from_default_env()
                    .add_directive("wash_runtime=debug".parse().unwrap()),
            )
            .try_init()
            .expect("Failed initting tracing");
    })
}

pub async fn start_test_grpc_server(addr: SocketAddr) -> anyhow::Result<()> {
    let data_api_server = DataGrpcServer::default();
    Server::builder()
        .add_service(DataApiServer::new(data_api_server))
        .serve(addr)
        .await?;

    Ok(())
}

struct TestConfig {
    host: Arc<Host>,
    grpc_addr: SocketAddr,
    http_addr: SocketAddr,
}

impl TestConfig {
    fn new(host: Arc<Host>, grpc_addr: SocketAddr, http_addr: SocketAddr) -> Self {
        Self {
            host,
            grpc_addr,
            http_addr,
        }
    }
}

async fn setup_wasmcloud_host() -> anyhow::Result<TestConfig> {
    let grpc_port = find_available_port().await?;
    let grpc_addr: SocketAddr = format!("127.0.0.1:{grpc_port}").parse().unwrap();

    info!("Starting server");
    let _handle = task::spawn(start_test_grpc_server(grpc_addr));

    let http_port = find_available_port().await?;
    let http_addr: SocketAddr = format!("127.0.0.1:{http_port}").parse().unwrap();

    let engine = Engine::builder().build()?;

    let http_handler = DevRouter::default();
    let http_plugin = HttpServer::new(http_handler, http_addr);

    info!("Starting host");
    let host = HostBuilder::new()
        .with_engine(engine.clone())
        .with_http_handler(Arc::new(http_plugin))
        .with_grpc(HashMap::new())
        .build()?;

    let host = host.start().await?;

    Ok(TestConfig::new(host, grpc_addr, http_addr))
}

async fn start_workload(test_config: &TestConfig) -> anyhow::Result<String> {
    let config = HashMap::from([
        (
            "GRPC_SERVER_URI".to_string(),
            format!("http://{}", test_config.grpc_addr),
        ),
        ("JAWS_SECRET_KEY".to_string(), JAWS_SECRET.to_string()),
    ]);

    start_workload_with_config(test_config, config).await
}

async fn start_workload_with_config(
    test_config: &TestConfig,
    config: HashMap<String, String>,
) -> anyhow::Result<String> {
    // NOTE:
    // For both the components the environment variables are set
    // There is a context issue: https://github.com/wasmCloud/wash/issues/103
    // In short, when the DATA_API_COMPONENT is called it uses the TEST_COMPONENT context
    // Therefore, if you only set the environment for the DATA_API_COMPONENT, that will
    // not work
    let req = WorkloadStartRequest {
        workload_id: Uuid::new_v4().to_string(),
        workload: Workload {
            namespace: "test".to_string(),
            name: "grpc-hello-workload".to_string(),
            annotations: HashMap::new(),
            service: None,
            components: vec![
                Component {
                    bytes: bytes::Bytes::from_static(DATA_API_COMPONENT),
                    local_resources: LocalResources {
                        memory_limit_mb: 256,
                        cpu_limit: 1,
                        config: HashMap::new(),
                        environment: config.clone(),
                        volume_mounts: vec![],
                        allowed_hosts: vec![],
                    },
                    pool_size: 1,
                    max_invocations: 100,
                },
                Component {
                    bytes: bytes::Bytes::from_static(TEST_COMPONENT),
                    local_resources: LocalResources {
                        memory_limit_mb: 256,
                        cpu_limit: 1,
                        config: HashMap::new(),
                        environment: config.clone(),
                        volume_mounts: vec![],
                        allowed_hosts: vec![],
                    },
                    pool_size: 1,
                    max_invocations: 100,
                },
            ],
            host_interfaces: vec![
                WitInterface {
                    namespace: "wasi".to_string(),
                    package: "http".to_string(),
                    interfaces: ["incoming-handler".to_string()].into_iter().collect(),
                    version: Some(semver::Version::parse("0.2.2").unwrap()),
                    config: {
                        let mut config = HashMap::new();
                        config.insert("host".to_string(), GRPC_HOST_HEADER.to_string());
                        config
                    },
                },
                WitInterface {
                    namespace: "wasi".to_string(),
                    package: "http".to_string(),
                    interfaces: ["outgoing-handler".to_string()].into_iter().collect(),
                    version: Some(semver::Version::parse("0.2.4").unwrap()),
                    config: HashMap::new(),
                },
            ],
            volumes: vec![],
        },
    };

    info!("Starting workload");
    let request = test_config
        .host
        .workload_start(req)
        .await
        .context("Failed to start workload")?;

    Ok(request.workload_status.workload_id)
}

#[tokio::test]
#[serial]
async fn data_api_component_should_return_data_rpc_server_error() -> anyhow::Result<()> {
    init_tracing_for_test();

    let test_config = setup_wasmcloud_host().await?;
    let id = start_workload(&test_config).await?;

    let client = Client::new();
    info!("Performing request");
    let response = client
        .post(format!("http://{}", test_config.http_addr))
        .json(&json!(
        {"query": "",
        "variables":"",
        "context": {"application_id":APPLICATION_ID.to_string(), }}
        ))
        .header("HOST", GRPC_HOST_HEADER)
        .send()
        .await?;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    let result: serde_json::Value = response.json().await?;

    let expected = json!({
        "errors": [
            { "message": "something went wrong" }
        ]
    });

    assert_eq!(expected, result);

    let _ = &test_config
        .host
        .workload_stop(wash_runtime::types::WorkloadStopRequest { workload_id: id })
        .await?;

    Ok(())
}

#[tokio::test]
#[serial]
async fn data_api_component_should_return_error_if_token_is_invalid() -> anyhow::Result<()> {
    init_tracing_for_test();
    let test_config = setup_wasmcloud_host().await?;

    let config = HashMap::from([
        (
            "GRPC_SERVER_URI".to_string(),
            format!("http://{}", test_config.grpc_addr),
        ),
        ("JAWS_SECRET_KEY".to_string(), "WRONG_SECRET".to_string()),
    ]);

    let id = start_workload_with_config(&test_config, config).await?;
    let client = Client::new();

    info!("Performing request");
    let response = client
        .post(format!("http://{}", test_config.http_addr))
        .json(&json!(
        {"query": "",
        "variables":"",
        "context": {"application_id":APPLICATION_ID.to_string(), }}
        ))
        .header("host", GRPC_HOST_HEADER)
        .send()
        .await?;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    let result: serde_json::Value = response.json().await?;

    let expected = json!({
        "errors": [
            {
                "extensions": {"code": "UNAUTHENTICATED" },
                "message": "Request not authenticated" },
        ]
    });
    assert_eq!(expected, result);

    let _ = &test_config
        .host
        .workload_stop(wash_runtime::types::WorkloadStopRequest { workload_id: id })
        .await?;
    Ok(())
}

/// REALLY SIMPLE MOCK OF THE DATA API
#[derive(Debug, Default)]
pub struct DataGrpcServer {}

#[tonic::async_trait]
impl DataApi for DataGrpcServer {
    async fn execute(
        &self,
        request: Request<DataApiRequest>,
    ) -> Result<Response<DataApiResult>, tonic::Status> {
        let (metadata, _extensions, data_api_request) = request.into_parts();

        let application_id = match data_api_request.context {
            Some(ctx) => ctx.application_id,
            None => {
                let body = format_error_unauthenticated();
                let reply = DataApiResult {
                    status: Status::Error as i32,
                    result: body,
                };
                return Ok(Response::new(reply));
            }
        };

        match authenticate(&metadata, &application_id).await {
            AuthResult::Ok => {
                if application_id == APPLICATION_ID {
                    let body = format_error_json(serde_json::json!({
                        "message": "something went wrong"
                    }));

                    let reply = DataApiResult {
                        status: Status::Error as i32,
                        result: body,
                    };

                    Ok(Response::new(reply))
                } else {
                    let body = format_result_json(serde_json::json!({}));

                    let reply = DataApiResult {
                        status: Status::Ok as i32,
                        result: body,
                    };

                    Ok(Response::new(reply))
                }
            }
            AuthResult::Unauthenticated => {
                let body = format_error_unauthenticated();

                let reply = DataApiResult {
                    status: Status::Error as i32,
                    result: body,
                };

                Ok(Response::new(reply))
            }
        }
    }
}

enum AuthResult {
    Ok,
    Unauthenticated,
}

async fn authenticate(metadata: &MetadataMap, application_id: &str) -> AuthResult {
    let token = metadata
        .get("authorization")
        .and_then(|val| val.to_str().ok())
        .unwrap_or("")
        .to_string();

    let token = token
        .strip_prefix("Bearer ")
        .unwrap_or(token.as_str())
        .to_string();

    let mut keys = Keys::new();
    keys.insert(JAWS_DEFAULT_ISSUER.to_string(), JAWS_SECRET.to_string());
    match jaws_rs::decode_and_validate_jwt(&token, &keys) {
        Ok(claims) if claims.application_id == application_id => AuthResult::Ok,
        _ => AuthResult::Unauthenticated,
    }
}

fn format_result_json(value: serde_json::Value) -> String {
    serde_json::to_string(&serde_json::json!({ "data": value }))
        .expect("JSON serialization should not fail")
}

fn format_error_unauthenticated() -> String {
    format_error_json(serde_json::json!({
        "message": "Request not authenticated",
        "extensions": { "code": "UNAUTHENTICATED" }
    }))
}

fn format_error_json(error: serde_json::Value) -> String {
    let errors = match error {
        serde_json::Value::Array(arr) => arr,
        other => vec![other],
    };

    serde_json::to_string(&serde_json::json!({ "errors": errors }))
        .expect("JSON serialization should not fail")
}
