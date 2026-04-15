use std::io::Read;

use wasmcloud_component::http::{self, IncomingBody};

pub mod bindings {
    wit_bindgen::generate!({ generate_all });
}

use crate::bindings::betty_blocks::actions::actions::{
    Error as ActionError, RunInput as Input, RunOutput as Output, RunPayload as Payload, call,
};

struct Component;

#[derive(serde::Deserialize, Debug)]
#[cfg_attr(test, derive(serde::Serialize))]
struct PayloadWrapper {
    input: String,
    configurations: String,
}
#[derive(serde::Deserialize, Debug)]
#[cfg_attr(test, derive(serde::Serialize))]
struct InputWrapper {
    action_id: String,
    payload: PayloadWrapper,
}

impl From<InputWrapper> for Input {
    fn from(val: InputWrapper) -> Self {
        Input {
            action_id: val.action_id,
            payload: Payload {
                input: val.payload.input,
                configurations: val.payload.configurations,
            },
        }
    }
}

// 2**24 = 16mb
const MAX_READ_MB: u64 = 16;
const MEGABYTE: u64 = 2u64.pow(20);
const MAX_READ: u64 = MAX_READ_MB * MEGABYTE;

trait IncomingRequestImpl {
    fn body_mut(&mut self) -> &mut impl IncomingBodyImpl;
}

trait IncomingBodyImpl: std::io::Read {}

impl IncomingRequestImpl for http::IncomingRequest {

    fn body_mut(&mut self) -> &mut impl IncomingBodyImpl {
        http::IncomingRequest::body_mut(self)
    }
}

impl IncomingBodyImpl for IncomingBody {}

#[derive(Debug, PartialEq)]
enum Error {
    InvalidBody(String),
    InvalidInput(String),
    InputTooLarge,
    ActionCallFailed(String),
    Forbidden(String),
}

impl From<Error> for http::Response<String> {
    fn from(val: Error) -> Self {
        match val {
            Error::InvalidBody(message) => {
                http::Response::builder().status(500).body(message).unwrap()
            }
            Error::InvalidInput(message) => {
                http::Response::builder().status(400).body(message).unwrap()
            }
            Error::InputTooLarge => http::Response::builder()
                .status(400)
                .body(format!("Body size exceeded the maximum of {MAX_READ_MB}mb"))
                .unwrap(),
            Error::ActionCallFailed(message) => {
                http::Response::builder().status(400).body(message).unwrap()
            }
            Error::Forbidden(message) => {
                http::Response::builder().status(403).body(message).unwrap()
            }
        }
    }
}

fn inner_handle<F>(
    mut request: impl IncomingRequestImpl,
    call_function: F,
) -> Result<http::Response<String>, Error>
where
    F: for<'a> FnOnce(&'a Input) -> Result<Output, ActionError>,
{

    let body = request.body_mut();
    let reader = body.take(MAX_READ);

    let input_wrapper = serde_json::from_reader::<_, InputWrapper>(reader).map_err(|e| {
        if e.is_io() {
            Error::InvalidBody(e.to_string())
        } else if e.is_eof() {
            Error::InputTooLarge
        } else {
            Error::InvalidInput(e.to_string())
        }
    })?;

    let input = input_wrapper.into();
    let result = call_function(&input).map_err(|action_error| match action_error {
        ActionError::RunFailed(msg) => Error::ActionCallFailed(msg),
        ActionError::Forbidden => Error::Forbidden("Action forbidden".to_string()),
    })?;

    Ok(http::Response::new(result.result))
}

impl http::Server for Component {
    fn handle(
        request: http::IncomingRequest,
    ) -> http::Result<http::Response<impl http::OutgoingBody>> {
        match inner_handle(request, call) {
            Ok(response) => Ok(response),
            Err(e) => Ok(e.into()),
        }
    }
}

http::export!(Component);

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use wasmcloud_component::http::StatusCode;

    use super::*;

    struct TestIncomingRequest {
        body: TestIncomingRequestBody,
    }

    struct TestIncomingRequestBody {
        cursor: Cursor<Vec<u8>>,
    }

    impl IncomingRequestImpl for TestIncomingRequest {
        fn method(&self) -> &http::Method {
            &http::Method::POST
        }

        fn body_mut(&mut self) -> &mut impl IncomingBodyImpl {
            &mut self.body
        }
    }

    impl std::io::Read for TestIncomingRequestBody {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            self.cursor.read(buf)
        }
    }

    impl IncomingBodyImpl for TestIncomingRequestBody {}

    #[test]
    fn handles_loading_the_input_in_chunks() {
        let input = InputWrapper {
            action_id: String::from("951e9a1360bc44d8a28943ab94d461be"),
            payload: PayloadWrapper {
                input: serde_json::Value::Object(Default::default()).to_string(),
                configurations: serde_json::Value::Object(Default::default()).to_string(),
            },
        };

        let test_action = |action_input: &Input| -> Result<Output, ActionError> {
            assert_eq!(action_input.action_id, input.action_id);
            assert_eq!(action_input.payload.input, input.payload.input);
            assert_eq!(
                action_input.payload.configurations,
                input.payload.configurations
            );
            Ok(Output {
                result: String::from("done"),
            })
        };

        let request = TestIncomingRequest {
            body: TestIncomingRequestBody {
                cursor: Cursor::new(serde_json::to_vec(&input).unwrap()),
            },
        };
        let response = inner_handle(request, test_action).unwrap();
        assert_eq!(response.body(), "done");
    }

    #[test]
    fn maximum_input_is_reached() {
        let object = serde_json::json!(
            {
                "abc": "abcdefgh",
                "def": "abcdefgh",
            }
        );

        let mut data = Vec::new();

        for _ in 0..390000 {
            data.push(object.clone());
        }

        let input = serde_json::json!(
            {
                "data": data
            }
        );

        let input = InputWrapper {
            action_id: String::from("951e9a1360bc44d8a28943ab94d461be"),
            payload: PayloadWrapper {
                input: input.to_string(),
                configurations: serde_json::Value::Object(Default::default()).to_string(),
            },
        };

        assert!(serde_json::to_vec(&input).unwrap().len() as u64 > MAX_READ);

        let test_action = |action_input: &Input| -> Result<Output, ActionError> {
            assert_eq!(action_input.action_id, input.action_id);
            assert_eq!(action_input.payload.input, input.payload.input);
            assert_eq!(
                action_input.payload.configurations,
                input.payload.configurations
            );
            Ok(Output {
                result: String::from("done"),
            })
        };

        let request = TestIncomingRequest {
            body: TestIncomingRequestBody {
                cursor: Cursor::new(serde_json::to_vec(&input).unwrap()),
            },
        };
        let response = inner_handle(request, test_action).unwrap_err();
        assert_eq!(response, Error::InputTooLarge);
    }

    #[test]
    fn test_error_messages() {
        let response = http::Response::from(Error::InvalidBody("it broke :(".to_string()));
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(response.body(), "it broke :(");

        let response = http::Response::from(Error::InvalidInput("it broke :(".to_string()));
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert_eq!(response.body(), "it broke :(");

        let response = http::Response::from(Error::InputTooLarge);
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert_eq!(response.body(), "Body size exceeded the maximum of 16mb");

        let response = http::Response::from(Error::ActionCallFailed("it broke :(".to_string()));
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert_eq!(response.body(), "it broke :(");

        let response = http::Response::from(Error::HealthCheckFailed("it broke :(".to_string()));
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(response.body(), "it broke :(");

        let response = http::Response::from(Error::Forbidden("it broke :(".to_string()));
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        assert_eq!(response.body(), "it broke :(");
    }

    #[test]
    fn test_action_returns_forbidden() {
        let input = InputWrapper {
            action_id: String::from("951e9a1360bc44d8a28943ab94d461be"),
            payload: PayloadWrapper {
                input: serde_json::Value::Object(Default::default()).to_string(),
                configurations: serde_json::Value::Object(Default::default()).to_string(),
            },
        };

        let test_action =
            |_: &Input| -> Result<Output, ActionError> { Err(ActionError::Forbidden) };

        let request = TestIncomingRequest {
            body: TestIncomingRequestBody {
                cursor: Cursor::new(serde_json::to_vec(&input).unwrap()),
            },
        };
        let result = inner_handle(request, test_action);

        assert!(matches!(result, Err(Error::Forbidden(_))));
        if let Err(Error::Forbidden(msg)) = result {
            assert_eq!(msg, "Action forbidden");
        }
    }

    #[test]
    fn test_action_returns_run_failed() {
        let input = InputWrapper {
            action_id: String::from("951e9a1360bc44d8a28943ab94d461be"),
            payload: PayloadWrapper {
                input: serde_json::Value::Object(Default::default()).to_string(),
                configurations: serde_json::Value::Object(Default::default()).to_string(),
            },
        };

        let test_action = |_: &Input| -> Result<Output, ActionError> {
            Err(ActionError::RunFailed("Custom error message".to_string()))
        };

        let request = TestIncomingRequest {
            body: TestIncomingRequestBody {
                cursor: Cursor::new(serde_json::to_vec(&input).unwrap()),
            },
        };
        let result = inner_handle(request, test_action);

        assert!(matches!(result, Err(Error::ActionCallFailed(_))));
        if let Err(Error::ActionCallFailed(msg)) = result {
            assert_eq!(msg, "Custom error message");
        }
    }
}
