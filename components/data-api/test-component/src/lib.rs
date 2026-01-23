use serde::Deserialize;
use wasmcloud_component::http;

wit_bindgen::generate!({ generate_all });

use crate::betty_blocks::data_api::data_api::{request as data_api_request, HelperContext};

#[derive(Deserialize, Default)]
struct DataApiContext {
    jwt: Option<String>,
    application_id: Option<String>,
}

#[derive(Deserialize)]
struct RequestBody {
    context: Option<DataApiContext>,
    query: String,
    variables: String,
}

struct Component;
// 2**24 = 16mb
const MAX_READ: u64 = 2u64.pow(24);

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
    } = match serde_json::from_slice(&body_bytes) {
        Ok(rb) => rb,
        Err(e) => return Err(Error::InvalidInput(e.to_string())),
    };

    let context = context.unwrap_or(DataApiContext::default());

    let helper_context = HelperContext {
        application_id: context
            .application_id
            .unwrap_or("empty".to_string())
            .to_string(),
        action_id: "empty".to_string(),
        log_id: "empty".to_string(),
        encrypted_configurations: None,
        jwt: context.jwt,
    };

    let result = data_api_request(&helper_context, &query, &variables);
    match result {
        Ok(response) => Ok(http::Response::new(response)),
        Err(e) => {
            let mut response = http::Response::new(e);
            *response.status_mut() = http::StatusCode::BAD_REQUEST;
            Ok(response)
        },
    }
}

impl http::Server for Component {
    fn handle(
        request: http::IncomingRequest,
    ) -> http::Result<http::Response<impl http::OutgoingBody>> {
        match inner_handle(request) {
            Ok(response) => Ok(response),
            Err(e) => Ok(e.into()),
        }
    }
}

http::export!(Component);
