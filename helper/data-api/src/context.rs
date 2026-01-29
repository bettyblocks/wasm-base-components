use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};

use crate::exports::betty_blocks::data_api::data_api::{GuestDataApi, JsonString};
use crate::{inner_request, Config};

pub struct DataAPIContext {
    application_id: String,
    action_id: String,
    /// jwt of the customer (so not the jaws jwt used for authenticating the server to server communication)
    jwt: Option<String>,
    mass_mutate_entries: Arc<Mutex<HashMap<String, MassMutateEntries>>>,
    capture_stack: Arc<Mutex<Vec<String>>>,
}

#[derive(Default)]
pub struct MassMutateEntries {
    create: VecDeque<MassMutateEntry>,
    update: VecDeque<MassMutateEntry>,
    delete: VecDeque<MassMutateEntry>,
}

pub struct MassMutateEntry {
    model_name: String,
    variables: serde_json::Map<String, serde_json::Value>,
}

impl GuestDataApi for DataAPIContext {
    fn new(application_id: String, action_id: String, jwt: Option<String>) -> Self {
        DataAPIContext {
            application_id,
            action_id,
            jwt,
            mass_mutate_entries: Default::default(),
            capture_stack: Default::default(),
        }
    }

    fn apply_capture(&self) -> Result<String, String> {
        todo!("construct the different mutations and apply them")
    }

    fn discard_capture(&self) -> Result<String, String> {
        if let Some(capture_id) = self.capture_stack.lock().expect("lock is poisoned").pop() {
            match self
                .mass_mutate_entries
                .lock()
                .expect("lock is poisoned")
                .remove(&capture_id)
            {
                Some(_) => return Ok(format!("deleted capture entry with id {capture_id}")),
                None => {
                    return Ok(format!(
                        "capture entry with id {capture_id} was already deleted"
                    ))
                }
            }
        }

        Ok(String::from("nothing to do"))
    }

    fn start_capture(&self) -> Result<String, String> {
        let random_capture_id = String::from("hallo1234");
        {
            self.mass_mutate_entries
                .lock()
                .expect("lock is poisoned")
                .insert(random_capture_id.clone(), MassMutateEntries::default());
        }

        {
            self.capture_stack
                .lock()
                .expect("lock is poisoned")
                .push(random_capture_id.clone());
        }

        Ok(random_capture_id)
    }

    fn request(&self, query: String, variables: JsonString) -> Result<JsonString, String> {
        if let Ok(capture_stack) = self.capture_stack.lock() {
            if capture_stack.len() > 0 {
                todo!("parse gql and put to correct capture stack if it is a single create,update,delete. If it is a many let it continue")
            }
        }

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

#[derive(Debug, PartialEq)]
pub enum DataMutation {
    Create,
    Update,
    Delete,
}

#[derive(Debug, PartialEq)]
pub struct Intruction {
    operator: DataMutation,
    name: String,
    data: Option<serde_json::Map<String, serde_json::Value>>,
}

impl Intruction {
    pub fn create(name: String) -> Self {
        Intruction {
            name,
            operator: DataMutation::Create,
            data: None,
        }
    }

    pub fn update(name: String) -> Self {
        Intruction {
            name,
            operator: DataMutation::Update,
            data: None,
        }
    }

    pub fn delete(name: String) -> Self {
        Intruction {
            name,
            operator: DataMutation::Delete,
            data: None,
        }
    }
}

use graphql_parser::parse_query;
use graphql_parser::query::ParseError;
use serde_json::Number;

pub fn parse_graphql_to_intruction(graphql: &str) -> Result<Option<Intruction>, ParseError> {
    let document = parse_query::<String>(graphql)?;
    let mut instruction = None;

    // TODO: get the inlined data from the mutation
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
                                let mut new_instruction = Intruction::create(name);

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
                                let mut new_instruction = Intruction::update(name);

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
                                let mut new_instruction = Intruction::delete(name);

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
    instruction: &mut Intruction,
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
fn parse_graphql_to_intruction_test() {
    let query = "mutation {createSong(input: $input)} {id}";
    let out = parse_graphql_to_intruction(query).unwrap().unwrap();
    assert_eq!(out, Intruction::create("Song".to_string()));

    let query = r#"
    mutation ($input: userInput, $validationSets: [String]) {
        createuser(input: $input, validationSets: $validationSets) {
            id
        }
    }"#;

    let out = parse_graphql_to_intruction(query).unwrap().unwrap();
    assert_eq!(out, Intruction::create("user".to_string()));

    let query = r#"
        mutation ($id: Int!, $input: userInput, $validationSets: [String]) {
            updateuser(id: $id, input: $input, validationSets: $validationSets) {
                id
        }
    }"#;

    let out = parse_graphql_to_intruction(query).unwrap().unwrap();
    assert_eq!(out, Intruction::update("user".to_string()));

    let query = r#"
        mutation ($id: Int!) {
            deleteuser(id: $id) {
                id
            }
        }"#;

    let out = parse_graphql_to_intruction(query).unwrap().unwrap();
    assert_eq!(out, Intruction::delete("user".to_string()));

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
    let mut instruction = Intruction::delete("user".to_string());
    instruction.data = Some(serde_json::Map::from_iter([(
        String::from("id"),
        serde_json::Value::Number(2.into()),
    )]));

    assert_eq!(out, instruction);
}
