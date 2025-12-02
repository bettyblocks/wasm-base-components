use wasmcloud_component::http;

wit_bindgen::generate!({ generate_all });

use crate::betty_blocks::crud::crud::{create as crud_create, HelperContext, Model};

struct Component;

enum Error {}

impl From<Error> for http::Response<String> {
    fn from(val: Error) -> Self {
        match val {}
    }
}

fn inner_handle(_request: http::IncomingRequest) -> Result<http::Response<String>, Error> {
    let helper_context = HelperContext {
        application_id: "haha".to_string(),
        action_id: "empty".to_string(),
        log_id: "empty".to_string(),
        encrypted_configurations: None,
        jwt: None,
    };

    let validates = vec!["default".to_string()];

    let response = crud_create(
        &helper_context,
        &Model {
            name: "user".to_string(),
        },
        &vec![],
        Some(&validates),
    );

    match response {
        Ok(resp) => Ok(http::response::Response::new(resp.to_string())),
        Err(e) => Ok(http::response::Response::new(e.to_string())),
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
