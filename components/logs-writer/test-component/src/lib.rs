use wasmcloud_component::http;

wit_bindgen::generate!({ generate_all });

use crate::betty_blocks_utilities::logs_writer::debug_logging::send_log_and_variables;

const APPLICATION_ID: &str = "app-1";

struct Component;

impl http::Server for Component {
    fn handle(
        _request: http::IncomingRequest,
    ) -> http::Result<http::Response<impl http::OutgoingBody>> {
        let message = serde_json::json!({
            "application_id": APPLICATION_ID,
            "level": "info",
            "message": "test-log: ping"
        })
        .to_string();

        let variables = serde_json::json!({
            APPLICATION_ID: {
                "v1": "test"
            }
        })
        .to_string();

        send_log_and_variables(&[message], &variables);

        Ok(http::Response::new("ok".to_string()))
    }
}

http::export!(Component);
