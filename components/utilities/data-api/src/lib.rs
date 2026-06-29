use std::env;

use anyhow::Context;
use tonic::Request;
use tracing::debug;
use wasmcloud_grpc_client::GrpcEndpoint;

use exports::betty_blocks_utilities::data_api::data_api::{Guest, HelperContext};

pub mod data_grpc {
    tonic::include_proto!("data_grpc");
}
pub mod context;

use context::DataAPIContext;
use data_grpc::{
    Context as GrpcContext, DataApiRequest, DataApiResult, data_api_client::DataApiClient,
    data_api_result::Status,
};

wit_bindgen::generate!({ generate_all });

const STATUS_ERROR: i32 = Status::Error as i32;
const STATUS_OK: i32 = Status::Ok as i32;

struct Config {
    grpc_server_uri: String,
    jaws_issuer: String,
    jaws_secret_key: String,
}

impl Config {
    fn from_env() -> anyhow::Result<Self> {
        Ok(Self {
            grpc_server_uri: env::var("GRPC_SERVER_URI")
                .unwrap_or_else(|_| "http://data-api:50054".to_string()),
            jaws_issuer: env::var("JAWS_ISSUER").unwrap_or_else(|_| "actions-wasm".to_string()),
            jaws_secret_key: env::var("JAWS_SECRET_KEY").context("JAWS_SECRET_KEY must be set")?,
        })
    }
}

pub struct DataApi {}

impl Guest for DataApi {
    type DataApi = DataAPIContext;
    fn request(
        helper_context: HelperContext,
        query: String,
        variables: String,
    ) -> Result<String, String> {
        let config = match Config::from_env() {
            Ok(config) => config,
            Err(e) => return Err(format!("Configuration error: {e:#}")),
        };

        let runtime = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(rt) => rt,
            Err(e) => return Err(format!("failed to create tokio runtime: {e}")),
        };

        runtime
            .block_on(inner_request(
                config,
                &helper_context.application_id,
                helper_context.jwt,
                query,
                variables,
            ))
            .map_err(|e| format!("{e:#}"))
    }
}

export!(DataApi);

async fn inner_request(
    config: Config,
    application_id: &str,
    jwt: Option<String>,
    query: String,
    variables: String,
) -> anyhow::Result<String> {
    debug!("Parsing gRPC endpoint URI");
    let endpoint_uri = config
        .grpc_server_uri
        .parse()
        .context("Failed to parse GRPC_SERVER_URI")?;

    let endpoint = GrpcEndpoint::new(endpoint_uri);
    let mut client = DataApiClient::new(endpoint);

    let mut request = Request::new(DataApiRequest {
        query,
        variables,
        context: Some(GrpcContext {
            application_id: application_id.to_string(),
            jwt: jwt.unwrap_or_default(),
        }),
    });

    let token = generate_jaws(&config, application_id.to_string())?;

    let metadata = request.metadata_mut();
    metadata.insert(
        "authorization",
        format!("Bearer {token}")
            .parse()
            .context("Failed to create valid bearer header")?,
    );

    let response = client.execute(request).await.context("gRPC call failed")?;

    match response.into_inner() {
        DataApiResult {
            status: STATUS_OK,
            result,
        } => Ok(result),
        DataApiResult {
            status: STATUS_ERROR,
            result,
        } => Err(anyhow::anyhow!(result)),
        DataApiResult { .. } => unreachable!(),
    }
}

fn generate_jaws(config: &Config, application_id: String) -> anyhow::Result<String> {
    let issued_at = jaws_rs::jsonwebtoken::get_current_timestamp();
    let claims = jaws_rs::Claims::new(
        config.jaws_issuer.clone(),
        application_id,
        issued_at,
        uuid::Uuid::new_v4().to_string(),
    );

    jaws_rs::encode(&claims, &config.jaws_secret_key).context("Failed to encode JAWS token")
}
