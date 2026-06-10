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
    #[serde(default)]
    capture_endpoint: Option<String>,
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

fn read_body(request: http::IncomingRequest) -> Result<Vec<u8>, Error> {
    let body = request.body();
    body.subscribe().block();
    let body_bytes = body
        .read(MAX_READ)
        .map_err(|error| Error::FailedToReadBody(error.to_string()))?;
    Ok(body_bytes)
}

fn handle_request(body_bytes: &[u8]) -> Result<http::Response<String>, Error> {
    let RequestBody {
        context,
        query,
        variables,
        ..
    } = match serde_json::from_slice(body_bytes) {
        Ok(rb) => rb,
        Err(error) => return Err(Error::InvalidInput(error.to_string())),
    };

    let context = context.unwrap_or_default();

    let helper_context = HelperContext {
        application_id: context
            .application_id
            .unwrap_or_else(|| "empty".to_string()),
        action_id: "empty".to_string(),
        log_id: "empty".to_string(),
        encrypted_configurations: None,
        jwt: context.jwt,
    };

    let result = data_api_request(&helper_context, &query, &variables);
    match result {
        Ok(response) => Ok(http::Response::new(response)),
        Err(error) => {
            let mut response = http::Response::new(error);
            *response.status_mut() = http::StatusCode::BAD_REQUEST;
            Ok(response)
        },
    }
}

fn make_helper_context(context: Option<DataApiContext>) -> HelperContext {
    let ctx = context.unwrap_or_default();
    HelperContext {
        application_id: ctx
            .application_id
            .unwrap_or_else(|| "empty".to_string()),
        action_id: "empty".to_string(),
        log_id: "empty".to_string(),
        encrypted_configurations: None,
        jwt: ctx.jwt,
    }
}

fn handle_capture_create(body_bytes: &[u8]) -> http::Result<http::Response<String>> {
    let RequestBody { context, .. } = match serde_json::from_slice(body_bytes) {
        Ok(rb) => rb,
        Err(error) => {
            let error = Error::InvalidInput(error.to_string());
            return Ok(error.into());
        }
    };

    let helper_context = make_helper_context(context);
    let result = data_api_request(
        &helper_context,
        r#"mutation ($input: userInput) { createUser(input: $input) { id } }"#,
        r#"{"name": "Alice"}"#,
    );

    Ok(match result {
        Ok(response) => http::Response::new(response),
        Err(error) => {
            let mut response = http::Response::new(error);
            *response.status_mut() = http::StatusCode::BAD_REQUEST;
            response
        }
    })
}

fn handle_capture_update(body_bytes: &[u8]) -> http::Result<http::Response<String>> {
    let RequestBody { context, .. } = match serde_json::from_slice(body_bytes) {
        Ok(rb) => rb,
        Err(error) => {
            let error = Error::InvalidInput(error.to_string());
            return Ok(error.into());
        }
    };

    let helper_context = make_helper_context(context);
    let result = data_api_request(
        &helper_context,
        r#"mutation ($id: Int!, $input: userInput) { updateUser(id: $id, input: $input) { id } }"#,
        r#"{"id": 5, "name": "Bob"}"#,
    );

    Ok(match result {
        Ok(response) => http::Response::new(response),
        Err(error) => {
            let mut response = http::Response::new(error);
            *response.status_mut() = http::StatusCode::BAD_REQUEST;
            response
        }
    })
}

fn handle_capture_delete(body_bytes: &[u8]) -> http::Result<http::Response<String>> {
    let RequestBody { context, .. } = match serde_json::from_slice(body_bytes) {
        Ok(rb) => rb,
        Err(error) => {
            let error = Error::InvalidInput(error.to_string());
            return Ok(error.into());
        }
    };

    let helper_context = make_helper_context(context);
    let result = data_api_request(
        &helper_context,
        r#"mutation ($id: Int!) { deleteUser(id: $id) { id } }"#,
        r#"{"id": 5}"#,
    );

    Ok(match result {
        Ok(response) => http::Response::new(response),
        Err(error) => {
            let mut response = http::Response::new(error);
            *response.status_mut() = http::StatusCode::BAD_REQUEST;
            response
        }
    })
}

fn handle_capture_passthrough(body_bytes: &[u8]) -> http::Result<http::Response<String>> {
    let RequestBody { context, .. } = match serde_json::from_slice(body_bytes) {
        Ok(rb) => rb,
        Err(error) => {
            let error = Error::InvalidInput(error.to_string());
            return Ok(error.into());
        }
    };

    let helper_context = make_helper_context(context);
    let result = data_api_request(
        &helper_context,
        r#"query { allUser { id name } }"#,
        r#"{}"#,
    );

    Ok(match result {
        Ok(response) => http::Response::new(response),
        Err(error) => {
            let mut response = http::Response::new(error);
            *response.status_mut() = http::StatusCode::BAD_REQUEST;
            response
        }
    })
}

fn handle_capture_all_operations(body_bytes: &[u8]) -> http::Result<http::Response<String>> {
    let RequestBody { context, .. } = match serde_json::from_slice(body_bytes) {
        Ok(rb) => rb,
        Err(error) => {
            let error = Error::InvalidInput(error.to_string());
            return Ok(error.into());
        }
    };

    let helper_context = make_helper_context(context);

    let _ = data_api_request(
        &helper_context,
        r#"mutation ($input: userInput) { createUser(input: $input) { id } }"#,
        r#"{"name": "Alice"}"#,
    );

    let _ = data_api_request(
        &helper_context,
        r#"mutation ($id: Int!, $input: userInput) { updateUser(id: $id, input: $input) { id } }"#,
        r#"{"id": 5, "name": "Bob"}"#,
    );

    let result = data_api_request(
        &helper_context,
        r#"mutation ($id: Int!) { deleteUser(id: $id) { id } }"#,
        r#"{"id": 10}"#,
    );

    Ok(match result {
        Ok(response) => http::Response::new(response),
        Err(error) => {
            let mut response = http::Response::new(error);
            *response.status_mut() = http::StatusCode::BAD_REQUEST;
            response
        }
    })
}

fn handle_capture_negative_id_relation(body_bytes: &[u8]) -> http::Result<http::Response<String>> {
    let RequestBody { context, .. } = match serde_json::from_slice(body_bytes) {
        Ok(rb) => rb,
        Err(error) => {
            let error = Error::InvalidInput(error.to_string());
            return Ok(error.into());
        }
    };

    let helper_context = make_helper_context(context);

    let _ = data_api_request(
        &helper_context,
        r#"mutation ($input: userInput) { createUser(input: $input) { id } }"#,
        r#"{"id": -1, "name": "Alice"}"#,
    );

    let result = data_api_request(
        &helper_context,
        r#"mutation ($input: orderInput) { createOrder(input: $input) { id } }"#,
        r#"{"user": {"id": -1}, "product": "Widget"}"#,
    );

    Ok(match result {
        Ok(response) => http::Response::new(response),
        Err(error) => {
            let mut response = http::Response::new(error);
            *response.status_mut() = http::StatusCode::BAD_REQUEST;
            response
        }
    })
}

impl http::Server for Component {
    fn handle(
        request: http::IncomingRequest,
    ) -> http::Result<http::Response<impl http::OutgoingBody>> {
        let body_bytes = match read_body(request) {
            Ok(bytes) => bytes,
            Err(error) => return Ok(error.into()),
        };

        // Try to parse the request body to get the capture endpoint
        if let Ok(request_body) = serde_json::from_slice::<RequestBody>(&body_bytes) {
            if let Some(endpoint) = request_body.capture_endpoint {
                return match endpoint.as_str() {
                    "create" => handle_capture_create(&body_bytes),
                    "update" => handle_capture_update(&body_bytes),
                    "delete" => handle_capture_delete(&body_bytes),
                    "passthrough" => handle_capture_passthrough(&body_bytes),
                    "all-operations" => handle_capture_all_operations(&body_bytes),
                    "negative-id-relation" => handle_capture_negative_id_relation(&body_bytes),
                    _ => {
                        let error = Error::InvalidInput("Unknown capture endpoint".to_string());
                        Ok(error.into())
                    }
                };
            }
        }

        // Default handler
        match handle_request(&body_bytes) {
            Ok(response) => Ok(response),
            Err(error) => Ok(error.into()),
        }
    }
}

http::export!(Component);
