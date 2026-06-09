use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};

use crate::exports::betty_blocks::data_api::data_api::{self, GuestDataApi, JsonString};
use crate::{Config, inner_request};

type ModelName = String;
type InternalId = isize;
type RealId = i32;

pub struct DataAPIContext {
    application_id: String,

    // TODO: Check if this is still dead
    #[allow(dead_code)]
    action_id: String,
    /// jwt of the customer (so not the jaws jwt used for authenticating the server to server communication)
    jwt: Option<String>,
    capture_data: Arc<Mutex<CaptureData>>,
}

#[derive(Debug, Default)]
pub struct CaptureData {
    model_names_of_local_ids: Vec<ModelName>,
    reserve_id_count_per_model: HashMap<ModelName, usize>,
    capture_stack: Vec<MassMutateEntries>,
}

#[derive(Default, Debug)]
pub struct MassMutateEntries {
    // TODO: We are probably able to turn these into Vecs because we iterate through them front-to-back anyway
    create: VecDeque<MassMutateEntry>,
    update: VecDeque<MassMutateEntry>,
    delete: VecDeque<MassMutateEntry>,
}

impl MassMutateEntries {
    fn as_pending_mutations(&self) -> [Vec<data_api::PendingMutation>; 3] {
        [
            self.create
                .iter()
                .map(|x| x.as_pending_mutation(DataMutation::Create))
                .collect(),
            self.update
                .iter()
                .map(|x| x.as_pending_mutation(DataMutation::Update))
                .collect(),
            self.delete
                .iter()
                .map(|x| x.as_pending_mutation(DataMutation::Delete))
                .collect(),
        ]
    }
}

#[derive(Debug)]
pub struct MassMutateEntry {
    model_name: String,
    variables: serde_json::Map<String, serde_json::Value>,
}

impl MassMutateEntry {
    fn as_pending_mutation(&self, operation: DataMutation) -> data_api::PendingMutation {
        data_api::PendingMutation {
            mutation_name: format_mutation_name(&self.model_name, &operation),
            mutation: format_mutation(&self.model_name, &operation),
            variables: serde_json::to_string(&self.variables).expect("incorrect variables"),
        }
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
pub struct ReserveIdMutationResult {
    data: ReserveIdResult,
}

#[derive(serde::Serialize, serde::Deserialize)]
pub struct ReserveIdResult {
    #[serde(rename = "reserveRecords")]
    reserved_ids: ReservedIds,
}

#[derive(serde::Serialize, serde::Deserialize)]
pub struct ReservedIds {
    ids: VecDeque<u32>,
}

#[derive(serde::Serialize, serde::Deserialize)]
pub struct MutationInput {
    input: MutationInputVariable,
}

#[derive(serde::Serialize, serde::Deserialize)]
pub struct MutationInputVariable {
    id: InternalId,
    #[serde(flatten)]
    other_inputs: HashMap<String, serde_json::Value>,
}

fn format_mutation(model_name: &str, operation: &DataMutation) -> String {
    format!(
        "mutation {{ {}{model_name}(input: $input) {{ id }} }}",
        operation.as_static_str()
    )
}

fn format_mutation_name(model_name: &str, operation: &DataMutation) -> String {
    format!("{}{model_name}", operation.as_static_str())
}

fn generate_delayed_id_response(id: &str, mutation_name: &str) -> JsonString {
    format!(r#"{{"data": {{"{mutation_name}": {{"id": "{id}"}}}}}}"#)
}

fn reserve_ids(
    data_api_context: &impl GuestDataApi,
    (model_name, id_count): (ModelName, usize),
) -> Result<(ModelName, VecDeque<u32>), String> {
    let query = r#"mutation ($model: String!, $amount: Int!) {
                    reserveRecords(model: $model, amount: $amount) {
                    ids
                    }
                }"#;

    let variables = format!(r#"{{"model": "{model_name}", "amount": {id_count}}}"#);

    let res = data_api_context.request_raw(query.to_string(), variables)?;

    let ReserveIdMutationResult {
        data: ReserveIdResult {
            reserved_ids: ReservedIds { ids },
        },
    } = serde_json::from_str(&res)
        .map_err(|_| String::from("could not parse data api result for reserving ids"))?;

    Ok((model_name, ids))
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

    fn apply_capture(&self) -> Result<String, String> {
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
            // TODO
            // Problem: This will currently not work when reserving ids for a higher scoped capture, because the reserved ids will be dropped by the time they're needed.
            // Solution: Save the reserved ids in the capture data until the capture data is empty.
            // if stack_is_empty {clear} else {extend; return extended_vec}
            let mut reserved_id_map = capture_data
                .reserve_id_count_per_model
                .drain()
                .map(|entry| reserve_ids(self, entry))
                .collect::<Result<HashMap<ModelName, VecDeque<u32>>, String>>()?;

            let reserved_ids = capture_data
                .model_names_of_local_ids
                .iter()
                .map(|model_name| {
                    Ok(reserved_id_map
                        .get_mut(model_name)
                        .ok_or_else(|| {
                            format!("ids for model {model_name} were not properly reserved")
                        })?
                        .pop_front()
                        .ok_or_else(|| {
                            format!("not enough ids were reserved for model {model_name}")
                        })? as RealId)
                })
                .collect::<Result<Vec<RealId>, String>>()?;

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

            let mut upsert_manys = HashMap::new();

            // If we want to be able to specify a unique by for the upsert we would want to handle them separately,
            // but this is left out of scope for now.

            for MassMutateEntry {
                model_name,
                mut variables,
            } in [create, update].into_iter().flatten()
            {
                replace_negative_ids_in_variables(&reserved_ids, &mut variables).unwrap();

                upsert_manys
                    .entry(model_name)
                    .or_insert(Vec::new())
                    .push(variables);
            }

            for (model_name, variables) in upsert_manys {
                let query =
                    format!("mutation {{ upsertMany{model_name}(input: $input) {{ id }} }}");

                let variables = format!(
                    "{{\"input\": {}}}",
                    serde_json::to_string(&variables)
                        .map_err(|_| String::from("could not format input variables"))?
                );

                // TODO: set up some kind of mocking so this doesn't break when testing. Same goes for the delete manys and reserve ids.
                // TODO: implement chunking so that this doesn't break when we have too many queries.
                self.request_raw(query, variables)?;
            }

            let delete_manys = delete
                .into_iter()
                .fold(HashMap::new(), |mut map, mut item| {
                    if let Some(id) = item.variables.remove("id") {
                        map.entry(item.model_name).or_insert(Vec::new()).push(id);
                    }
                    map
                });

            for (model_name, variables) in delete_manys {
                let query =
                    format!("mutation {{ deleteMany{model_name}(input: $input) {{ id }} }}");

                let variables = format!(
                    "{{\"input\": {{\"ids\": {}}}}}",
                    serde_json::to_string(&variables)
                        .map_err(|_| String::from("could not format input variables"))?
                );

                self.request_raw(query, variables)?;
            }
        }

        Ok(String::from("nothing to do"))
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
            let query_remainder = match query.split_once("mutation") {
                Some((_, remainder)) => remainder,
                None => return self.request_raw(query, variables),
            };

            let query_remainder = query_remainder
                .split_once('{')
                .ok_or_else(|| String::from("query is improperly formatted"))?
                .1
                .trim();

            if query_remainder[6..10] == *"Many" {
                return self.request_raw(query, variables);
            }

            let mutation_name = query_remainder
                .split_once(|c: char| c.is_whitespace() || c == '(')
                .ok_or_else(|| String::from("query is improperly formatted"))?
                .0;

            let model_name = mutation_name[6..].to_string();

            let mass_mutate_entry = MassMutateEntry {
                model_name: model_name.clone(),
                variables: serde_json::from_str(&variables)
                    .map_err(|_| String::from("could not parse variables"))?,
            };

            // It is safe to unwrap the capture stack last_mut here because we checked the length to be non-zero before. This looks a bit wack but is necessary to not mutably borrow capture_data multiple times.
            let id: isize = match &query_remainder[0..6] {
                "create" => match serde_json::from_str(&variables) {
                    Ok(MutationInput {
                        input: MutationInputVariable { id, .. },
                    }) => {
                        capture_data
                            .capture_stack
                            .last_mut()
                            .unwrap()
                            .create
                            .push_back(mass_mutate_entry);

                        id
                    }
                    Err(_) => {
                        capture_data
                            .model_names_of_local_ids
                            .push(model_name.clone());
                        *capture_data
                            .reserve_id_count_per_model
                            .entry(model_name)
                            .or_default() += 1;
                        capture_data
                            .capture_stack
                            .last_mut()
                            .unwrap()
                            .create
                            .push_back(mass_mutate_entry);
                        -capture_data
                            .capture_stack
                            .len()
                            .try_into()
                            .map_err(|_| String::from("ran out of internal ids"))?
                    }
                },
                "update" => {
                    let MutationInput {
                        input: MutationInputVariable { id, .. },
                    } = serde_json::from_str(&variables)
                        .map_err(|_| String::from("could not find id input for update query"))?;

                    capture_data
                        .capture_stack
                        .last_mut()
                        .unwrap()
                        .update
                        .push_back(mass_mutate_entry);

                    id
                }
                "delete" => {
                    let MutationInput {
                        input: MutationInputVariable { id, .. },
                    } = serde_json::from_str(&variables)
                        .map_err(|_| String::from("could not find id input for delete query"))?;

                    capture_data
                        .capture_stack
                        .last_mut()
                        .unwrap()
                        .delete
                        .push_back(mass_mutate_entry);

                    id
                }
                _ => return self.request_raw(query, variables),
            };

            return Ok(generate_delayed_id_response(&id.to_string(), mutation_name));
        }

        self.request_raw(query, variables)
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

/// Loops through the entries of a serde_json::Map and replaces all the negative IDs with their
/// reserved counterparts.
fn replace_negative_ids_in_variables(
    reserved_ids: &[RealId],
    variables: &mut serde_json::Map<String, serde_json::Value>,
) -> Result<(), String> {
    for (key, value) in variables.iter_mut() {
        if key == "id" {
            handle_id_key(reserved_ids, value)?;
        } else if let serde_json::Value::Object(object) = value {
            replace_negative_ids_in_variables(reserved_ids, object)?;
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
    } else if let serde_json::Value::Array(array_of_ids) = value {
        handle_array_of_ids(reserved_ids, array_of_ids)?;
    }

    Ok(())
}

/// Loops over an Vec of IDs and replaces negative ones with their reserved counterpart.
fn handle_array_of_ids(
    reserved_ids: &[RealId],
    array_of_ids: &mut [serde_json::Value],
) -> Result<(), String> {
    for item in array_of_ids.iter_mut() {
        if let Some(id) = item.as_i64()
            && id.is_negative()
        {
            *item = get_reserved_id_as_value_for_negative_id(reserved_ids, id)?;
        }
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
            replace_negative_ids_in_variables(reserved_ids, object)?;
        } else if let serde_json::Value::Array(array) = item {
            handle_array_of_values(reserved_ids, array)?;
        }
    }

    Ok(())
}

#[derive(Debug, PartialEq)]
pub enum DataMutation {
    Create,
    Update,
    Delete,
}

impl DataMutation {
    fn as_static_str(&self) -> &'static str {
        use DataMutation::*;

        match self {
            Create => "create",
            Update => "update",
            Delete => "delete",
        }
    }
}

#[derive(Debug, PartialEq)]
pub struct Instruction {
    operator: DataMutation,
    name: String,
    data: Option<serde_json::Map<String, serde_json::Value>>,
}

impl Instruction {
    pub fn create(name: String) -> Self {
        Instruction {
            name,
            operator: DataMutation::Create,
            data: None,
        }
    }

    pub fn update(name: String) -> Self {
        Instruction {
            name,
            operator: DataMutation::Update,
            data: None,
        }
    }

    pub fn delete(name: String) -> Self {
        Instruction {
            name,
            operator: DataMutation::Delete,
            data: None,
        }
    }
}

use graphql_parser::parse_query;
use graphql_parser::query::ParseError;
use serde_json::Number;

pub fn parse_graphql_to_intruction(graphql: &str) -> Result<Option<Instruction>, ParseError> {
    let document = parse_query::<String>(graphql)?;
    let mut instruction = None;

    for def in document.definitions {
        match def {
            graphql_parser::query::Definition::Operation(
                graphql_parser::query::OperationDefinition::Mutation(mutation),
            ) => {
                if mutation.selection_set.items.len() > 1 {
                    return Ok(None);
                }

                for item in mutation.selection_set.items {
                    match item {
                        graphql_parser::query::Selection::Field(field)
                            if field.name.starts_with("create")
                                && !field.name.starts_with("createMany") =>
                        {
                            let name = field.name.split_at("create".len()).1.to_string();
                            let mut new_instruction = Instruction::create(name);

                            extract_values_from_gql_argument(&mut new_instruction, field.arguments);

                            instruction = Some(new_instruction);
                        }
                        graphql_parser::query::Selection::Field(field)
                            if field.name.starts_with("update")
                                && !field.name.starts_with("updateMany") =>
                        {
                            let name = field.name.split_at("update".len()).1.to_string();
                            let mut new_instruction = Instruction::update(name);

                            extract_values_from_gql_argument(&mut new_instruction, field.arguments);

                            instruction = Some(new_instruction);
                        }
                        graphql_parser::query::Selection::Field(field)
                            if field.name.starts_with("delete")
                                && !field.name.starts_with("deleteMany") =>
                        {
                            let name = field.name.split_at("delete".len()).1.to_string();
                            let mut new_instruction = Instruction::delete(name);

                            extract_values_from_gql_argument(&mut new_instruction, field.arguments);

                            instruction = Some(new_instruction);
                        }
                        _ => {}
                    }
                }
            }
            graphql_parser::query::Definition::Operation(
                graphql_parser::query::OperationDefinition::SelectionSet(set),
            ) => {
                for item in set.items {
                    match item {
                        graphql_parser::query::Selection::Field(field) if field.name == "id" => {}
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }

    Ok(instruction)
}

fn extract_values_from_gql_argument(
    instruction: &mut Instruction,
    arguments: Vec<(String, graphql_parser::query::Value<'_, String>)>,
) {
    let mut tmp_data = serde_json::Map::new();
    for (key, value) in arguments {
        if let Some(json) = graphql_to_json(value) {
            tmp_data.insert(key, json);
        }
    }
    if !tmp_data.is_empty() {
        instruction.data = Some(tmp_data);
    }
}

fn graphql_to_json(val: graphql_parser::query::Value<'_, String>) -> Option<serde_json::Value> {
    match val {
        graphql_parser::query::Value::Variable(_) => None,
        graphql_parser::query::Value::Boolean(b) => Some(serde_json::Value::Bool(b)),
        graphql_parser::query::Value::String(s) => Some(serde_json::Value::String(s)),
        graphql_parser::query::Value::Int(i) => {
            let num = i.as_i64()?;
            Some(serde_json::Value::Number(num.into()))
        }
        graphql_parser::query::Value::Float(f) => {
            Some(serde_json::Value::Number(Number::from_f64(f)?))
        }
        graphql_parser::query::Value::Null => Some(serde_json::Value::Null),
        graphql_parser::query::Value::Enum(s) => Some(serde_json::Value::String(s)),
        graphql_parser::query::Value::List(l) => {
            let list = l.into_iter().flat_map(graphql_to_json).collect();
            Some(serde_json::Value::Array(list))
        }
        graphql_parser::query::Value::Object(m) => {
            let mut map = serde_json::Map::new();
            for (k, v) in m {
                if let Some(json) = graphql_to_json(v) {
                    map.insert(k, json);
                }
            }
            Some(serde_json::Value::Object(map))
        }
    }
}

#[test]
fn capture_mutation_test() {
    let ctx = DataAPIContext::new(String::new(), String::new(), Some(String::new()));

    ctx.start_capture().unwrap();

    assert_eq!(
        ctx.request(
            String::from(
                r#"
    mutation ($input: userInput, $validationSets: [String]) {
        createuser(input: $input, validationSets: $validationSets) {
            id
        }
    }"#
            ),
            String::from(r#"{"input": {"id": 2}}"#)
        )
        .unwrap()
        .as_str(),
        r#"{"data": {"createuser": {"id": "2"}}}"#
    );

    assert_eq!(
        ctx.request(
            String::from(
                r#"
    mutation ($input: userInput, $validationSets: [String]) {
        deleteuser(input: $input, validationSets: $validationSets) {
            id
        }
    }"#
            ),
            String::from(r#"{"input": {"id": 2}}"#)
        )
        .unwrap()
        .as_str(),
        r#"{"data": {"deleteuser": {"id": "2"}}}"#
    );

    ctx.apply_capture().unwrap();
}

#[test]
fn parse_graphql_to_intruction_test() {
    let query = "mutation {createSong(input: $input)} {id}";
    let out = parse_graphql_to_intruction(query).unwrap().unwrap();
    assert_eq!(out, Instruction::create("Song".to_string()));

    let query = r#"
    mutation ($input: userInput, $validationSets: [String]) {
        createuser(input: $input, validationSets: $validationSets) {
            id
        }
    }"#;

    let out = parse_graphql_to_intruction(query).unwrap().unwrap();
    assert_eq!(out, Instruction::create("user".to_string()));

    let query = r#"
        mutation ($id: Int!, $input: userInput, $validationSets: [String]) {
            updateuser(id: $id, input: $input, validationSets: $validationSets) {
                id
        }
    }"#;

    let out = parse_graphql_to_intruction(query).unwrap().unwrap();
    assert_eq!(out, Instruction::update("user".to_string()));

    let query = r#"
        mutation ($id: Int!) {
            deleteuser(id: $id) {
                id
            }
        }"#;

    let out = parse_graphql_to_intruction(query).unwrap().unwrap();
    assert_eq!(out, Instruction::delete("user".to_string()));

    let query = r#"
       {
            allSong {
                results {
                    id
                    name
                }
            }
        }"#;

    let out = parse_graphql_to_intruction(query).unwrap();
    assert_eq!(out, None);
}

#[test]
fn extract_data_from_mutation() {
    let query = r#"
        mutation {
            deleteuser(id: 2) {
                id
            }
        }"#;

    let out = parse_graphql_to_intruction(query).unwrap().unwrap();
    let mut instruction = Instruction::delete("user".to_string());
    instruction.data = Some(serde_json::Map::from_iter([(
        String::from("id"),
        serde_json::Value::Number(2.into()),
    )]));

    assert_eq!(out, instruction);
}
