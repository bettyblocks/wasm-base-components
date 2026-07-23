pub mod bindings {
    wit_bindgen::generate!({
        generate_all,
    });
}

use std::env;

use anyhow::Context;
use wstd::http::{Body, Client, Request};

use bindings::{
    betty_blocks_types::data_api::data_api::HelperContext,
    exports::betty_blocks_types::pdf_generator::pdf_generator::Guest,
};

struct Config {
    pdf_generator_url: String,
    jaws_issuer: String,
    jaws_secret_key: String,
}

impl Config {
    fn from_env() -> anyhow::Result<Self> {
        Ok(Self {
            pdf_generator_url: env::var("PDF_GENERATOR_URL")
                .unwrap_or_else(|_| "http://pdf-generator:4000".to_string()),
            jaws_issuer: env::var("JAWS_ISSUER").unwrap_or_else(|_| "actions-wasm".to_string()),
            jaws_secret_key: env::var("JAWS_SECRET_KEY").context("JAWS_SECRET_KEY must be set")?,
        })
    }
}

struct Component;

impl Guest for Component {
    fn generate(helper_context: HelperContext, html: String) -> Result<Vec<u8>, String> {
        let config = match Config::from_env() {
            Ok(config) => config,
            Err(e) => return Err(format!("Configuration error: {e:#}")),
        };

        wstd::runtime::block_on(generate_pdf(config, &helper_context.application_id, html))
            .map_err(|e| format!("{e:#}"))
    }
}

bindings::export!(Component with_types_in bindings);

async fn generate_pdf(
    config: Config,
    application_id: &str,
    html: String,
) -> anyhow::Result<Vec<u8>> {
    let token = generate_jaws(&config, application_id.to_string())?;
    let url = format!(
        "{}/html-to-pdf/{}",
        config.pdf_generator_url.trim_end_matches('/'),
        application_id
    );

    let body_bytes = html.into_bytes();
    let request = Request::post(url.as_str())
        .header("content-type", "text/html")
        .header("authorization", format!("Bearer {token}").as_str())
        .header("content-length", body_bytes.len().to_string().as_str())
        .body(Body::from(body_bytes))
        .map_err(|e| anyhow::anyhow!("Failed to build pdf-generator request: {e}"))?;

    let response = Client::new()
        .send(request)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to send request to pdf-generator: {e}"))?;

    let status = response.status().as_u16();
    let mut body = response.into_body();
    let contents = body
        .contents()
        .await
        .map_err(|e| anyhow::anyhow!("Failed to read pdf-generator response body: {e}"))?;

    if !(200..300).contains(&status) {
        anyhow::bail!(
            "Failed to generate PDF: pdf-generator returned status {}: {}",
            status,
            String::from_utf8_lossy(contents)
        );
    }

    Ok(contents.to_vec())
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
