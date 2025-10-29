use anyhow::Context as _;
use azure_identity::ManagedIdentityCredential;
use azure_security_keyvault_secrets::SecretClient;
use moka::future::Cache;
use std::collections::HashMap;
use std::time::Duration;
use wasmcloud_provider_sdk::{initialize_observability, load_host_data};
use wasmcloud_provider_sdk::{run_provider, serve_provider_exports, Context, Provider};

use bindings::exports::betty_blocks::key_vault::key_vault::Handler;
const CACHE_TTL_SECONDS: u64 = 30 * 60 * 60; // 30 minutes

pub(crate) mod bindings {
    wit_bindgen_wrpc::generate!();
}

#[derive(Debug, Clone)]
pub struct KeyVaultProvider {
    endpoint: String,
    key: String,
    keyvault_mock: Option<String>,
    cache: Cache<String, String>,
}

impl KeyVaultProvider {
    fn name() -> &'static str {
        "key-vault-provider"
    }

    fn new(endpoint: String, key: String, keyvault_mock: Option<String>) -> Self {
        let cache = Cache::builder()
            .time_to_live(Duration::from_secs(CACHE_TTL_SECONDS))
            .build();

        Self {
            endpoint,
            key,
            keyvault_mock,
            cache,
        }
    }

    pub async fn run() -> anyhow::Result<()> {
        initialize_observability!(
            Self::name(),
            std::env::var_os("PROVIDER_KEY_VAULT_FLAMEGRAPH_PATH")
        );
        let host_data = load_host_data().context("failed to load host data")?;
        let endpoint = host_data
            .config
            .get("endpoint")
            .expect("endpoint is required");
        let key = host_data.config.get("key").expect("key is required");
        let keyvault_mock = host_data.config.get("keyvault_mock").map(|s| s.to_string());

        let provider = Self::new(endpoint.to_string(), key.to_string(), keyvault_mock);
        let shutdown = run_provider(provider.clone(), Self::name())
            .await
            .context("failed to run provider")?;

        let connection = wasmcloud_provider_sdk::get_connection();

        serve_provider_exports(
            &connection
                .get_wrpc_client(connection.provider_key())
                .await
                .context("failed to get wrpc client")?,
            provider,
            shutdown,
            bindings::serve,
        )
        .await
    }

    async fn get_secret_from_keyvault(&self, key: &str) -> anyhow::Result<Option<String>> {
        let credential = ManagedIdentityCredential::new(None)?;
        let client = SecretClient::new(&self.endpoint, credential.clone(), None)?;

        let secret_response = client.get_secret(&self.key, None).await?;
        let secret_ref = secret_response.into_body().await?;
        if let Some(secret) = secret_ref.value {
            return self.get_from_json(&secret, key).await;
        }

        Ok(None)
    }

    async fn get_from_json(&self, json: &str, key: &str) -> anyhow::Result<Option<String>> {
        let value = serde_json::from_str::<HashMap<String, String>>(json)?
            .get(key)
            .cloned();

        if let Some(value) = value {
            self.cache.insert(key.to_string(), value.clone()).await;
            return Ok(Some(value));
        }

        Ok(None)
    }
}

impl Provider for KeyVaultProvider {}

impl Handler<Option<Context>> for KeyVaultProvider {
    async fn get_secret(
        &self,
        _cx: Option<Context>,
        key: String,
    ) -> anyhow::Result<Option<String>> {
        if let Some(value) = self.cache.get(&key).await {
            return Ok(Some(value));
        }

        if let Some(keyvault_mock) = &self.keyvault_mock {
            return self.get_from_json(keyvault_mock, &key).await;
        }

        self.get_secret_from_keyvault(&key).await
    }
}

#[cfg(test)]
use std::sync::Arc;

#[tokio::test]
async fn test_get_secret() -> anyhow::Result<()> {
    let json = r#"{"secret": "test"}"#;

    let provider = KeyVaultProvider::new(
        "https://example.vault.azure.net/".to_string(),
        "my-example-secrets".to_string(),
        Some(json.to_string()),
    );

    let secret = provider.get_secret(None, "secret".to_string()).await?;
    assert_eq!(secret, Some("test".to_string()));

    let secret = provider.get_secret(None, "not_found".to_string()).await?;
    assert_eq!(secret, None);

    Ok(())
}

#[tokio::test]
async fn test_get_secret_sets_cache() -> anyhow::Result<()> {
    let json = r#"{"secret": "test", "other": "test"}"#;

    let provider = KeyVaultProvider::new(
        "https://example.vault.azure.net/".to_string(),
        "my-example-secrets".to_string(),
        Some(json.to_string()),
    );

    let secret = provider.get_secret(None, "secret".to_string()).await?;
    assert_eq!(secret, Some("test".to_string()));

    let cache_data: Vec<(String, String)> = provider
        .cache
        .iter()
        .map(|(k, v)| (Arc::unwrap_or_clone(k), v))
        .collect();

    // only the key that was fetched gets cached
    let expected_cache_data = vec![(String::from("secret"), String::from("test"))];
    assert_eq!(cache_data, expected_cache_data);

    // double check that it really gets fetched from cache, set mock to empty object
    let mut empty_mock_provider = provider.clone();
    empty_mock_provider
        .keyvault_mock
        .replace(String::from("{}"));

    let secret = empty_mock_provider
        .get_secret(None, "secret".to_string())
        .await?;
    assert_eq!(secret, Some("test".to_string()));

    let secret = empty_mock_provider
        .get_secret(None, "other".to_string())
        .await?;
    assert_eq!(secret, None);

    Ok(())
}
