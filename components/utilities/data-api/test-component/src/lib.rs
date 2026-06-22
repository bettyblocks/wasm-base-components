use serde::Deserialize;
use wasmcloud_component::http;

wit_bindgen::generate!({ generate_all });

use crate::betty_blocks_utilities::data_api::data_api::{
    request as data_api_request, HelperContext as DataApiContext,
};

const EMPTY_FIELD: &str = "empty";
const MAX_READ: u64 = 2u64.pow(24); // 2**24 = 16mb

#[derive(Deserialize, Default)]
struct DataApiContextInput {
    jwt: Option<String>,
    application_id: Option<String>,
}

impl From<DataApiContextInput> for DataApiContext {
    fn from(input: DataApiContextInput) -> Self {
        DataApiContext {
            application_id: input
                .application_id
                .unwrap_or_else(|| EMPTY_FIELD.to_string()),
            action_id: EMPTY_FIELD.to_string(),
            log_id: EMPTY_FIELD.to_string(),
            encrypted_configurations: None,
            jwt: input.jwt,
        }
    }
}

#[derive(Deserialize)]
struct RequestBody {
    context: Option<DataApiContextInput>,
    query: String,
    variables: String,
}

struct Component;

enum Error {
    InvalidInput(String),
    FailedToReadBody(String),
}

impl From<Error> for http::Response<String> {
    fn from(val: Error) -> Self {
        match val {
            Error::InvalidInput(message) => {
                http::Response::builder().status(400).body(message).unwrap()
            }
            Error::FailedToReadBody(message) => {
                http::Response::builder().status(500).body(message).unwrap()
            }
        }
    }
}

fn inner_handle(request: http::IncomingRequest) -> Result<http::Response<String>, Error> {
    let body = request.body();

    body.subscribe().block();
    let body_bytes = body
        .read(MAX_READ)
        .map_err(|e| Error::FailedToReadBody(e.to_string()))?;

    let RequestBody {
        context,
        query,
        variables,
    } = serde_json::from_slice(&body_bytes).map_err(|e| Error::InvalidInput(e.to_string()))?;

    let context: DataApiContext = context.unwrap_or_default().into();

    let result = data_api_request(&context, &query, &variables);
    match result {
        Ok(response) => Ok(http::Response::new(response)),
        Err(e) => {
            let mut response = http::Response::new(e);
            *response.status_mut() = http::StatusCode::BAD_REQUEST;
            Ok(response)
        }
    }
}

impl http::Server for Component {
    fn handle(
        request: http::IncomingRequest,
    ) -> http::Result<http::Response<impl http::OutgoingBody>> {
        Ok(inner_handle(request).unwrap_or_else(|e| e.into()))
    }
}

http::export!(Component);
