mod bindings {
    wasmtime_testing_helper::bindgen!({ path: "wit", world: "provider", additional_derives: [PartialEq] });

    wasmtime_testing_helper::setup!(Provider);
}

use jaws_rs::Keys;
use prost::Message;
use prost::bytes::Buf;
use std::collections::VecDeque;
use wasmtime_testing_helper::http::{HttpHandler, hyper};
use wasmtime_testing_helper::wasmtime::Store;
use wasmtime_testing_helper::wasmtime::component::ResourceAny;
use wasmtime_testing_helper::{ComponentState, InstantiatedComponent};
#[allow(clippy::result_large_err)]
pub mod data_grpc {
    tonic::include_proto!("data_grpc");
}

use data_grpc::DataApiRequest;
use data_grpc::{DataApiResult, data_api_result::Status};

use crate::bindings::Provider;
use crate::bindings::exports::betty_blocks_types::data_api::data_api::{
    GuestDataApi, PendingMutation,
};

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

struct DataApiResource<'a> {
    resource_def: GuestDataApi<'a>,
    resource: ResourceAny,
    store: &'a mut Store<ComponentState>,
}

impl<'a> DataApiResource<'a> {
    fn new(component: &'a mut InstantiatedComponent<Provider>) -> Self {
        let interface = component.component.betty_blocks_types_data_api_data_api();

        let data_api_resource_def = interface.data_api();
        let data_api_resource = data_api_resource_def
            .call_constructor(&mut component.store, APPLICATION_ID, ACTION_ID, None)
            .unwrap();

        Self {
            resource_def: data_api_resource_def,
            resource: data_api_resource,
            store: &mut component.store,
        }
    }
}

impl<'a> Drop for DataApiResource<'a> {
    fn drop(&mut self) {
        self.resource.resource_drop(&mut self.store).unwrap();
    }
}

macro_rules! call_data_api_resource {
    ($resource:ident.$function:ident($($param:expr),*)) => {
        $resource.resource_def.$function(&mut $resource.store, $resource.resource, $($param),*).unwrap()
    };
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
            assert!(matches!(
            new_req,
            DataApiRequest {
                context: Some(
                    data_grpc::Context {
                        application_id, jwt
                    }
                ),
                ..}
                if application_id.as_str() == APPLICATION_ID
                && jwt.as_str() == JWT
            ));
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

    let mut resource = DataApiResource::new(&mut component);

    assert!(
        call_data_api_resource!(resource.call_request("", &String::from("")))
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

    let mut resource = DataApiResource::new(&mut component);

    assert!(
        call_data_api_resource!(
            resource.call_request("", &String::from(""))
        ).is_err_and(|err| {
            err == r#"{"errors":[{"extensions":{"code":"UNAUTHENTICATED"},"message":"Request not authenticated"}]}"#
        })
    );
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
                r#"{"input":[{"id":4},{"id":7,"subordinates":{"_replace":[{"id":4}]}}],"validationSets":["default"]}"#,
            ),
            DataApiResult {
                status: Status::Ok as i32,
                result: String::new(),
            },
        ),
    ]);

    let mut resource = DataApiResource::new(&mut component);

    call_data_api_resource!(resource.call_start_capture()).unwrap();

    assert_eq!(
        call_data_api_resource!(resource.call_request(
            "mutation ($input: userInput, $validationSets: [String]) { createUser(input: $input, validationSets: $validationSets) { id } }",
            &String::from(r#"{"input": {}}"#)
        )).unwrap(),
        r#"{"data":{"createUser":{"id":"-1"}}}"#
    );

    assert_eq!(
        call_data_api_resource!(resource.call_request(
            "mutation ($input: userInput, $validationSets: [String]) { createUser(input: $input, validationSets: $validationSets) { id } }",
            &String::from(r#"{"input": {"subordinates": {"_replace": [{"id": -1}]}}}"#)
        )).unwrap(),
        r#"{"data":{"createUser":{"id":"-2"}}}"#
    );

    call_data_api_resource!(resource.call_apply_capture()).unwrap();
}

#[test]
fn pending_capture_test() {
    let mut mutation1 = PendingMutation::new(
        "createUser",
        "mutation { createUser(input: $input, validationSets: $validationSets) { id } }",
        r#"{"input":{"name":"John"},"validationSets":["default"]}"#,
    );

    let mutation2 = PendingMutation::new(
        "updateUser",
        "mutation { updateUser(input: $input, validationSets: $validationSets) { id } }",
        r#"{"input":{"id":-1,"name":"Joe"},"validationSets":["default"]}"#,
    );

    let mutation3 = PendingMutation::new(
        "deleteUser",
        "mutation { deleteUser(input: $input) { id } }",
        r#"{"id":-1}"#,
    );

    let mut component = instantiate_component([]);

    let mut resource = DataApiResource::new(&mut component);

    call_data_api_resource!(resource.call_start_capture()).unwrap();

    assert_eq!(
        call_data_api_resource!(resource.call_request(&mutation1.mutation, &mutation1.variables))
            .unwrap(),
        r#"{"data":{"createUser":{"id":"-1"}}}"#
    );

    assert_eq!(
        call_data_api_resource!(resource.call_request(&mutation2.mutation, &mutation2.variables))
            .unwrap(),
        r#"{"data":{"updateUser":{"id":"-1"}}}"#
    );

    assert_eq!(
        call_data_api_resource!(resource.call_request(&mutation3.mutation, &mutation3.variables))
            .unwrap(),
        r#"{"data":{"deleteUser":{"id":"-1"}}}"#
    );

    let [create_mutations, update_mutations, delete_mutations] =
        call_data_api_resource!(resource.call_pending_capture())
            .unwrap()
            .try_into()
            .unwrap();
    mutation1.variables =
        String::from(r#"{"input":{"id":-1,"name":"John"},"validationSets":["default"]}"#);
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
}

#[test]
fn discard_capture_test() {
    let mutation1 = PendingMutation::new(
        "createUser",
        "mutation { createUser(input: $input, validationSets: $validationSets) { id } }",
        r#"{"input":{"id":1,"name":"John"},"validationSets":["default"]}"#,
    );

    let mut component = instantiate_component([]);

    let mut resource = DataApiResource::new(&mut component);

    call_data_api_resource!(resource.call_start_capture()).unwrap();

    assert_eq!(
        call_data_api_resource!(resource.call_request(&mutation1.mutation, &mutation1.variables))
            .unwrap(),
        r#"{"data":{"createUser":{"id":"1"}}}"#
    );

    call_data_api_resource!(resource.call_discard_capture()).unwrap();

    assert!(
        call_data_api_resource!(resource.call_pending_capture())
            .unwrap()
            .is_empty()
    );

    call_data_api_resource!(resource.call_apply_capture()).unwrap();
}

#[test]
fn nested_capture_test() {
    let mutation1 = PendingMutation::new(
        "createUser",
        "mutation { createUser(input: $input, validationSets: $validationSets) { id } }",
        r#"{"input":{"name":"John"},"validationSets":["default"]}"#,
    );

    let mutation2 = PendingMutation::new(
        "createUser",
        "mutation { createUser(input: $input, validationSets: $validationSets) { id } }",
        r#"{"input":{"name":"Joe","subordinates":{"_replace":[{"id":-1}]}},"validationSets":["default"]}"#,
    );

    let mutation3 = PendingMutation::new(
        "createUser",
        "mutation { createUser(input: $input, validationSets: $validationSets) { id } }",
        r#"{"input":{"name":"James","subordinates":{"_replace":[{"id":-1},{"id":-2}]}},"validationSets":["default"]}"#,
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
                r#"{"input":[{"id":2,"name":"Joe","subordinates":{"_replace":[{"id":1}]}}],"validationSets":["default"]}"#,
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
                r#"{"input":[{"id":3,"name":"James","subordinates":{"_replace":[{"id":1},{"id":2}]}}],"validationSets":["default"]}"#,
            ),
            DataApiResult {
                status: Status::Ok as i32,
                result: String::new(),
            },
        ),
        (
            (
                "mutation { upsertManyUser(input: $input, validationSets: $validationSets) { id } }",
                r#"{"input":[{"id":1,"name":"John"}],"validationSets":["default"]}"#,
            ),
            DataApiResult {
                status: Status::Ok as i32,
                result: String::new(),
            },
        ),
    ]);

    let mut resource = DataApiResource::new(&mut component);

    call_data_api_resource!(resource.call_start_capture()).unwrap();

    assert_eq!(
        call_data_api_resource!(resource.call_request(&mutation1.mutation, &mutation1.variables))
            .unwrap(),
        r#"{"data":{"createUser":{"id":"-1"}}}"#
    );

    call_data_api_resource!(resource.call_start_capture()).unwrap();

    assert_eq!(
        call_data_api_resource!(resource.call_request(&mutation2.mutation, &mutation2.variables))
            .unwrap(),
        r#"{"data":{"createUser":{"id":"-2"}}}"#
    );

    call_data_api_resource!(resource.call_apply_capture()).unwrap();

    call_data_api_resource!(resource.call_start_capture()).unwrap();

    assert_eq!(
        call_data_api_resource!(resource.call_request(&mutation3.mutation, &mutation3.variables))
            .unwrap(),
        r#"{"data":{"createUser":{"id":"-3"}}}"#
    );

    call_data_api_resource!(resource.call_apply_capture()).unwrap();

    call_data_api_resource!(resource.call_apply_capture()).unwrap();
}

/// A request body larger than the outgoing body buffer of the host must not hang.
///
/// wasmtime's `wasi:http` outgoing body only buffers two writes before the
/// request is handed to the host, so a client that writes the whole body in
/// 4 KiB chunks before calling `outgoing-handler.handle` deadlocks on anything
/// bigger than 4 KiB (PHOEN-1806).
#[test]
fn large_request_body_test() {
    let query = "query { allPokemon { totalCount } }";
    let variables = format!(r#"{{"padding":"{}"}}"#, "x".repeat(16 * 1024));

    let mut component = instantiate_component([(
        (query, variables.as_str()),
        DataApiResult {
            status: Status::Ok as i32,
            result: format_result_json(serde_json::json!({"allPokemon": {"totalCount": 1}})),
        },
    )]);

    let mut resource = DataApiResource::new(&mut component);

    assert_eq!(
        call_data_api_resource!(resource.call_request(query, &variables)).unwrap(),
        r#"{"data":{"allPokemon":{"totalCount":1}}}"#
    );
}

/// Applying a capture with many records sends one `upsertMany` whose body is far
/// larger than 4 KiB; this mirrors a mass mutate around a loop of 100 creates.
#[test]
fn apply_capture_with_many_records_test() {
    const RECORDS: usize = 100;
    let payload = "p".repeat(100);
    let reserved_ids: Vec<i32> = (1..=RECORDS as i32).map(|i| 1000 + i).collect();

    let upsert_input: Vec<serde_json::Value> = reserved_ids
        .iter()
        .enumerate()
        .map(|(i, id)| serde_json::json!({"id": id, "name": format!("pokemon {i}"), "payload": payload}))
        .collect();
    let upsert_variables =
        serde_json::json!({"input": upsert_input, "validationSets": ["default"]}).to_string();
    assert!(upsert_variables.len() > 4 * 1024);

    let mut component = instantiate_component([
        (
            (
                "mutation ($model: String!, $amount: Int!) { reserveRecords(model: $model, amount: $amount) { ids } }",
                r#"{"model":"Pokemon","amount":100}"#,
            ),
            DataApiResult {
                status: Status::Ok as i32,
                result: format_result_json(
                    serde_json::json!({"reserveRecords": {"ids": reserved_ids}}),
                ),
            },
        ),
        (
            (
                "mutation { upsertManyPokemon(input: $input, validationSets: $validationSets) { id } }",
                upsert_variables.as_str(),
            ),
            DataApiResult {
                status: Status::Ok as i32,
                result: String::new(),
            },
        ),
    ]);

    let mut resource = DataApiResource::new(&mut component);

    call_data_api_resource!(resource.call_start_capture()).unwrap();

    for i in 0..RECORDS {
        let variables =
            serde_json::json!({"input": {"name": format!("pokemon {i}"), "payload": payload}})
                .to_string();
        assert_eq!(
            call_data_api_resource!(resource.call_request(
                "mutation ($input: PokemonInput, $validationSets: [String]) { createPokemon(input: $input, validationSets: $validationSets) { id } }",
                &variables
            ))
            .unwrap(),
            format!(r#"{{"data":{{"createPokemon":{{"id":"-{}"}}}}}}"#, i + 1)
        );
    }

    call_data_api_resource!(resource.call_apply_capture()).unwrap();
}
