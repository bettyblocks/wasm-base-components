use anyhow::Context;
use axum::{
    Router,
    body::Bytes,
    extract::State,
    http::{HeaderMap, StatusCode, Uri},
    routing::post,
};
use reqwest::Client;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex, Once};
use tokio::task;
use tokio::time::{Duration, Instant, sleep};
use tracing::info;
use wash_runtime::host::http::{DevRouter, HttpServer};
use wash_runtime::host::{Host, HostBuilder};
use wash_runtime::types::{Component, LocalResources, Workload, WorkloadStartRequest};
use wash_runtime::wit::WitInterface;
use wash_runtime::{engine::Engine, host::HostApi};

#[path = "common/mod.rs"]
mod common;
use common::find_available_port;

const LOGS_WRITER_COMPONENT: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/target/wasm32-wasip2/release/deps/logs_writer.wasm"
));
const TEST_COMPONENT: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/test-component/target/wasm32-wasip2/release/deps/test_component.wasm"
));

const APPLICATION_ID: &str = "app-1";
const JAWS_SECRET: &str = "SUPER_SECRET";
const HTTP_HOST_HEADER: &str = "logs-writer-test";

static TRACING: Once = Once::new();

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

#[derive(Debug, Clone)]
struct CapturedRequest {
    path: String,
    headers: HeaderMap,
    body: String,
}

#[derive(Clone, Default)]
struct CaptureState {
    requests: Arc<Mutex<Vec<CapturedRequest>>>,
}

async fn capture_request(
    State(state): State<CaptureState>,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> StatusCode {
    let body = String::from_utf8(body.to_vec()).unwrap_or_default();
    let mut requests = state.requests.lock().unwrap();
    requests.push(CapturedRequest {
        path: uri.path().to_string(),
        headers,
        body,
    });

    StatusCode::CREATED
}

async fn start_mock_logging_server()
-> anyhow::Result<(SocketAddr, CaptureState, task::JoinHandle<()>)> {
    let state = CaptureState::default();
    let app = Router::new()
        .route("/internal/bulk_logs", post(capture_request))
        .route("/internal/bulk_variables", post(capture_request))
        .with_state(state.clone());

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;

    let handle = task::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("mock logging server failed");
    });

    Ok((addr, state, handle))
}

struct TestConfig {
    host: Arc<Host>,
    http_addr: SocketAddr,
}

async fn setup_wasmcloud_host() -> anyhow::Result<TestConfig> {
    let http_port = find_available_port().await?;
    let http_addr: SocketAddr = format!("127.0.0.1:{http_port}").parse().unwrap();

    let engine = Engine::builder().build()?;

    let http_handler = DevRouter::default();
    let http_plugin = HttpServer::new(http_handler, http_addr).await?;

    info!("Starting host");
    let host = HostBuilder::new()
        .with_engine(engine)
        .with_http_handler(Arc::new(http_plugin))
        .build()?;

    let host = host.start().await?;

    Ok(TestConfig { host, http_addr })
}

async fn start_workload(test_config: &TestConfig, logs_addr: SocketAddr) -> anyhow::Result<String> {
    let config = HashMap::from([
        ("APPLICATION_ID".to_string(), APPLICATION_ID.to_string()),
        ("LOGS_WRITER_SCHEME".to_string(), "http".to_string()),
        ("LOGS_WRITER_HOST".to_string(), "127.0.0.1".to_string()),
        ("LOGS_WRITER_PORT".to_string(), logs_addr.port().to_string()),
        (
            "ACTIONS_WASM_LOGS_WRITER_SECRET".to_string(),
            JAWS_SECRET.to_string(),
        ),
        ("JAWS_ISSUER".to_string(), "actions-wasm".to_string()),
    ]);

    let allowed_hosts = vec![format!("127.0.0.1:{}", logs_addr.port())];

    let req = WorkloadStartRequest {
        workload_id: uuid::Uuid::new_v4().to_string(),
        workload: Workload {
            namespace: "test".to_string(),
            name: "logs-writer-workload".to_string(),
            annotations: HashMap::new(),
            service: None,
            components: vec![
                Component {
                    name: "logs-writer".to_string(),
                    bytes: bytes::Bytes::from_static(LOGS_WRITER_COMPONENT),
                    digest: None,
                    local_resources: LocalResources {
                        memory_limit_mb: 256,
                        cpu_limit: 1,
                        config: HashMap::new(),
                        environment: config.clone(),
                        volume_mounts: vec![],
                        allowed_hosts: allowed_hosts.into(),
                    },
                    pool_size: 1,
                    max_invocations: 100,
                },
                Component {
                    name: "test-component".to_string(),
                    bytes: bytes::Bytes::from_static(TEST_COMPONENT),
                    digest: None,
                    local_resources: LocalResources {
                        memory_limit_mb: 256,
                        cpu_limit: 1,
                        config: HashMap::new(),
                        environment: config.clone(),
                        volume_mounts: vec![],
                        allowed_hosts: vec![].into(),
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
                        config.insert("host".to_string(), HTTP_HOST_HEADER.to_string());
                        config
                    },
                    name: None,
                },
                WitInterface {
                    namespace: "wasi".to_string(),
                    package: "http".to_string(),
                    interfaces: ["outgoing-handler".to_string()].into_iter().collect(),
                    version: Some(semver::Version::parse("0.2.4").unwrap()),
                    config: HashMap::new(),
                    name: None,
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

async fn wait_for_requests(
    state: &CaptureState,
    expected: usize,
    timeout: Duration,
) -> Vec<CapturedRequest> {
    let start = Instant::now();

    loop {
        let current = state.requests.lock().unwrap().clone();
        if current.len() >= expected {
            return current;
        }

        if start.elapsed() > timeout {
            return current;
        }

        sleep(Duration::from_millis(50)).await;
    }
}

#[tokio::test]
async fn logs_writer_sends_logs_and_variables_to_server() -> anyhow::Result<()> {
    init_tracing_for_test();

    let (logs_addr, capture_state, server_handle) = start_mock_logging_server().await?;
    let test_config = setup_wasmcloud_host().await?;
    let _ = start_workload(&test_config, logs_addr).await?;

    let client = Client::new();
    info!("Performing request");
    let response = client
        .get(format!("http://{}/ping", test_config.http_addr))
        .header("HOST", HTTP_HOST_HEADER)
        .send()
        .await?;

    assert_eq!(response.status(), reqwest::StatusCode::OK);

    let captured = wait_for_requests(&capture_state, 2, Duration::from_secs(5)).await;

    assert!(
        captured
            .iter()
            .any(|request| request.path == "/internal/bulk_logs"),
        "missing bulk logs request"
    );
    assert!(
        captured
            .iter()
            .any(|request| request.path == "/internal/bulk_variables"),
        "missing bulk variables request"
    );
    assert!(
        captured
            .iter()
            .any(|request| request.body.contains("test-log: ping")),
        "missing log message in payloads"
    );
    assert!(
        captured
            .iter()
            .any(|request| request.body.contains("\"hash\":\"v1\"")
                && request.body.contains("app-1")),
        "missing variable payload"
    );

    for request in &captured {
        assert!(
            request.headers.get("authorization").is_some(),
            "missing authorization header"
        );
    }

    server_handle.abort();

    Ok(())
}
