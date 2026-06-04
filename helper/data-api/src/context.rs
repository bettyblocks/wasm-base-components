use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};

use crate::exports::betty_blocks::data_api::data_api::{self, GuestDataApi, JsonString};
use crate::{Config, inner_request};

pub struct DataAPIContext {
    application_id: String,

    // TODO: Check if this is still dead
    #[allow(dead_code)]
    action_id: String,
    /// jwt of the customer (so not the jaws jwt used for authenticating the server to server communication)
    jwt: Option<String>,
    capture_stack: Arc<Mutex<Vec<MassMutateEntries>>>,
}

#[derive(Default, Debug)]
pub struct MassMutateEntries {
    create: VecDeque<MassMutateEntry>,
    update: VecDeque<MassMutateEntry>,
    upsert: VecDeque<MassMutateEntry>,
    delete: VecDeque<MassMutateEntry>,
}

impl MassMutateEntries {
    fn as_pending_mutations(&self) -> [Vec<data_api::PendingMutation>; 4] {
        [
            self.create
                .iter()
                .map(|x| x.as_pending_mutation(DataMutation::Create))
                .collect(),
            self.update
                .iter()
                .map(|x| x.as_pending_mutation(DataMutation::Update))
                .collect(),
            self.upsert
                .iter()
                .map(|x| x.as_pending_mutation(DataMutation::Upsert))
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

fn format_mutation(model_name: &str, operation: &DataMutation) -> String {
    format!(
        "mutation {{ {}{model_name}(input: $input) {{ id }} }}",
        operation.as_static_str()
    )
}

fn format_mutation_name(model_name: &str, operation: &DataMutation) -> String {
    format!("{}{model_name}", operation.as_static_str())
}

fn generate_delayed_id(id: &str, mutation_name: &str) -> JsonString {
    format!(r#"{{"data": {{"{mutation_name}": {{"id": "{id}"}}}}}}"#)
}

impl GuestDataApi for DataAPIContext {
    fn new(application_id: String, action_id: String, jwt: Option<String>) -> Self {
        DataAPIContext {
            application_id,
            action_id,
            jwt,
            capture_stack: Default::default(),
        }
    }

    fn apply_capture(&self) -> Result<String, String> {
        if let Some(MassMutateEntries {
            create,
            update,
            upsert,
            delete,
        }) = self.capture_stack.lock().expect("lock is poisoned").pop()
        {
            /* TODO: resolve ids
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

            // If we want to be able to specify a unique by for the upsert we would want to handle them separately,
            // but this is left out of cope for now.
            let upsert_manys = [create, update, upsert].into_iter().flatten().fold(
                HashMap::new(),
                |mut map, mut item| {
                    // TODO: Add reserved_ids.
                    replace_negative_ids_in_variables(&vec![], &mut item.variables);

                    map.entry(item.model_name)
                        .or_insert(Vec::new())
                        .push(item.variables);
                    map
                },
            );

            // TODO: Do we want to delete entries before they get upserted where possible?
            // Would mean less unnecessary mutations in the data api, but likely worse performance if we need to check for that here.
            // It would look something like this.
            // for entry in delete {
            //     if let Some(id) = entry.variables.get("id") && let Some(model_entry) = x.get_mut(&entry.model_name) {
            //         model_entry.retain(|v| !v.get("id").is_some_and(|y| y == id))
            //         // Still need to delete if this doesn't match anything
            //     }
            // }

            for (model_name, all_variables) in upsert_manys {
                let query =
                    format!("mutation {{ upsertMany{model_name}(input: $input) {{ id }} }}");

                let variables = format!(
                    "{{\"input\": {}}}",
                    serde_json::to_string(&all_variables)
                        .map_err(|_| String::from("could not format input variables"))?
                );

                // TODO: set up some kind of mocking so this doesn't break when testing. Same goes for the delete manys.
                // self.request_raw(query, variables)?;
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

                // self.request_raw(query, variables)?;
            }
        }

        Ok(String::from("nothing to do"))
    }

    fn discard_capture(&self) -> Result<String, String> {
        if let Some(_) = self.capture_stack.lock().expect("lock is poisoned").pop() {
            return Ok(format!("deleted most recent capture stack entry"));
        }

        Ok(String::from("nothing to do"))
    }

    fn start_capture(&self) -> Result<(), String> {
        self.capture_stack
            .lock()
            .map_err(|_| String::from("capture stack lock poisoned"))?
            .push(MassMutateEntries::default());
        Ok(())
    }

    fn pending_capture(&self) -> Result<Vec<Vec<data_api::PendingMutation>>, String> {
        if let Some(entries) = self.capture_stack.lock().expect("lock is poisoned").last() {
            return Ok(entries.as_pending_mutations().to_vec());
        }

        Ok(Vec::new())
    }

    fn request(&self, query: String, variables: JsonString) -> Result<JsonString, String> {
        if let Ok(mut capture_stack) = self.capture_stack.lock() {
            if let Some(entries) = capture_stack.last_mut() {
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

                match &query_remainder[0..6] {
                    "create" => &mut entries.create,
                    "update" => &mut entries.update,
                    "upsert" => &mut entries.upsert,
                    "delete" => &mut entries.delete,
                    _ => return self.request_raw(query, variables),
                }
                .push_back(MassMutateEntry {
                    model_name: mutation_name[6..].to_string(),
                    variables: serde_json::from_str(&variables)
                        .map_err(|_| String::from("could not parse variables"))?,
                });

                // TODO: Generate a local id that will be resolved to a real id later.
                // Read more about this logic in [[Self::apply_capture]]
                return Ok(generate_delayed_id("1", mutation_name));
            }
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
    reserved_ids: &Vec<usize>,
    variables: &mut serde_json::Map<String, serde_json::Value>,
) {
    for (key, value) in variables.iter_mut() {
        if key == "id" {
            handle_id_key(reserved_ids, value)
        } else if let serde_json::Value::Object(object) = value {
            replace_negative_ids_in_variables(reserved_ids, object);
        } else if let serde_json::Value::Array(array_of_values) = value {
            handle_array_of_values(reserved_ids, array_of_values);
        }
    }
}

/// Replaces the negative ID or IDs with their reserved counterpart.
fn handle_id_key(reserved_ids: &[usize], value: &mut serde_json::Value) {
    if let Some(id) = value.as_i64()
        && id.is_negative()
    {
        *value = get_reserved_id_as_value_for_negative_id(reserved_ids, id);
    } else if let serde_json::Value::Array(array_of_ids) = value {
        handle_array_of_ids(reserved_ids, array_of_ids);
    }
}

/// Loops over an Vec of IDs and replaces negative ones with their reserved counterpart.
fn handle_array_of_ids(reserved_ids: &[usize], array_of_ids: &mut [serde_json::Value]) {
    for item in array_of_ids.iter_mut() {
        if let Some(id) = item.as_i64()
            && id.is_negative()
        {
            *item = get_reserved_id_as_value_for_negative_id(reserved_ids, id);
        }
    }
}

/// Uses the negative_id to index for its reserved id and put it into a serde_json::Value.
fn get_reserved_id_as_value_for_negative_id(
    reserved_ids: &[usize],
    negative_id: i64,
) -> serde_json::Value {
    serde_json::Value::Number(serde_json::Number::from(
        // If `negative_id` is -1 which is the first possible negative ID, then `-1 - -1 = 0`.
        // Which is the first possible item in the reserved ID list.
        reserved_ids[(-1 - negative_id) as usize],
    ))
}

/// Looks at the items in an array and if they're an object tries to replace negative IDs in that
/// object, and if they are an array it searches for objects or arrays within that object which
/// might have negative IDs to replace.
fn handle_array_of_values(reserved_ids: &Vec<usize>, array_of_values: &mut [serde_json::Value]) {
    for item in array_of_values.iter_mut() {
        if let serde_json::Value::Object(object) = item {
            replace_negative_ids_in_variables(reserved_ids, object);
        } else if let serde_json::Value::Array(array) = item {
            handle_array_of_values(reserved_ids, array);
        }
    }
}

#[derive(Debug, PartialEq)]
pub enum DataMutation {
    Create,
    Update,
    Upsert,
    Delete,
}

impl DataMutation {
    fn as_static_str(&self) -> &'static str {
        use DataMutation::*;

        match self {
            Create => "create",
            Update => "update",
            Upsert => "upsert",
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

    pub fn upsert(name: String) -> Self {
        Instruction {
            name,
            operator: DataMutation::Upsert,
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
            graphql_parser::query::Definition::Operation(operation) => match operation {
                graphql_parser::query::OperationDefinition::Mutation(mutation) => {
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

                                extract_values_from_gql_argument(
                                    &mut new_instruction,
                                    field.arguments,
                                );

                                instruction = Some(new_instruction);
                            }
                            graphql_parser::query::Selection::Field(field)
                                if field.name.starts_with("update")
                                    && !field.name.starts_with("updateMany") =>
                            {
                                let name = field.name.split_at("update".len()).1.to_string();
                                let mut new_instruction = Instruction::update(name);

                                extract_values_from_gql_argument(
                                    &mut new_instruction,
                                    field.arguments,
                                );

                                instruction = Some(new_instruction);
                            }
                            graphql_parser::query::Selection::Field(field)
                                if field.name.starts_with("delete")
                                    && !field.name.starts_with("deleteMany") =>
                            {
                                let name = field.name.split_at("delete".len()).1.to_string();
                                let mut new_instruction = Instruction::delete(name);

                                extract_values_from_gql_argument(
                                    &mut new_instruction,
                                    field.arguments,
                                );

                                instruction = Some(new_instruction);
                            }
                            _ => {}
                        }
                    }
                }
                graphql_parser::query::OperationDefinition::SelectionSet(set) => {
                    for item in set.items {
                        match item {
                            graphql_parser::query::Selection::Field(field)
                                if field.name == "id" => {}
                            _ => {}
                        }
                    }
                }
                _ => {}
            },
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
        graphql_parser::query::Value::Variable(_) => return None,
        graphql_parser::query::Value::Boolean(b) => return Some(serde_json::Value::Bool(b)),
        graphql_parser::query::Value::String(s) => {
            return Some(serde_json::Value::String(s));
        }
        graphql_parser::query::Value::Int(i) => {
            let num = i.as_i64()?;
            return Some(serde_json::Value::Number(num.into()));
        }
        graphql_parser::query::Value::Float(f) => {
            return Some(serde_json::Value::Number(Number::from_f64(f)?));
        }
        graphql_parser::query::Value::Null => {
            return Some(serde_json::Value::Null);
        }
        graphql_parser::query::Value::Enum(s) => {
            return Some(serde_json::Value::String(s));
        }
        graphql_parser::query::Value::List(l) => {
            let list = l.into_iter().flat_map(graphql_to_json).collect();
            return Some(serde_json::Value::Array(list));
        }
        graphql_parser::query::Value::Object(m) => {
            let mut map = serde_json::Map::new();
            for (k, v) in m {
                if let Some(json) = graphql_to_json(v) {
                    map.insert(k, json);
                }
            }
            return Some(serde_json::Value::Object(map));
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
            String::from(r#"{"id": 1}"#)
        )
        .unwrap()
        .as_str(),
        r#"{"data": {"createuser": {"id": "1"}}}"#
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
            String::from(r#"{"id": 1}"#)
        )
        .unwrap()
        .as_str(),
        r#"{"data": {"deleteuser": {"id": "1"}}}"#
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
