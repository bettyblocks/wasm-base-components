use std::env;

use anyhow::Context;
use tonic::Request;
use tracing::debug;
use wasmcloud_grpc_client::GrpcEndpoint;

use exports::betty_blocks_types::auth::validator::{Guest as ValidatorGuest, ValidateError};
use exports::betty_blocks_types::data_api::data_api::{Guest, HelperContext};

#[allow(clippy::result_large_err)]
pub mod data_grpc {
    tonic::include_proto!("data_grpc");
}
pub mod context;

use context::DataAPIContext;
use data_grpc::{
    Context as GrpcContext, DataApiRequest, DataApiResult, ValidateJwtRequest, ValidateJwtResult,
    data_api_client::DataApiClient, data_api_result::Status, validate_jwt_result,
};

wit_bindgen::generate!({ generate_all });

const STATUS_ERROR: i32 = Status::Error as i32;
const STATUS_OK: i32 = Status::Ok as i32;
const VALIDATE_STATUS_OK: i32 = validate_jwt_result::Status::Ok as i32;

/// Error codes of the data-api that say the data-api failed, not that the token is invalid.
const VALIDATE_UNAVAILABLE_CODES: [&str; 3] = [
    "INTERNAL_SERVER_ERROR",
    "ARTIFACT_FETCHING_FAILED",
    "UNAUTHENTICATED_REQUEST",
];

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

impl ValidatorGuest for DataApi {
    fn validate_jwt(application_id: String, jwt: String) -> Result<String, ValidateError> {
        let config = Config::from_env()
            .map_err(|e| ValidateError::Unavailable(format!("Configuration error: {e:#}")))?;

        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| {
                ValidateError::Unavailable(format!("failed to create tokio runtime: {e}"))
            })?;

        runtime.block_on(inner_validate_jwt(config, &application_id, jwt))
    }
}

export!(DataApi);

async fn inner_validate_jwt(
    config: Config,
    application_id: &str,
    jwt: String,
) -> Result<String, ValidateError> {
    let unavailable = |e: anyhow::Error| ValidateError::Unavailable(format!("{e:#}"));

    let mut client = grpc_client(&config).map_err(unavailable)?;

    let mut request = Request::new(ValidateJwtRequest {
        context: Some(GrpcContext {
            application_id: application_id.to_string(),
            jwt,
        }),
    });
    add_jaws(&mut request, &config, application_id).map_err(unavailable)?;

    let response = client
        .validate_jwt(request)
        .await
        .map_err(|e| ValidateError::Unavailable(format!("gRPC call failed: {e}")))?;

    match response.into_inner() {
        ValidateJwtResult {
            status: VALIDATE_STATUS_OK,
            result,
        } => Ok(result),
        ValidateJwtResult { result, .. } => Err(validate_error(&result)),
    }
}

/// The data-api answers an error with `{"code": "...", "message": "..."}`.
fn validate_error(result: &str) -> ValidateError {
    #[derive(serde::Deserialize)]
    struct DataApiError {
        code: String,
        message: String,
    }

    match serde_json::from_str::<DataApiError>(result) {
        Ok(DataApiError { code, message })
            if VALIDATE_UNAVAILABLE_CODES.contains(&code.as_str()) =>
        {
            ValidateError::Unavailable(message)
        }
        Ok(DataApiError { message, .. }) => ValidateError::Invalid(message),
        Err(_) => ValidateError::Unavailable(format!("unexpected response: {result}")),
    }
}

fn grpc_client(config: &Config) -> anyhow::Result<DataApiClient<GrpcEndpoint>> {
    debug!("Parsing gRPC endpoint URI");
    let endpoint_uri = config
        .grpc_server_uri
        .parse()
        .context("Failed to parse GRPC_SERVER_URI")?;

    Ok(DataApiClient::new(GrpcEndpoint::new(endpoint_uri)))
}

fn add_jaws<T>(
    request: &mut Request<T>,
    config: &Config,
    application_id: &str,
) -> anyhow::Result<()> {
    let token = generate_jaws(config, application_id.to_string())?;

    request.metadata_mut().insert(
        "authorization",
        format!("Bearer {token}")
            .parse()
            .context("Failed to create valid bearer header")?,
    );
    Ok(())
}

async fn inner_request(
    config: Config,
    application_id: &str,
    jwt: Option<String>,
    query: String,
    variables: String,
) -> anyhow::Result<String> {
    let mut client = grpc_client(&config)?;

    let mut request = Request::new(DataApiRequest {
        query,
        variables,
        context: Some(GrpcContext {
            application_id: application_id.to_string(),
            jwt: jwt.unwrap_or_default(),
        }),
    });
    add_jaws(&mut request, &config, application_id)?;

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_error_of_an_invalid_token_is_invalid() {
        let result = validate_error(r#"{"code":"EXPIRED","message":"Token expired"}"#);
        assert!(matches!(result, ValidateError::Invalid(msg) if msg == "Token expired"));
    }

    #[test]
    fn validate_error_of_a_failing_data_api_is_unavailable() {
        for code in VALIDATE_UNAVAILABLE_CODES {
            let result = validate_error(&format!(r#"{{"code":"{code}","message":"oops"}}"#));
            assert!(
                matches!(result, ValidateError::Unavailable(msg) if msg == "oops"),
                "{code}"
            );
        }
    }

    #[test]
    fn validate_error_of_an_unexpected_response_is_unavailable() {
        let result = validate_error("not json");
        assert!(matches!(result, ValidateError::Unavailable(_)));
    }
}
