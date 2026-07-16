use anyhow::Result;
use multipart::client::lazy::Multipart;
use serde::Deserialize;
use std::io::Read;
use tracing::{debug, error};
use wstd::http::{Body, Client, Request};

use crate::bindings::{
    betty_blocks_types::data_api::data_api::{self, HelperContext},
    betty_blocks_types::types::types::{BettyModel, BettyProperty},
    exports::betty_blocks_types::upload_file::upload_file::UploadResult,
};

const GENERATE_FILE_UPLOAD_URL_REQUEST: &str = r#"
mutation GenerateFileUploadRequest(
    $modelName: String!,
    $propertyName: String!,
    $contentType: String!,
    $fileName: String!
) {
    generateFileUploadRequest(
        modelName: $modelName,
        propertyName: $propertyName,
        contentType: $contentType,
        fileName: $fileName
    ) {
        ... on PresignedPostRequest {
            reference
            fields
            url
        }
    }
}
"#;

struct PolicyField {
    key: String,
    value: String,
}

fn deserialize_policy_fields<'de, D>(
    deserializer: D,
) -> std::result::Result<Vec<PolicyField>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de;
    use serde_json::Value;

    let value = Value::deserialize(deserializer)?;
    match value {
        Value::Array(array) => {
            let mut fields = Vec::new();
            for item in array {
                let key = item
                    .get("key")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| de::Error::custom("missing 'key' in policy field"))?
                    .to_string();
                let value = item
                    .get("value")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| de::Error::custom("missing 'value' in policy field"))?
                    .to_string();
                fields.push(PolicyField { key, value });
            }
            Ok(fields)
        }
        Value::Object(map) => {
            let fields = map
                .into_iter()
                .map(|(key, value)| {
                    let value = value
                        .as_str()
                        .ok_or_else(|| de::Error::custom("expected string value in fields map"))?
                        .to_string();
                    Ok(PolicyField { key, value })
                })
                .collect::<std::result::Result<Vec<_>, D::Error>>()?;
            Ok(fields)
        }
        _ => Err(de::Error::custom("expected array or object for fields")),
    }
}

#[derive(Deserialize)]
struct PresignedPostRequest {
    reference: String,
    #[serde(deserialize_with = "deserialize_policy_fields")]
    fields: Vec<PolicyField>,
    url: String,
}

pub async fn upload_bytes_internal(
    helper_context: HelperContext,
    model: BettyModel,
    property: BettyProperty,
    file_bytes: Vec<u8>,
    filename: String,
) -> Result<UploadResult> {
    let file_size = file_bytes.len() as u64;

    let variables = serde_json::json!({
        "modelName": model.name,
        "propertyName": property.name,
        "fileName": filename,
        // The contentType will be handled by the data-api.
        "contentType": "",
    })
    .to_string();

    let response_str = data_api::request(
        &helper_context,
        GENERATE_FILE_UPLOAD_URL_REQUEST,
        &variables,
    )
    .map_err(|e| {
        error!("upload_bytes_internal: GraphQL mutation failed: {}", e);
        anyhow::anyhow!("GraphQL mutation failed: {}", e)
    })?;

    let response: serde_json::Value = serde_json::from_str(&response_str)
        .map_err(|e| anyhow::anyhow!("Failed to parse mutation response: {}", e))?;

    if let Some(serde_json::Value::Array(errors)) = response.get("errors") {
        let messages: Vec<String> = errors
            .iter()
            .filter_map(|e| Some(e.get("message")?.as_str()?.to_owned()))
            .collect();
        return Err(anyhow::anyhow!("GraphQL errors: {}", messages.join("; ")));
    }

    let presigned_post: PresignedPostRequest = serde_json::from_value(
        response
            .pointer("/data/generateFileUploadRequest")
            .ok_or_else(|| anyhow::anyhow!("Missing data.generateFileUploadRequest in response"))?
            .clone(),
    )
    .map_err(|e| anyhow::anyhow!("Failed to parse presigned post: {}", e))?;

    upload_to_presigned_post(&presigned_post, file_bytes, &filename).await?;

    Ok(UploadResult {
        reference: presigned_post.reference,
        file_size,
        message: Some("Upload successful".into()),
    })
}

async fn upload_to_presigned_post(
    presigned_post: &PresignedPostRequest,
    file_bytes: Vec<u8>,
    filename: &str,
) -> Result<()> {
    let client = Client::new();
    let mut form = Multipart::new();

    for field in &presigned_post.fields {
        form.add_text(field.key.clone(), field.value.clone());
    }

    form.add_stream("file", file_bytes.as_slice(), Some(filename), None);

    let mut prepared = form
        .prepare()
        .map_err(|e| anyhow::anyhow!("Failed to prepare multipart form: {e}"))?;

    let content_type_header = format!("multipart/form-data; boundary={}", prepared.boundary());

    let mut body_bytes = Vec::new();
    prepared
        .read_to_end(&mut body_bytes)
        .map_err(|e| anyhow::anyhow!("Failed to read multipart body: {e}"))?;

    let request = Request::post(&presigned_post.url)
        .header("content-type", &*content_type_header)
        .header("content-length", body_bytes.len().to_string().as_str())
        .body(Body::from(body_bytes))
        .map_err(|e| anyhow::anyhow!("Failed to build upload request: {e}"))?;

    let response = client
        .send(request)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to send upload request: {e}"))?;

    let status = response.status().as_u16();
    debug!("Status: {}", status);

    if status >= 300 {
        let mut err_body = response.into_body();
        let err = match err_body.contents().await {
            Ok(b) => String::from_utf8_lossy(b).to_string(),
            Err(_) => String::new(),
        };
        debug!("Error body: {}", err);
        return Err(anyhow::anyhow!(
            "upload failed with status {}: {}",
            status,
            err
        ));
    }

    debug!("Presigned POST upload succeeded");
    Ok(())
}
