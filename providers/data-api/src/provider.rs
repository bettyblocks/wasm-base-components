use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Context as _;
use data_grpc::data_api_client::DataApiClient;
use data_grpc::DataApiRequest;
use rand::distr::Alphanumeric;
use rand::distr::SampleString;
use tokio::sync::RwLock;
use tracing::info;
use wasmcloud_provider_sdk::provider::WrpcClient;
use wasmcloud_provider_sdk::{initialize_observability, load_host_data};
use wasmcloud_provider_sdk::{run_provider, serve_provider_exports, Context, Provider};

pub mod data_grpc {
    tonic::include_proto!("data_grpc"); // The string specified here must match the proto package name
}

use crate::provider::bindings::betty_blocks::key_vault::key_vault;
use crate::provider::data_grpc::data_api_result::Status;
use bindings::exports::betty_blocks::data_api::data_api::Handler;
use bindings::exports::betty_blocks::data_api::data_api::HelperContext;

// Set the limit to 8MB, Grpc defaults to 4MB.
const REQUEST_SIZE_LIMIT: usize = 8 * 1024 * 1024;
const DEFAULT_DATA_API_ADDRESS: &str = "http://0.0.0.0:50054";

pub(crate) mod bindings {
    wit_bindgen_wrpc::generate!({
        with: {
            "betty-blocks:key-vault/key-vault": generate
        }
    });
}

#[derive(Clone)]
pub struct DataAPIProvider {
    static_config: HashMap<String, String>,
    wrpc_client: Arc<RwLock<Option<WrpcClient>>>,
}

impl DataAPIProvider {
    fn new(config: HashMap<String, String>) -> Self {
        DataAPIProvider {
            static_config: config,
            wrpc_client: Arc::new(RwLock::new(None)),
        }
    }

    fn name() -> &'static str {
        "data-api-provider"
    }

    pub async fn run() -> anyhow::Result<()> {
        initialize_observability!(
            Self::name(),
            std::env::var_os("PROVIDER_CUSTOM_TEMPLATE_FLAMEGRAPH_PATH")
        );
        let host_data = load_host_data().context("failed to load host data")?;

        let provider = Self::new(host_data.config.clone());
        let shutdown = run_provider(provider.clone(), Self::name())
            .await
            .context("failed to run provider")?;

        let connection = wasmcloud_provider_sdk::get_connection();
        let wrpc_client = connection
            .get_wrpc_client(connection.provider_key())
            .await
            .context("failed to get wrpc client")?;

        serve_provider_exports(&wrpc_client, provider, shutdown, bindings::serve).await
    }

    fn data_api_address(&self) -> String {
        self.static_config
            .get("data-api-address")
            .unwrap_or(&DEFAULT_DATA_API_ADDRESS.to_string())
            .to_string()
    }

    async fn inner_request(
        &self,
        _ctx: Option<Context>,
        helper_context: HelperContext,
        query: String,
        variables: String,
    ) -> anyhow::Result<String> {
        let mut client = DataApiClient::connect(self.data_api_address())
            .await?
            .max_decoding_message_size(REQUEST_SIZE_LIMIT);
        let data_api_context = data_grpc::Context {
            application_id: helper_context.application_id.clone(),
            jwt: helper_context.jwt.unwrap_or_default(),
        };
        let data_api_request = DataApiRequest {
            query,
            variables,
            context: Some(data_api_context),
        };

        let token = self.generate_jaws(helper_context.application_id).await?;

        let mut request = tonic::Request::new(data_api_request);

        let metadata = request.metadata_mut();
        metadata.insert(
            "authorization",
            format!("Bearer {}", token)
                .parse()
                .expect("valid bearer header"),
        );

        info!("sending request");

        match client.execute(request).await {
            Ok(response) => {
                let response_data = response.into_inner();
                match Status::try_from(response_data.status) {
                    Ok(Status::Ok) => Ok(response_data.result),
                    _ => Err(anyhow::anyhow!(response_data.result)),
                }
            }
            Err(e) => Err(e.into()),
        }
    }

    async fn generate_jaws(&self, application_id: String) -> anyhow::Result<String> {
        let jaws_issuer = self
            .static_config
            .get("jaws-issuer")
            .cloned()
            .unwrap_or(String::from("actions-wasm"));

        let issued_at = jaws_rs::jsonwebtoken::get_current_timestamp();
        let claims = jaws_rs::Claims::new(jaws_issuer, application_id, issued_at, random_string());

        let secret_key = self.get_jaws_secret().await?;
        Ok(jaws_rs::encode(&claims, &secret_key)?)
    }

    async fn get_jaws_secret(&self) -> anyhow::Result<String> {
        let maybe_client = {
            let read_guard = self.wrpc_client.read().await;
            read_guard.clone()
        };

        if let Some(wrpc_client) = maybe_client {
            return self.inner_get_jaws_secret(&wrpc_client).await;
        }

        info!("making new connection to key-vault");

        let key_vault_target = self
            .static_config
            .get("key-vault-target")
            .cloned()
            .unwrap_or(String::from("key-vault"));

        let wrpc_client = wasmcloud_provider_sdk::get_connection()
            .get_wrpc_client(&key_vault_target)
            .await
            .context("failed to get wrpc client")?;

        {
            let mut write_guard = self.wrpc_client.write().await;
            *write_guard = Some(wrpc_client.clone());
        }

        self.inner_get_jaws_secret(&wrpc_client).await
    }

    async fn inner_get_jaws_secret(&self, wrpc_client: &WrpcClient) -> anyhow::Result<String> {
        let jaws_secret_key = self
            .static_config
            .get("jaws-secret-key")
            .cloned()
            .unwrap_or(String::from("ACTIONS_WASM_DATA_API_SECRET"));

        match key_vault::get_secret(wrpc_client, None, &jaws_secret_key).await {
            Ok(Some(secret)) => Ok(secret),
            Ok(None) => Err(anyhow::anyhow!("Jaws secret not found")),
            Err(e) => Err(e),
        }
    }
}

fn random_string() -> String {
    let mut rng = rand::rng();
    Alphanumeric::default().sample_string(&mut rng, 16)
}

impl Provider for DataAPIProvider {}

impl Handler<Option<Context>> for DataAPIProvider {
    async fn request(
        &self,
        ctx: Option<Context>,
        helper_context: HelperContext,
        query: String,
        variables: String,
    ) -> anyhow::Result<Result<String, String>> {
        info!("Hello to your logs from DataAPI provider");

        match self
            .inner_request(ctx, helper_context, query, variables)
            .await
        {
            Ok(x) => Ok(Ok(x)),
            Err(e) => Ok(Err(e.to_string())),
        }
    }
}
