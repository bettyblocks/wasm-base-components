mod bindings {
    wasmtime_testing_helper::bindgen!({ path: "wit", world: "provider", additional_derives: [PartialEq] });

    wasmtime_testing_helper::setup!(Provider);
}

use jaws_rs::Keys;
use prost::Message;
use prost::bytes::Buf;
use std::collections::VecDeque;
use wasmtime_testing_helper::http::{HttpHandler, hyper};
pub mod data_grpc {
    tonic::include_proto!("data_grpc");
}

use data_grpc::DataApiRequest;
use data_grpc::{DataApiResult, data_api_result::Status};

use crate::bindings::exports::betty_blocks_utilities::data_api::data_api::PendingMutation;

const APPLICATION_ID: &str = "06caae5da8234837a330c14a7350ed75";
const JWT: &str = "";
const ACTION_ID: &str = "";
const JAWS_SECRET: &str = "SUPER_SECRET";
const JAWS_DEFAULT_ISSUER: &str = "actions-wasm";

impl PendingMutation {
    fn new(mutation_name: &str, mutation: &str, variables: &str) -> Self {
        Self {
            mutation_name: String::from(mutation_name),
            mutation: String::from(mutation),
            variables: String::from(variables),
        }
    }
}

fn format_result_json(value: serde_json::Value) -> String {
    serde_json::to_string(&serde_json::json!({ "data": value }))
        .expect("JSON serialization should not fail")
}

fn format_error_unauthenticated() -> String {
    format_error_json(serde_json::json!({
        "message": "Request not authenticated",
        "extensions": { "code": "UNAUTHENTICATED" }
    }))
}

fn format_error_json(error: serde_json::Value) -> String {
    let errors = match error {
        serde_json::Value::Array(arr) => arr,
        other => vec![other],
    };

    serde_json::to_string(&serde_json::json!({ "errors": errors }))
        .expect("JSON serialization should not fail")
}

fn create_request_handler(
    mut requests: VecDeque<((String, String), DataApiResult)>,
) -> HttpHandler {
    Box::new(move |req, _conf| {
        let (expected_req, res) = requests
            .pop_front()
            .expect("received unexpected http request");
        Box::pin(async move {
            let (parts, body) = req.into_parts();
            let mut buf = body.await?;
            buf.advance(5);
            let new_req = DataApiRequest::decode(buf).unwrap();
            assert!(
                jaws_rs::decode_and_validate_jwt(
                    parts
                        .headers
                        .get("authorization")
                        .unwrap()
                        .to_str()
                        .unwrap()
                        .strip_prefix("Bearer ")
                        .unwrap(),
                    &Keys::from([(JAWS_DEFAULT_ISSUER.to_string(), JAWS_SECRET.to_string())])
                )
                .is_ok_and(|claims| claims.application_id == APPLICATION_ID)
            );
            assert!(
                matches!(new_req, DataApiRequest {context: Some(data_grpc::Context { application_id, jwt }), ..} if application_id.as_str() == APPLICATION_ID && jwt.as_str() == JWT)
            );
            assert_eq!((new_req.query, new_req.variables), expected_req);
            let res_bytes = res.encode_to_vec();
            let res_bytes_length = res_bytes.len();
            let mut response_body = Vec::with_capacity(res_bytes_length);
            response_body.push(0);
            response_body.extend_from_slice(&(res_bytes_length as u32).to_be_bytes());
            response_body.extend(res_bytes);

            Ok(hyper::Response::new(response_body.into()))
        })
    })
}

fn instantiate_component<const N: usize>(
    requests: [((&str, &str), DataApiResult); N],
) -> wasmtime_testing_helper::InstantiatedComponent<bindings::Provider> {
    let mut harness = bindings::harness();
    harness
        .wasi_context_builder_mut()
        .envs(&[
            ("JAWS_SECRET_KEY", JAWS_SECRET),
            ("GRPC_SERVER_URI", "http://."),
        ])
        .inherit_stdio();
    harness.mock_http_handler(create_request_handler(
        requests
            .into_iter()
            .map(|(request, response)| {
                ((String::from(request.0), String::from(request.1)), response)
            })
            .collect(),
    ));

    bindings::instantiate(harness)
}

#[test]
fn data_api_component_should_return_data_rpc_server_error() {
    let mut component = instantiate_component([(
        ("", ""),
        DataApiResult {
            status: Status::Error as i32,
            result: format_error_json(serde_json::json!({
                "message": "something went wrong"
            })),
        },
    )]);

    let interface = component
        .component
        .betty_blocks_utilities_data_api_data_api();

    let data_api_resource_def = interface.data_api();
    let data_api_resource = data_api_resource_def
        .call_constructor(&mut component.store, APPLICATION_ID, ACTION_ID, None)
        .unwrap();
    let res = data_api_resource_def.call_request(
        &mut component.store,
        data_api_resource,
        "",
        &String::from(""),
    );
    data_api_resource
        .resource_drop(&mut component.store)
        .unwrap();

    assert!(
        res.unwrap()
            .is_err_and(|err| { err == r#"{"errors":[{"message":"something went wrong"}]}"# })
    );
}

#[test]
fn data_api_component_should_return_error_if_token_is_invalid() {
    let mut component = instantiate_component([(
        ("", ""),
        DataApiResult {
            status: Status::Error as i32,
            result: format_error_unauthenticated(),
        },
    )]);

    let interface = component
        .component
        .betty_blocks_utilities_data_api_data_api();

    let data_api_resource_def = interface.data_api();
    let data_api_resource = data_api_resource_def
        .call_constructor(&mut component.store, APPLICATION_ID, ACTION_ID, None)
        .unwrap();
    let res = data_api_resource_def.call_request(
        &mut component.store,
        data_api_resource,
        "",
        &String::from(""),
    );
    data_api_resource
        .resource_drop(&mut component.store)
        .unwrap();

    assert!(res.unwrap().is_err_and(|err| {
        err == r#"{"errors":[{"extensions":{"code":"UNAUTHENTICATED"},"message":"Request not authenticated"}]}"#
    }));
}

#[test]
fn id_reserving_test() {
    let mut component = instantiate_component([
        (
            (
                "mutation ($model: String!, $amount: Int!) { reserveRecords(model: $model, amount: $amount) { ids } }",
                r#"{"model":"User","amount":2}"#,
            ),
            DataApiResult {
                status: Status::Ok as i32,
                result: format_result_json(serde_json::json!({"reserveRecords": {"ids": [4, 7]}})),
            },
        ),
        (
            (
                "mutation { upsertManyUser(input: $input, validationSets: $validationSets) { id } }",
                r#"{"input":[{"id":4},{"id":7,"subordinates":{"_replace":[{"id":4}]}}],"validationSets":"default"}"#,
            ),
            DataApiResult {
                status: Status::Ok as i32,
                result: String::new(),
            },
        ),
    ]);

    let interface = component
        .component
        .betty_blocks_utilities_data_api_data_api();

    let data_api_resource_def = interface.data_api();
    let data_api_resource = data_api_resource_def
        .call_constructor(&mut component.store, APPLICATION_ID, ACTION_ID, None)
        .unwrap();
    data_api_resource_def
        .call_start_capture(&mut component.store, data_api_resource)
        .unwrap()
        .unwrap();
    assert_eq!(data_api_resource_def.call_request(&mut component.store, data_api_resource, "mutation ($input: userInput, $validationSets: [String]) { createUser(input: $input, validationSets: $validationSets) { id } }", &String::from(r#"{"input": {}}"#)).unwrap().unwrap(), r#"{"data":{"createUser":{"id":"-1"}}}"#);
    assert_eq!(data_api_resource_def.call_request(&mut component.store, data_api_resource, "mutation ($input: userInput, $validationSets: [String]) { createUser(input: $input, validationSets: $validationSets) { id } }", &String::from(r#"{"input": {"subordinates": {"_replace": [{"id": -1}]}}}"#)).unwrap().unwrap(), r#"{"data":{"createUser":{"id":"-2"}}}"#);
    data_api_resource_def
        .call_apply_capture(&mut component.store, data_api_resource)
        .unwrap()
        .unwrap();
    data_api_resource
        .resource_drop(&mut component.store)
        .unwrap();
}

#[test]
fn pending_capture_test() {
    let mut mutation1 = PendingMutation::new(
        "createUser",
        "mutation { createUser(input: $input, validationSets: $validationSets) { id } }",
        r#"{"input":{"name":"John"},"validationSets":"default"}"#,
    );

    let mutation2 = PendingMutation::new(
        "updateUser",
        "mutation { updateUser(input: $input, validationSets: $validationSets) { id } }",
        r#"{"input":{"id":-1,"name":"Joe"},"validationSets":"default"}"#,
    );

    let mutation3 = PendingMutation::new(
        "deleteUser",
        "mutation { deleteUser(input: $input) { id } }",
        r#"{"id":-1}"#,
    );

    let mut component = instantiate_component([]);

    let interface = component
        .component
        .betty_blocks_utilities_data_api_data_api();

    let data_api_resource_def = interface.data_api();
    let data_api_resource = data_api_resource_def
        .call_constructor(&mut component.store, APPLICATION_ID, ACTION_ID, None)
        .unwrap();
    data_api_resource_def
        .call_start_capture(&mut component.store, data_api_resource)
        .unwrap()
        .unwrap();
    assert_eq!(
        data_api_resource_def
            .call_request(
                &mut component.store,
                data_api_resource,
                &mutation1.mutation,
                &mutation1.variables
            )
            .unwrap()
            .unwrap(),
        r#"{"data":{"createUser":{"id":"-1"}}}"#
    );
    assert_eq!(
        data_api_resource_def
            .call_request(
                &mut component.store,
                data_api_resource,
                &mutation2.mutation,
                &mutation2.variables
            )
            .unwrap()
            .unwrap(),
        r#"{"data":{"updateUser":{"id":"-1"}}}"#
    );
    assert_eq!(
        data_api_resource_def
            .call_request(
                &mut component.store,
                data_api_resource,
                &mutation3.mutation,
                &mutation3.variables
            )
            .unwrap()
            .unwrap(),
        r#"{"data":{"deleteUser":{"id":"-1"}}}"#
    );
    let [create_mutations, update_mutations, delete_mutations] = data_api_resource_def
        .call_pending_capture(&mut component.store, data_api_resource)
        .unwrap()
        .unwrap()
        .try_into()
        .unwrap();
    mutation1.variables =
        String::from(r#"{"input":{"id":-1,"name":"John"},"validationSets":"default"}"#);
    assert_eq!(
        TryInto::<[_; 1]>::try_into(create_mutations).unwrap(),
        [mutation1]
    );
    assert_eq!(
        TryInto::<[_; 1]>::try_into(update_mutations).unwrap(),
        [mutation2]
    );
    assert_eq!(
        TryInto::<[_; 1]>::try_into(delete_mutations).unwrap(),
        [mutation3]
    );
    data_api_resource
        .resource_drop(&mut component.store)
        .unwrap();
}

#[test]
fn discard_capture_test() {
    let mutation1 = PendingMutation::new(
        "createUser",
        "mutation { createUser(input: $input, validationSets: $validationSets) { id } }",
        r#"{"input":{"id":1,"name":"John"},"validationSets":"default"}"#,
    );

    let mut component = instantiate_component([]);

    let interface = component
        .component
        .betty_blocks_utilities_data_api_data_api();

    let data_api_resource_def = interface.data_api();
    let data_api_resource = data_api_resource_def
        .call_constructor(&mut component.store, APPLICATION_ID, ACTION_ID, None)
        .unwrap();
    data_api_resource_def
        .call_start_capture(&mut component.store, data_api_resource)
        .unwrap()
        .unwrap();
    assert_eq!(
        data_api_resource_def
            .call_request(
                &mut component.store,
                data_api_resource,
                &mutation1.mutation,
                &mutation1.variables
            )
            .unwrap()
            .unwrap(),
        r#"{"data":{"createUser":{"id":"1"}}}"#
    );
    data_api_resource_def
        .call_discard_capture(&mut component.store, data_api_resource)
        .unwrap()
        .unwrap();
    assert_eq!(
        data_api_resource_def
            .call_pending_capture(&mut component.store, data_api_resource)
            .unwrap()
            .unwrap()
            .as_slice(),
        [] as [Vec<PendingMutation>; 0]
    );
    data_api_resource_def
        .call_apply_capture(&mut component.store, data_api_resource)
        .unwrap()
        .unwrap();
    data_api_resource
        .resource_drop(&mut component.store)
        .unwrap();
}

#[test]
fn nested_capture_test() {
    let mutation2 = PendingMutation::new(
        "createUser",
        "mutation { createUser(input: $input, validationSets: $validationSets) { id } }",
        r#"{"input":{"name":"Joe","subordinates":{"_replace":[{"id":-1}]}},"validationSets":"default"}"#,
    );

    let mutation3 = PendingMutation::new(
        "createUser",
        "mutation { createUser(input: $input, validationSets: $validationSets) { id } }",
        r#"{"input":{"name":"James","subordinates":{"_replace":[{"id":-1},{"id":-2}]}},"validationSets":"default"}"#,
    );

    let mut component = instantiate_component([
        (
            (
                "mutation ($model: String!, $amount: Int!) { reserveRecords(model: $model, amount: $amount) { ids } }",
                r#"{"model":"User","amount":2}"#,
            ),
            DataApiResult {
                status: Status::Ok as i32,
                result: format_result_json(serde_json::json!({"reserveRecords": {"ids": [1, 2]}})),
            },
        ),
        (
            (
                "mutation { upsertManyUser(input: $input, validationSets: $validationSets) { id } }",
                r#"{"input":[{"id":2,"name":"Joe","subordinates":{"_replace":[{"id":1}]}}],"validationSets":"default"}"#,
            ),
            DataApiResult {
                status: Status::Ok as i32,
                result: String::new(),
            },
        ),
        (
            (
                "mutation ($model: String!, $amount: Int!) { reserveRecords(model: $model, amount: $amount) { ids } }",
                r#"{"model":"User","amount":1}"#,
            ),
            DataApiResult {
                status: Status::Ok as i32,
                result: format_result_json(serde_json::json!({"reserveRecords": {"ids": [3]}})),
            },
        ),
        (
            (
                "mutation { upsertManyUser(input: $input, validationSets: $validationSets) { id } }",
                r#"{"input":[{"id":3,"name":"James","subordinates":{"_replace":[{"id":1},{"id":2}]}}],"validationSets":"default"}"#,
            ),
            DataApiResult {
                status: Status::Ok as i32,
                result: String::new(),
            },
        ),
        (
            (
                "mutation { upsertManyUser(input: $input, validationSets: $validationSets) { id } }",
                r#"{"input":[{"id":1,"name":"John"}],"validationSets":"default"}"#,
            ),
            DataApiResult {
                status: Status::Ok as i32,
                result: String::new(),
            },
        ),
    ]);

    let interface = component
        .component
        .betty_blocks_utilities_data_api_data_api();

    let data_api_resource_def = interface.data_api();
    let data_api_resource = data_api_resource_def
        .call_constructor(&mut component.store, APPLICATION_ID, ACTION_ID, None)
        .unwrap();
    data_api_resource_def
        .call_start_capture(&mut component.store, data_api_resource)
        .unwrap()
        .unwrap();
    assert_eq!(
        data_api_resource_def
            .call_request(
                &mut component.store,
                data_api_resource,
                "mutation { createUser(input: $input, validationSets: $validationSets) { id } }",
                &String::from(r#"{"input":{"name":"John"},"validationSets":"default"}"#)
            )
            .unwrap()
            .unwrap(),
        r#"{"data":{"createUser":{"id":"-1"}}}"#
    );
    data_api_resource_def
        .call_start_capture(&mut component.store, data_api_resource)
        .unwrap()
        .unwrap();
    assert_eq!(
        data_api_resource_def
            .call_request(
                &mut component.store,
                data_api_resource,
                &mutation2.mutation,
                &mutation2.variables
            )
            .unwrap()
            .unwrap(),
        r#"{"data":{"createUser":{"id":"-2"}}}"#
    );
    data_api_resource_def
        .call_apply_capture(&mut component.store, data_api_resource)
        .unwrap()
        .unwrap();
    data_api_resource_def
        .call_start_capture(&mut component.store, data_api_resource)
        .unwrap()
        .unwrap();
    assert_eq!(
        data_api_resource_def
            .call_request(
                &mut component.store,
                data_api_resource,
                &mutation3.mutation,
                &mutation3.variables
            )
            .unwrap()
            .unwrap(),
        r#"{"data":{"createUser":{"id":"-3"}}}"#
    );
    data_api_resource_def
        .call_apply_capture(&mut component.store, data_api_resource)
        .unwrap()
        .unwrap();
    data_api_resource_def
        .call_apply_capture(&mut component.store, data_api_resource)
        .unwrap()
        .unwrap();
    data_api_resource
        .resource_drop(&mut component.store)
        .unwrap();
}
