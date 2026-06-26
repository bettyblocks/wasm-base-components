use std::collections::{HashMap, VecDeque};
use std::fmt::Display;
use std::sync::{Arc, Mutex};

use crate::exports::betty_blocks_utilities::data_api::data_api::{self, GuestDataApi, JsonString};
use crate::{Config, inner_request};

type ModelName = String;
type InternalId = isize;
type RealId = i32;

const CHUNK_SIZE: usize = 100_000;

const RESERVE_ID_QUERY: &str = "mutation ($model: String!, $amount: Int!) { reserveRecords(model: $model, amount: $amount) { ids } }";

pub struct DataAPIContext {
    application_id: String,
    #[expect(dead_code, reason = "Will be used for logging in the future")]
    action_id: String,
    /// jwt of the customer (so not the jaws jwt used for authenticating the server to server communication)
    jwt: Option<String>,
    capture_data: Arc<Mutex<CaptureData>>,
}

#[derive(Debug, Default)]
pub struct CaptureData {
    model_names_of_local_ids: Vec<ModelName>,
    reserve_id_count_per_model: HashMap<ModelName, u32>,
    reserved_ids: Vec<RealId>,
    capture_stack: Vec<MassMutateEntries>,
}

impl CaptureData {
    fn reserve_ids(&mut self, request_sender: &impl RequestRaw) -> Result<Vec<RealId>, String> {
        let mut reserved_id_map = self
            .reserve_id_count_per_model
            .drain()
            .map(|entry| Self::request_reserved_ids(request_sender, entry))
            .collect::<Result<HashMap<ModelName, VecDeque<u32>>, String>>()?;

        for model_name in self.model_names_of_local_ids.drain(..) {
            let reserved_ids_for_model = reserved_id_map
                .get_mut(&model_name)
                .ok_or_else(|| format!("ids for model {model_name} were not properly reserved"))?
                .pop_front()
                .ok_or_else(|| format!("not enough ids were reserved for model {model_name}"))?
                as RealId;

            self.reserved_ids.push(reserved_ids_for_model);
        }

        if self.capture_stack.is_empty() {
            Ok(std::mem::take(&mut self.reserved_ids))
        } else {
            Ok(self.reserved_ids.clone())
        }
    }

    fn request_reserved_ids(
        request_sender: &impl RequestRaw,
        (model_name, id_count): (ModelName, u32),
    ) -> Result<(ModelName, VecDeque<u32>), String> {
        let variables = format!(r#"{{"model": "{model_name}", "amount": {id_count}}}"#);

        let res = request_sender.request_raw(RESERVE_ID_QUERY.to_string(), variables)?;

        let ReserveIdMutationResult {
            data: ReserveIdResult {
                reserved_ids: ReservedIds { ids },
            },
        } = serde_json::from_str(&res)
            .map_err(|_| String::from("could not parse data api result for reserving ids"))?;

        Ok((model_name, ids))
    }
}

#[derive(Default, Debug)]
pub struct MassMutateEntries {
    create: Vec<MassMutateEntry<MutationInput>>,
    update: Vec<MassMutateEntry<MutationInput>>,
    delete: Vec<MassMutateEntry<DeleteInput>>,
}

impl MassMutateEntries {
    fn as_pending_mutations(&self) -> [Vec<data_api::PendingMutation>; 3] {
        [
            self.create
                .iter()
                .map(|x| x.as_pending_mutation("create"))
                .collect(),
            self.update
                .iter()
                .map(|x| x.as_pending_mutation("update"))
                .collect(),
            self.delete
                .iter()
                .map(|x| x.as_pending_mutation("delete"))
                .collect(),
        ]
    }
}

#[derive(Debug)]
pub struct MassMutateEntry<T: serde::Serialize> {
    model_name: String,
    variables: T,
}

impl<T: serde::Serialize> MassMutateEntry<T> {
    fn as_pending_mutation(&self, operation: &str) -> data_api::PendingMutation {
        data_api::PendingMutation {
            mutation_name: format_mutation_name(&self.model_name, operation),
            mutation: format_mutation(&self.model_name, operation),
            variables: serde_json::to_string(&self.variables).expect("incorrect variables"),
        }
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
struct ReserveIdMutationResult {
    data: ReserveIdResult,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct ReserveIdResult {
    #[serde(rename = "reserveRecords")]
    reserved_ids: ReservedIds,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct ReservedIds {
    ids: VecDeque<u32>,
}

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone)]
struct MutationInput {
    input: MutationInputVariable,
    #[serde(rename = "validationSets")]
    validation_sets: Option<ValidationSets>,
}

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone)]
struct MutationInputVariable {
    id: InternalId,
    #[serde(flatten)]
    other_inputs: serde_json::Map<String, serde_json::Value>,
}

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone)]
struct DeleteInput {
    id: InternalId,
}

#[derive(Default, serde::Serialize, serde::Deserialize, Debug, Clone)]
#[serde(untagged)]
enum ValidationSets {
    Empty,
    #[default]
    Default,
}

impl ValidationSets {
    fn as_str(&self) -> &'static str {
        match self {
            ValidationSets::Default => "default",
            ValidationSets::Empty => "empty",
        }
    }
}

impl Display for ValidationSets {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

fn format_mutation(model_name: &str, operation: &str) -> String {
    format!(
        "mutation {{ {}{model_name}(input: $input) {{ id }} }}",
        operation
    )
}

fn format_mutation_name(model_name: &str, operation: &str) -> String {
    format!("{}{model_name}", operation)
}

fn generate_delayed_id_response(id: &str, mutation_name: &str) -> JsonString {
    format!(r#"{{"data": {{"{mutation_name}": {{"id": "{id}"}}}}}}"#)
}

fn generate_upsert_many_inputs(
    create_entries: Vec<MassMutateEntry<MutationInput>>,
    update_entries: Vec<MassMutateEntry<MutationInput>>,
    reserved_ids: &[RealId],
) -> Result<HashMap<String, (ValidationSets, Vec<MutationInputVariable>)>, String> {
    let mut upsert_manys: HashMap<String, (ValidationSets, Vec<_>)> = HashMap::new();

    for MassMutateEntry {
        model_name,
        mut variables,
    } in [create_entries, update_entries].into_iter().flatten()
    {
        replace_negative_ids_in_mutation_input(reserved_ids, &mut variables.input)?;

        let (validation_sets, input_vec) = upsert_manys.entry(model_name).or_default();

        if let ValidationSets::Default = validation_sets
            && let Some(ValidationSets::Empty) = variables.validation_sets
        {
            *validation_sets = ValidationSets::Empty
        }

        input_vec.push(variables.input);
    }

    Ok(upsert_manys)
}

fn generate_delete_many_inputs(
    delete_entries: Vec<MassMutateEntry<DeleteInput>>,
    reserved_ids: &[RealId],
) -> Result<HashMap<String, Vec<InternalId>>, String> {
    let mut delete_ids = HashMap::new();

    for MassMutateEntry {
        model_name,
        variables: DeleteInput { mut id },
    } in delete_entries
    {
        replace_id(reserved_ids, &mut id)?;

        delete_ids.entry(model_name).or_insert(Vec::new()).push(id);
    }

    Ok(delete_ids)
}

fn send_upsert_many_mutations(
    upsert_many_inputs: HashMap<String, (ValidationSets, Vec<MutationInputVariable>)>,
    request_sender: &impl RequestRaw,
) -> Result<(), String> {
    for (model_name, (validation_sets, input_variables)) in upsert_many_inputs {
        let query = format!(
            "mutation {{ upsertMany{model_name}(input: $input, validationSets: $validationSets) {{ id }} }}"
        );

        for input_chunk in input_variables.chunks(CHUNK_SIZE) {
            let variables = format!(
                "{{\"input\": {}, \"validationSets\": \"{validation_sets}\"}}",
                serde_json::to_string(&input_chunk)
                    .map_err(|_| String::from("could not format input variables"))?
            );

            request_sender.request_raw(query.clone(), variables)?;
        }
    }

    Ok(())
}

fn send_delete_many_mutations(
    delete_many_inputs: HashMap<String, Vec<isize>>,
    request_sender: &impl RequestRaw,
) -> Result<(), String> {
    for (model_name, variables) in delete_many_inputs {
        let query = format!("mutation {{ deleteMany{model_name}(input: $input) {{ id }} }}");

        for input_chunk in variables.chunks(CHUNK_SIZE) {
            let variables = format!(
                "{{\"input\": {{\"ids\": {}}}}}",
                serde_json::to_string(&input_chunk)
                    .map_err(|_| String::from("could not format input variables"))?
            );

            request_sender.request_raw(query.clone(), variables)?;
        }
    }

    Ok(())
}

trait RequestRaw {
    fn request_raw(&self, query: String, variables: JsonString) -> Result<JsonString, String>;
}

impl<T: GuestDataApi> RequestRaw for T {
    fn request_raw(&self, query: String, variables: JsonString) -> Result<JsonString, String> {
        GuestDataApi::request_raw(self, query, variables)
    }
}

impl GuestDataApi for DataAPIContext {
    fn new(application_id: String, action_id: String, jwt: Option<String>) -> Self {
        DataAPIContext {
            application_id,
            action_id,
            jwt,
            capture_data: Default::default(),
        }
    }

    fn apply_capture(&self) -> Result<(), String> {
        let mut capture_data = self
            .capture_data
            .lock()
            .map_err(|_| String::from("capture stack lock poisoned"))?;
        if let Some(MassMutateEntries {
            create,
            update,
            delete,
        }) = capture_data.capture_stack.pop()
        {
            let reserved_ids = capture_data.reserve_ids(self)?;

            /*
            Locally generated ids need to be distinguishable from normal ids after being sent back to the caller and then sent here as for example a related row.
            This will probably be done with negative numbered ids? Unless we have a better way of distinguising them.
            Preferrably the caller wouldn't be able to distinguish them.

            Here, we need to go through all reserved ids.
            In theory the relations would still work if we only reserve ids that are referenced as relations by other mutations, and set the rest to default.
            However, that would mess with the order of the inserted records.
            I believe it would be better to preserve the insert order of the action, but it is an optimization worth considering.
            These need to be reserved by a query, and we need to map them to the locally generated ids.
            Then we need to replace all locally generated ids with the real ids.
            Then we can upsert all creates, upsert all updates, upsert all upserts and delete all deletes.
            These can be clustered by model, so we can do upsertManyUser but not upsertManyUserAndRole.
            */

            send_upsert_many_mutations(
                generate_upsert_many_inputs(create, update, &reserved_ids)?,
                self,
            )?;

            send_delete_many_mutations(generate_delete_many_inputs(delete, &reserved_ids)?, self)?;
        }

        Ok(())
    }

    fn discard_capture(&self) -> Result<String, String> {
        if self
            .capture_data
            .lock()
            .expect("lock is poisoned")
            .capture_stack
            .pop()
            .is_some()
        {
            return Ok(String::from("deleted most recent capture stack entry"));
        }

        Ok(String::from("nothing to do"))
    }

    fn start_capture(&self) -> Result<(), String> {
        self.capture_data
            .lock()
            .map_err(|_| String::from("capture stack lock poisoned"))?
            .capture_stack
            .push(Default::default());
        Ok(())
    }

    fn pending_capture(&self) -> Result<Vec<Vec<data_api::PendingMutation>>, String> {
        if let Some(entries) = self
            .capture_data
            .lock()
            .expect("lock is poisoned")
            .capture_stack
            .last()
        {
            return Ok(entries.as_pending_mutations().to_vec());
        }

        Ok(Vec::new())
    }

    fn request(&self, query: String, variables: JsonString) -> Result<JsonString, String> {
        if let Ok(mut capture_data) = self.capture_data.lock()
            && !capture_data.capture_stack.is_empty()
        {
            extract_mutation_data(self, &mut capture_data, query, variables)
        } else {
            RequestRaw::request_raw(self, query, variables)
        }
    }

    fn request_raw(&self, query: String, variables: JsonString) -> Result<JsonString, String> {
        let config = match Config::from_env() {
            Ok(config) => config,
            Err(e) => return Err(format!("Configuration error: {e:#}")),
        };

        let runtime = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(rt) => rt,
            Err(e) => return Err(format!("failed to create tokio runtime: {e}")),
        };

        runtime
            .block_on(inner_request(
                config,
                &self.application_id,
                self.jwt.clone(),
                query,
                variables,
            ))
            .map_err(|e| format!("{e:#}"))
    }
}

fn extract_mutation_data(
    request_sender: &impl RequestRaw,
    capture_data: &mut CaptureData,
    query: String,
    variables: JsonString,
) -> Result<JsonString, String> {
    let query_remainder = match query.split_once("mutation") {
        Some((_, remainder)) => remainder,
        None => return request_sender.request_raw(query, variables),
    };

    let query_remainder = query_remainder
        .split_once('{')
        .ok_or_else(|| String::from("query is improperly formatted"))?
        .1
        .trim();

    if query_remainder[6..10] == *"Many" {
        return request_sender.request_raw(query, variables);
    }

    let (mutation_name_with_whitespace, _) = query_remainder
        .split_once('(')
        .ok_or_else(|| String::from("query is improperly formatted"))?;

    let mutation_name = mutation_name_with_whitespace.trim();

    let model_name = mutation_name[6..].to_string();

    // It is safe to unwrap the capture stack last_mut here because we checked the length to be non-zero before. This looks a bit wack but is necessary to not mutably borrow capture_data multiple times.
    let id: isize = match &mutation_name[..6] {
        "create" => match serde_json::from_str(&variables) {
            Ok(
                variables @ MutationInput {
                    input: MutationInputVariable { id, .. },
                    ..
                },
            ) => {
                capture_data
                    .capture_stack
                    .last_mut()
                    .unwrap()
                    .create
                    .push(MassMutateEntry {
                        model_name,
                        variables,
                    });

                id
            }
            Err(_) => {
                capture_data
                    .model_names_of_local_ids
                    .push(model_name.clone());
                *capture_data
                    .reserve_id_count_per_model
                    .entry(model_name.clone())
                    .or_default() += 1;

                let capture = capture_data.capture_stack.last_mut().unwrap();

                let internal_id = -1
                    - TryInto::<isize>::try_into(capture.create.len())
                        .map_err(|_| String::from("ran out of internal ids"))?;

                let mut variables: serde_json::Value = serde_json::from_str(&variables)
                    .map_err(|_| String::from("could not parse variables"))?;

                variables
                    .get_mut("input")
                    .ok_or_else(|| String::from("create mutation was missing input"))?
                    .as_object_mut()
                    .ok_or_else(|| String::from("create mutation input was not an object"))?
                    .insert(
                        String::from("id"),
                        serde_json::Value::Number(
                            serde_json::Number::from_i128(internal_id as i128).unwrap(),
                        ),
                    );

                capture.create.push(MassMutateEntry {
                    model_name,
                    variables: serde_json::from_value(variables)
                        .map_err(|_| String::from("mutation variables are improperly formatted"))?,
                });

                internal_id
            }
        },
        "update" => {
            let variables @ MutationInput {
                input: MutationInputVariable { id, .. },
                ..
            } = serde_json::from_str(&variables)
                .map_err(|_| String::from("could not find id input for update query"))?;

            capture_data
                .capture_stack
                .last_mut()
                .unwrap()
                .update
                .push(MassMutateEntry {
                    model_name,
                    variables,
                });

            id
        }
        "delete" => {
            let variables @ DeleteInput { id } = serde_json::from_str(&variables)
                .map_err(|_| String::from("could not find id input for delete query"))?;

            capture_data
                .capture_stack
                .last_mut()
                .unwrap()
                .delete
                .push(MassMutateEntry {
                    model_name,
                    variables,
                });

            id
        }
        _ => return request_sender.request_raw(query, variables),
    };

    Ok(generate_delayed_id_response(&id.to_string(), mutation_name))
}

fn replace_negative_ids_in_mutation_input(
    reserved_ids: &[RealId],
    MutationInputVariable { id, other_inputs }: &mut MutationInputVariable,
) -> Result<(), String> {
    replace_id(reserved_ids, id)?;

    replace_negative_ids_in_object(reserved_ids, other_inputs)?;

    Ok(())
}

fn replace_id(reserved_ids: &[RealId], id: &mut isize) -> Result<(), String> {
    if id.is_negative() {
        let index: usize = (-1 - *id)
            .try_into()
            .map_err(|error| format!("could not convert number to usize: {error}"))?;

        *id = reserved_ids[index]
            .try_into()
            .map_err(|error| format!("could not convert number to isize: {error}"))?;
    }

    Ok(())
}

/// Loops through the entries of a serde_json::Map and replaces all the negative IDs with their
/// reserved counterparts.
fn replace_negative_ids_in_object(
    reserved_ids: &[RealId],
    variables: &mut serde_json::Map<String, serde_json::Value>,
) -> Result<(), String> {
    for (key, value) in variables.iter_mut() {
        if key == "id" {
            handle_id_key(reserved_ids, value)?;
        } else if let serde_json::Value::Object(object) = value {
            replace_negative_ids_in_object(reserved_ids, object)?;
        } else if let serde_json::Value::Array(array_of_values) = value {
            handle_array_of_values(reserved_ids, array_of_values)?;
        }
    }

    Ok(())
}

/// Replaces the negative ID or IDs with their reserved counterpart.
fn handle_id_key(reserved_ids: &[RealId], value: &mut serde_json::Value) -> Result<(), String> {
    if let Some(id) = value.as_i64()
        && id.is_negative()
    {
        *value = get_reserved_id_as_value_for_negative_id(reserved_ids, id)?;
    }

    Ok(())
}

/// Uses the negative_id to index for its reserved id and put it into a serde_json::Value.
fn get_reserved_id_as_value_for_negative_id(
    reserved_ids: &[RealId],
    negative_id: i64,
) -> Result<serde_json::Value, String> {
    // If `negative_id` is -1 which is the first possible negative ID, then `-1 - -1 = 0`.
    // Which is the first possible item in the reserved ID list.
    let index: usize = (-1 - negative_id)
        .try_into()
        .map_err(|error| format!("could not convert number to usize: {error}"))?;

    Ok(serde_json::Value::Number(serde_json::Number::from(
        reserved_ids[index],
    )))
}

/// Looks at the items in an array and if they're an object tries to replace negative IDs in that
/// object, and if they are an array it searches for objects or arrays within that object which
/// might have negative IDs to replace.
fn handle_array_of_values(
    reserved_ids: &[RealId],
    array_of_values: &mut [serde_json::Value],
) -> Result<(), String> {
    for item in array_of_values.iter_mut() {
        if let serde_json::Value::Object(object) = item {
            replace_negative_ids_in_object(reserved_ids, object)?;
        } else if let serde_json::Value::Array(array) = item {
            handle_array_of_values(reserved_ids, array)?;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::*;

    impl Default for DataAPIContext {
        fn default() -> Self {
            DataAPIContext::new(Default::default(), Default::default(), Default::default())
        }
    }

    struct RequestRawMock {
        should_expect: bool,
        expected_requests: RefCell<VecDeque<(String, JsonString)>>,
        responses: RefCell<VecDeque<Result<JsonString, String>>>,
    }

    impl Default for RequestRawMock {
        fn default() -> Self {
            Self::without_expect()
        }
    }

    impl RequestRawMock {
        fn without_expect() -> Self {
            Self {
                should_expect: false,
                expected_requests: Default::default(),
                responses: Default::default(),
            }
        }

        fn with_expect(
            expected_requests: VecDeque<(String, JsonString)>,
            responses: VecDeque<Result<JsonString, String>>,
        ) -> Self {
            Self {
                should_expect: true,
                expected_requests: RefCell::new(expected_requests),
                responses: RefCell::new(responses),
            }
        }
    }

    impl RequestRaw for RequestRawMock {
        fn request_raw(&self, query: String, variables: JsonString) -> Result<JsonString, String> {
            if self.should_expect {
                if let Some((expected_query, expected_variables)) =
                    self.expected_requests.borrow_mut().pop_front()
                {
                    if expected_query != query || expected_variables != variables {
                        panic!(
                            "Unexpected request_raw call.\nExpected:\n{expected_query}\nwith variables:\n{expected_variables}\nGot:\n{query}\nwith variables:\n{variables}"
                        );
                    }
                } else {
                    panic!(
                        "Unexpected request_raw call.\nExpected nothing\nGot:\n{query}\nwith variables:\n{variables}"
                    );
                }
            }

            self.responses
                .borrow_mut()
                .pop_front()
                .ok_or_else(|| String::from("Default mock response"))
                .flatten()
        }
    }

    #[test]
    fn ignore_unsupported_queries_test() {
        let mut capture_data = CaptureData::default();
        capture_data.capture_stack.push(Default::default());

        let request_raw_mock = RequestRawMock::default();

        assert_eq!(
            extract_mutation_data(
                &request_raw_mock,
                &mut capture_data,
                String::from("query {createUser(input: $input) {id}}"),
                String::from("irrelevant")
            )
            .unwrap_err()
            .as_str(),
            "Default mock response"
        );

        assert_eq!(
            extract_mutation_data(
                &request_raw_mock,
                &mut capture_data,
                String::from(
                    "mutation {createManyUser(input: $input, validationSets: $validationSets) {id}}"
                ),
                String::from("irrelevant")
            )
            .unwrap_err()
            .as_str(),
            "Default mock response"
        );

        assert_eq!(
            extract_mutation_data(
                &request_raw_mock,
                &mut capture_data,
                String::from("mutation {reserveRecords(input: $input) {id}}"),
                String::from("irrelevant")
            )
            .unwrap_err()
            .as_str(),
            "Default mock response"
        );

        let MassMutateEntries {
            create,
            update,
            delete,
        } = capture_data.capture_stack.pop().unwrap();

        assert!(capture_data.reserved_ids.is_empty());
        assert!(capture_data.model_names_of_local_ids.is_empty());
        assert!(capture_data.reserve_id_count_per_model.is_empty());

        assert!(create.is_empty());
        assert!(update.is_empty());
        assert!(delete.is_empty());
    }

    #[test]
    fn real_id_extraction_test() {
        let mut capture_data = CaptureData::default();
        capture_data.capture_stack.push(Default::default());

        let request_raw_mock = RequestRawMock::with_expect(Default::default(), Default::default());

        assert_eq!(
            extract_mutation_data(
                &request_raw_mock,
                &mut capture_data,
                String::from(
                    r#"
    mutation ($input: userInput, $validationSets: [String]) {
        createUser(input: $input, validationSets: $validationSets) {
            id
        }
    }"#
                ),
                String::from(r#"{"input": {"id": 2}}"#)
            )
            .unwrap()
            .as_str(),
            r#"{"data": {"createUser": {"id": "2"}}}"#
        );

        assert_eq!(
            extract_mutation_data(
                &request_raw_mock,
                &mut capture_data,
                String::from(
                    r#"
    mutation ($input: userInput, $validationSets: [String]) {
        deleteUser(input: $input, validationSets: $validationSets) {
            id
        }
    }"#
                ),
                String::from(r#"{"id": 2}"#)
            )
            .unwrap()
            .as_str(),
            r#"{"data": {"deleteUser": {"id": "2"}}}"#
        );

        let MassMutateEntries {
            mut create,
            update,
            mut delete,
        } = capture_data.capture_stack.pop().unwrap();

        assert!(capture_data.reserved_ids.is_empty());
        assert!(capture_data.model_names_of_local_ids.is_empty());
        assert!(capture_data.reserve_id_count_per_model.is_empty());

        let MassMutateEntry {
            model_name: create_model_name,
            variables: create_variables,
        } = create.pop().unwrap();
        assert_eq!(create_model_name.as_str(), "User");
        assert!(
            matches!(create_variables, MutationInput { input: MutationInputVariable { id: 1, other_inputs }, validation_sets: None } if other_inputs.is_empty())
        );
        let MassMutateEntry {
            model_name: delete_model_name,
            variables: delete_variables,
        } = delete.pop().unwrap();
        assert_eq!(delete_model_name.as_str(), "User");
        assert!(matches!(delete_variables, DeleteInput { id: 2 }));
        assert!(update.is_empty());
    }

    #[test]
    fn internal_id_generation_test() {
        let mut capture_data = CaptureData::default();
        capture_data.capture_stack.push(Default::default());

        let request_raw_mock = RequestRawMock::with_expect(Default::default(), Default::default());

        assert_eq!(
            extract_mutation_data(
                &request_raw_mock,
                &mut capture_data,
                String::from(
                    r#"
    mutation ($input: userInput, $validationSets: [String]) {
        createUser(input: $input, validationSets: $validationSets) {
            id
        }
    }"#
                ),
                String::from(r#"{"input": {}}"#)
            )
            .unwrap()
            .as_str(),
            r#"{"data": {"createUser": {"id": "-1"}}}"#
        );

        assert_eq!(
            extract_mutation_data(
                &request_raw_mock,
                &mut capture_data,
                String::from(
                    r#"
    mutation ($input: userInput, $validationSets: [String]) {
        createUser(input: $input, validationSets: $validationSets) {
            id
        }
    }"#
                ),
                String::from(r#"{"input": {}}"#)
            )
            .unwrap()
            .as_str(),
            r#"{"data": {"createUser": {"id": "-2"}}}"#
        );

        let MassMutateEntries {
            mut create,
            update,
            delete,
        } = capture_data.capture_stack.pop().unwrap();

        assert!(capture_data.reserved_ids.is_empty());
        assert_eq!(
            capture_data
                .model_names_of_local_ids
                .pop()
                .unwrap()
                .as_str(),
            "User"
        );
        assert_eq!(
            capture_data
                .model_names_of_local_ids
                .pop()
                .unwrap()
                .as_str(),
            "User"
        );
        assert!(capture_data.model_names_of_local_ids.is_empty());
        assert_eq!(
            capture_data
                .reserve_id_count_per_model
                .remove("User")
                .unwrap(),
            2
        );
        assert!(capture_data.reserve_id_count_per_model.is_empty());

        let MassMutateEntry {
            model_name: create_model_name,
            variables: create_variables,
        } = create.pop().unwrap();
        assert_eq!(create_model_name.as_str(), "User");
        assert!(
            matches!(create_variables, MutationInput { input: MutationInputVariable { id: -1, other_inputs }, validation_sets: None } if other_inputs.is_empty())
        );
        let MassMutateEntry {
            model_name: create_model_name,
            variables: create_variables,
        } = create.pop().unwrap();
        assert_eq!(create_model_name.as_str(), "User");
        assert!(
            matches!(create_variables, MutationInput { input: MutationInputVariable { id: -2, other_inputs }, validation_sets: None } if other_inputs.is_empty())
        );
        assert!(create.is_empty());
        assert!(delete.is_empty());
        assert!(update.is_empty());
    }

    #[test]
    fn negative_id_replacement_test() {
        let mut mutation_input: MutationInput = serde_json::from_str(r#"{"input": {"id": -3, "key": "value", "relation": {"_replace": [{"id": -1}, {"id": 1}]}, "relation2": {"_add": [{"id": -2}]}}}"#).unwrap();

        replace_negative_ids_in_mutation_input(&[2, 3, 4], &mut mutation_input.input).unwrap();

        assert_eq!(mutation_input.input.id, 4);
        assert_eq!(
            mutation_input.input.other_inputs.remove("key").unwrap(),
            "value"
        );
        let serde_json::Value::Object(mut relation) = mutation_input
            .input
            .other_inputs
            .remove("relation")
            .unwrap()
        else {
            panic!("mutation input relation was not an object")
        };
        let serde_json::Value::Array(mut replace) = relation.remove("_replace").unwrap() else {
            panic!("mutation input relation replace was not an array")
        };
        let serde_json::Value::Object(mut id1) = replace.pop().unwrap() else {
            panic!("mutation input relation replace pop was not an object")
        };
        assert_eq!(id1.remove("id").unwrap(), 1);
        assert!(id1.is_empty());
        let serde_json::Value::Object(mut id2) = replace.pop().unwrap() else {
            panic!("mutation input relation replace pop was not an object")
        };
        assert_eq!(id2.remove("id").unwrap(), 2);
        assert!(id2.is_empty());
        assert!(replace.is_empty());
        assert!(relation.is_empty());
        let serde_json::Value::Object(mut relation2) = mutation_input
            .input
            .other_inputs
            .remove("relation2")
            .unwrap()
        else {
            panic!("mutation input relation2 was not an object")
        };
        let serde_json::Value::Array(mut add) = relation2.remove("_add").unwrap() else {
            panic!("mutation input relation2 add was not an array")
        };
        let serde_json::Value::Object(mut id3) = add.pop().unwrap() else {
            panic!("mutation input relation2 add pop was not an object")
        };
        assert_eq!(id3.remove("id").unwrap(), 3);
        assert!(id3.is_empty());
        assert!(add.is_empty());
        assert!(relation2.is_empty());
    }

    #[test]
    fn reserve_ids_test() {
        let capture_stack = Vec::default();
        let reserve_id_count_per_model =
            HashMap::from([(String::from("Oozer"), 2), (String::from("User"), 2)]);

        let mut expected_requests = VecDeque::default();
        let mut responses = VecDeque::default();

        for key in reserve_id_count_per_model.keys() {
            match key.as_str() {
                "User" => {
                    expected_requests.push_back((
                        String::from(RESERVE_ID_QUERY),
                        String::from(r#"{"model": "User", "amount": 2}"#),
                    ));
                    responses.push_back(Ok(String::from(
                        r#"{"data": {"reserveRecords": {"ids": [1, 2]}}}"#,
                    )));
                }
                "Oozer" => {
                    expected_requests.push_back((
                        String::from(RESERVE_ID_QUERY),
                        String::from(r#"{"model": "Oozer", "amount": 2}"#),
                    ));
                    responses.push_back(Ok(String::from(
                        r#"{"data": {"reserveRecords": {"ids": [3, 4]}}}"#,
                    )));
                }
                _ => panic!("Unexpected request model"),
            }
        }

        let mut capture_data = CaptureData {
            model_names_of_local_ids: vec![
                String::from("Oozer"),
                String::from("User"),
                String::from("User"),
                String::from("Oozer"),
            ],
            reserve_id_count_per_model,
            reserved_ids: Vec::new(),
            capture_stack,
        };

        let request_raw_mock = RequestRawMock::with_expect(expected_requests, responses);

        let real_ids = capture_data.reserve_ids(&request_raw_mock).unwrap();

        assert_eq!(real_ids.as_slice(), [3, 1, 2, 4]);
        assert!(capture_data.reserve_id_count_per_model.is_empty());
        assert!(capture_data.reserved_ids.is_empty());
        assert!(capture_data.model_names_of_local_ids.is_empty());
    }

    #[test]
    fn reserve_ids_saves_stacks_test() {
        let capture_stack = Vec::from([MassMutateEntries::default()]);
        let reserve_id_count_per_model = HashMap::from([(String::from("User"), 1)]);

        let expected_requests = VecDeque::from([
            (
                String::from(RESERVE_ID_QUERY),
                String::from(r#"{"model": "User", "amount": 1}"#),
            ),
            (
                String::from(RESERVE_ID_QUERY),
                String::from(r#"{"model": "User", "amount": 2}"#),
            ),
            (
                String::from(RESERVE_ID_QUERY),
                String::from(r#"{"model": "User", "amount": 1}"#),
            ),
        ]);
        let responses = VecDeque::from([
            Ok(String::from(
                r#"{"data": {"reserveRecords": {"ids": [1]}}}"#,
            )),
            Ok(String::from(
                r#"{"data": {"reserveRecords": {"ids": [2, 3]}}}"#,
            )),
            Ok(String::from(
                r#"{"data": {"reserveRecords": {"ids": [4]}}}"#,
            )),
        ]);

        let mut capture_data = CaptureData {
            model_names_of_local_ids: vec![String::from("User")],
            reserve_id_count_per_model,
            reserved_ids: Vec::new(),
            capture_stack,
        };

        let request_raw_mock = RequestRawMock::with_expect(expected_requests, responses);

        let real_ids = capture_data.reserve_ids(&request_raw_mock).unwrap();

        assert_eq!(real_ids.as_slice(), [1]);
        assert_eq!(capture_data.reserved_ids.as_slice(), [1]);
        assert!(capture_data.reserve_id_count_per_model.is_empty());
        assert!(capture_data.model_names_of_local_ids.is_empty());

        capture_data
            .reserve_id_count_per_model
            .insert(String::from("User"), 2);
        capture_data
            .model_names_of_local_ids
            .push(String::from("User"));
        capture_data
            .model_names_of_local_ids
            .push(String::from("User"));

        let real_ids = capture_data.reserve_ids(&request_raw_mock).unwrap();

        assert_eq!(real_ids.as_slice(), [1, 2, 3]);
        assert_eq!(capture_data.reserved_ids.as_slice(), [1, 2, 3]);
        assert!(capture_data.reserve_id_count_per_model.is_empty());
        assert!(capture_data.model_names_of_local_ids.is_empty());

        capture_data.capture_stack.pop();

        capture_data
            .reserve_id_count_per_model
            .insert(String::from("User"), 1);
        capture_data
            .model_names_of_local_ids
            .push(String::from("User"));

        let real_ids = capture_data.reserve_ids(&request_raw_mock).unwrap();

        assert_eq!(real_ids.as_slice(), [1, 2, 3, 4]);
        assert!(capture_data.reserved_ids.is_empty());
        assert!(capture_data.reserve_id_count_per_model.is_empty());
        assert!(capture_data.model_names_of_local_ids.is_empty());
    }

    #[test]
    fn upsert_many_input_formatting_test() {
        let mut upsert_many_inputs = generate_upsert_many_inputs(
            Vec::from([
                MassMutateEntry {
                    model_name: String::from("User"),
                    variables: MutationInput {
                        input: MutationInputVariable {
                            id: 1,
                            other_inputs: serde_json::Map::new(),
                        },
                        validation_sets: Some(ValidationSets::Default),
                    },
                },
                MassMutateEntry {
                    model_name: String::from("User"),
                    variables: MutationInput {
                        input: MutationInputVariable {
                            id: 2,
                            other_inputs: serde_json::Map::new(),
                        },
                        validation_sets: None,
                    },
                },
                MassMutateEntry {
                    model_name: String::from("Oozer"),
                    variables: MutationInput {
                        input: MutationInputVariable {
                            id: -1,
                            other_inputs: serde_json::Map::new(),
                        },
                        validation_sets: None,
                    },
                },
            ]),
            Vec::from([MassMutateEntry {
                model_name: String::from("User"),
                variables: MutationInput {
                    input: MutationInputVariable {
                        id: 1,
                        other_inputs: serde_json::Map::from_iter(
                            [(
                                String::from("key"),
                                serde_json::Value::String(String::from("value")),
                            )]
                            .into_iter(),
                        ),
                    },
                    validation_sets: Some(ValidationSets::Empty),
                },
            }]),
            &[1],
        )
        .unwrap();

        let (user_validation_sets, mut user_input) = upsert_many_inputs.remove("User").unwrap();

        assert!(matches!(user_validation_sets, ValidationSets::Empty));

        let mut update_input = user_input.pop().unwrap();

        assert_eq!(update_input.id, 1);
        assert_eq!(update_input.other_inputs.remove("key").unwrap(), "value");
        assert!(update_input.other_inputs.is_empty());

        let create_input2 = user_input.pop().unwrap();
        assert_eq!(create_input2.id, 2);
        assert!(create_input2.other_inputs.is_empty());

        let create_input = user_input.pop().unwrap();
        assert_eq!(create_input.id, 1);
        assert!(create_input.other_inputs.is_empty());

        assert!(user_input.is_empty());

        let (oozer_validation_sets, mut oozer_input) = upsert_many_inputs.remove("Oozer").unwrap();

        assert!(matches!(oozer_validation_sets, ValidationSets::Default));

        let create_input3 = oozer_input.pop().unwrap();

        assert_eq!(create_input3.id, 1);
        assert!(create_input3.other_inputs.is_empty());

        assert!(oozer_input.is_empty());

        assert!(upsert_many_inputs.is_empty());
    }

    #[test]
    fn upsert_many_sending_test() {
        let upsert_many_inputs = HashMap::from([
            (
                String::from("User"),
                (
                    ValidationSets::Empty,
                    vec![
                        MutationInputVariable {
                            id: 1,
                            other_inputs: serde_json::Map::default(),
                        },
                        MutationInputVariable {
                            id: 2,
                            other_inputs: serde_json::Map::from_iter(
                                [(
                                    String::from("key"),
                                    serde_json::Value::String(String::from("value")),
                                )]
                                .into_iter(),
                            ),
                        },
                    ],
                ),
            ),
            (
                String::from("Oozer"),
                (
                    ValidationSets::Default,
                    vec![MutationInputVariable {
                        id: 1,
                        other_inputs: serde_json::Map::default(),
                    }],
                ),
            ),
        ]);

        let mut expected_requests = VecDeque::default();
        let responses = VecDeque::from([Ok(String::default()), Ok(String::default())]);

        for key in upsert_many_inputs.keys() {
            match key.as_str() {
                "User" => 
                    expected_requests.push_back((String::from("mutation { upsertManyUser(input: $input, validationSets: $validationSets) { id } }"), String::from(r#"{"input": [{"id":1},{"id":2,"key":"value"}], "validationSets": "empty"}"#))),
                "Oozer" => 
                    expected_requests.push_back((String::from("mutation { upsertManyOozer(input: $input, validationSets: $validationSets) { id } }"), String::from(r#"{"input": [{"id":1}], "validationSets": "default"}"#))),
                _ => panic!("Unexpected request model")
            }
        }

        let request_raw_mock = RequestRawMock::with_expect(expected_requests, responses);

        send_upsert_many_mutations(upsert_many_inputs, &request_raw_mock).unwrap();
    }

    #[test]
    fn delete_many_input_formatting_test() {
        let mut delete_many_inputs = generate_delete_many_inputs(
            Vec::from([
                MassMutateEntry {
                    model_name: String::from("User"),
                    variables: DeleteInput { id: 1 },
                },
                MassMutateEntry {
                    model_name: String::from("User"),
                    variables: DeleteInput { id: 3 },
                },
                MassMutateEntry {
                    model_name: String::from("Oozer"),
                    variables: DeleteInput { id: -1 },
                },
            ]),
            &[1],
        )
        .unwrap();

        assert_eq!(
            delete_many_inputs.remove("User").unwrap().as_slice(),
            [1, 3]
        );
        assert_eq!(delete_many_inputs.remove("Oozer").unwrap().as_slice(), [1]);

        assert!(delete_many_inputs.is_empty());
    }

    #[test]
    fn delete_many_sending_test() {
        let delete_many_inputs: HashMap<String, Vec<isize>> = HashMap::from([
            (String::from("User"), vec![1, 2]),
            (String::from("Oozer"), vec![1]),
        ]);

        let mut expected_requests = VecDeque::default();
        let responses = VecDeque::from([Ok(String::default()), Ok(String::default())]);

        for key in delete_many_inputs.keys() {
            match key.as_str() {
                "User" => expected_requests.push_back((
                    String::from("mutation { deleteManyUser(input: $input) { id } }"),
                    String::from(r#"{"input": {"ids": [1,2]}}"#),
                )),
                "Oozer" => expected_requests.push_back((
                    String::from("mutation { deleteManyOozer(input: $input) { id } }"),
                    String::from(r#"{"input": {"ids": [1]}}"#),
                )),
                _ => panic!("Unexpected request model"),
            }
        }

        let request_raw_mock = RequestRawMock::with_expect(expected_requests, responses);

        send_delete_many_mutations(delete_many_inputs, &request_raw_mock).unwrap();
    }
}