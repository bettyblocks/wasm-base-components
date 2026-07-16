use std::collections::{HashMap, VecDeque};

use crate::{
    context::{
        CaptureData, InternalId, MassMutateEntry, ModelName, RealId, RequestRaw,
        data_structs::*,
        replace_ids::{replace_id, replace_negative_ids_in_mutation_input},
    },
    exports::betty_blocks_types::data_api::data_api::PendingMutation,
};

const CHUNK_SIZE: usize = 100_000;

const RESERVE_ID_QUERY: &str = "mutation ($model: String!, $amount: Int!) { reserveRecords(model: $model, amount: $amount) { ids } }";

impl CaptureData {
    pub fn reserve_ids(&mut self, request_sender: &impl RequestRaw) -> Result<Vec<RealId>, String> {
        let mut reserved_id_map = self
            .reserve_id_count_per_model
            .drain()
            .map(|(model, amount)| {
                Self::request_reserved_ids(
                    request_sender,
                    ReserveIdMutationVariables { model, amount },
                )
            })
            .collect::<Result<HashMap<ModelName, VecDeque<RealId>>, String>>()?;

        for model_name in self.model_names_of_local_ids.drain(..) {
            let reserved_ids_for_model = reserved_id_map
                .get_mut(&model_name)
                .ok_or_else(|| format!("ids for model {model_name} were not properly reserved"))?
                .pop_front()
                .ok_or_else(|| format!("not enough ids were reserved for model {model_name}"))?;

            self.reserved_ids.push(reserved_ids_for_model);
        }

        if self.capture_stack.is_empty() {
            Ok(std::mem::take(&mut self.reserved_ids))
        } else {
            Ok(self.reserved_ids.clone())
        }
    }

    pub fn request_reserved_ids(
        request_sender: &impl RequestRaw,
        variables: ReserveIdMutationVariables,
    ) -> Result<(ModelName, VecDeque<RealId>), String> {
        let variables_string = serde_json::to_string(&variables)
            .map_err(|_| String::from("could not serialize reserve id variables"))?;

        let res = request_sender.request_raw(RESERVE_ID_QUERY.to_string(), variables_string)?;

        let ReserveIdMutationResult {
            data: ReserveIdResult {
                reserved_ids: ReservedIds { ids },
            },
        } = serde_json::from_str(&res)
            .map_err(|_| String::from("could not parse data api result for reserving ids"))?;

        Ok((variables.model, ids))
    }
}

impl<T: serde::Serialize> MassMutateEntry<T> {
    pub fn as_pending_mutation(&self, operation: &str) -> PendingMutation {
        PendingMutation {
            mutation_name: format!("{operation}{}", self.model_name),
            mutation: format_mutation(&self.model_name, operation),
            variables: serde_json::to_string(&self.variables).expect("incorrect variables"),
        }
    }
}

fn format_mutation(model_name: &str, operation: &str) -> String {
    let validation_sets = if operation != "delete" {
        ", validationSets: $validationSets"
    } else {
        ""
    };

    format!(
        "mutation {{ {}{model_name}(input: $input{validation_sets}) {{ id }} }}",
        operation
    )
}

pub fn generate_upsert_many_inputs(
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
            && let ValidationSets::Empty = variables.validation_sets
        {
            *validation_sets = ValidationSets::Empty
        }

        input_vec.push(variables.input);
    }

    Ok(upsert_manys)
}

pub fn generate_delete_many_inputs(
    delete_entries: Vec<MassMutateEntry<DeleteInput>>,
    reserved_ids: &[RealId],
) -> Result<HashMap<String, Vec<InternalId>>, String> {
    let mut delete_ids = HashMap::new();

    for MassMutateEntry {
        model_name,
        variables: DeleteInput {
            id: MutationIdInput(mut id),
        },
    } in delete_entries
    {
        replace_id(reserved_ids, &mut id)?;

        delete_ids.entry(model_name).or_insert(Vec::new()).push(id);
    }

    Ok(delete_ids)
}

pub fn send_upsert_many_mutations(
    upsert_many_inputs: HashMap<String, (ValidationSets, Vec<MutationInputVariable>)>,
    request_sender: &impl RequestRaw,
) -> Result<(), String> {
    for (model_name, (validation_sets, input_variables)) in upsert_many_inputs {
        let query = format!(
            "mutation {{ upsertMany{model_name}(input: $input, validationSets: $validationSets) {{ id }} }}"
        );

        for input_chunk in input_variables.chunks(CHUNK_SIZE) {
            let variables = serde_json::to_string(
                &serde_json::json!({"input": input_chunk, "validationSets": validation_sets}),
            )
            .map_err(|_| String::from("could not format input variables"))?;

            request_sender.request_raw(query.clone(), variables)?;
        }
    }

    Ok(())
}

pub fn send_delete_many_mutations(
    delete_many_inputs: HashMap<String, Vec<isize>>,
    request_sender: &impl RequestRaw,
) -> Result<(), String> {
    for (model_name, variables) in delete_many_inputs {
        let query = format!("mutation {{ deleteMany{model_name}(input: $input) {{ id }} }}");

        for input_chunk in variables.chunks(CHUNK_SIZE) {
            let variables =
                serde_json::to_string(&serde_json::json!({"input": {"ids": input_chunk}}))
                    .map_err(|_| String::from("could not format input variables"))?;

            request_sender.request_raw(query.clone(), variables)?;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::context::{MassMutateEntries, tests::RequestRawMock};

    use super::*;

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
                        String::from(r#"{"model":"User","amount":2}"#),
                    ));
                    responses.push_back(Ok(String::from(
                        r#"{"data": {"reserveRecords": {"ids": [1, 2]}}}"#,
                    )));
                }
                "Oozer" => {
                    expected_requests.push_back((
                        String::from(RESERVE_ID_QUERY),
                        String::from(r#"{"model":"Oozer","amount":2}"#),
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
                String::from(r#"{"model":"User","amount":1}"#),
            ),
            (
                String::from(RESERVE_ID_QUERY),
                String::from(r#"{"model":"User","amount":2}"#),
            ),
            (
                String::from(RESERVE_ID_QUERY),
                String::from(r#"{"model":"User","amount":1}"#),
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
                            id: MutationIdInput(1),
                            other_inputs: serde_json::Map::new(),
                        },
                        validation_sets: ValidationSets::Default,
                    },
                },
                MassMutateEntry {
                    model_name: String::from("User"),
                    variables: MutationInput {
                        input: MutationInputVariable {
                            id: MutationIdInput(2),
                            other_inputs: serde_json::Map::new(),
                        },
                        validation_sets: ValidationSets::Default,
                    },
                },
                MassMutateEntry {
                    model_name: String::from("Oozer"),
                    variables: MutationInput {
                        input: MutationInputVariable {
                            id: MutationIdInput(-1),
                            other_inputs: serde_json::Map::new(),
                        },
                        validation_sets: ValidationSets::Default,
                    },
                },
            ]),
            Vec::from([MassMutateEntry {
                model_name: String::from("User"),
                variables: MutationInput {
                    input: MutationInputVariable {
                        id: MutationIdInput(1),
                        other_inputs: serde_json::Map::from_iter([(
                            String::from("key"),
                            serde_json::Value::String(String::from("value")),
                        )]),
                    },
                    validation_sets: ValidationSets::Empty,
                },
            }]),
            &[1],
        )
        .unwrap();

        let (user_validation_sets, user_input) = upsert_many_inputs.remove("User").unwrap();

        assert!(matches!(user_validation_sets, ValidationSets::Empty));

        let [create_input, create_input2, mut update_input] = user_input.try_into().unwrap();

        assert_eq!(*update_input.id, 1);
        assert_eq!(update_input.other_inputs.remove("key").unwrap(), "value");
        assert!(update_input.other_inputs.is_empty());

        assert_eq!(*create_input.id, 1);
        assert!(create_input.other_inputs.is_empty());
        assert_eq!(*create_input2.id, 2);
        assert!(create_input2.other_inputs.is_empty());

        let (oozer_validation_sets, oozer_input) = upsert_many_inputs.remove("Oozer").unwrap();

        assert!(matches!(oozer_validation_sets, ValidationSets::Default));

        let [create_input3] = oozer_input.try_into().unwrap();

        assert_eq!(*create_input3.id, 1);
        assert!(create_input3.other_inputs.is_empty());

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
                            id: MutationIdInput(1),
                            other_inputs: serde_json::Map::default(),
                        },
                        MutationInputVariable {
                            id: MutationIdInput(2),
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
                        id: MutationIdInput(1),
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
                    expected_requests.push_back((String::from("mutation { upsertManyUser(input: $input, validationSets: $validationSets) { id } }"), String::from(r#"{"input":[{"id":1},{"id":2,"key":"value"}],"validationSets":"empty"}"#))),
                "Oozer" => 
                    expected_requests.push_back((String::from("mutation { upsertManyOozer(input: $input, validationSets: $validationSets) { id } }"), String::from(r#"{"input":[{"id":1}],"validationSets":"default"}"#))),
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
                    variables: DeleteInput {
                        id: MutationIdInput(1),
                    },
                },
                MassMutateEntry {
                    model_name: String::from("User"),
                    variables: DeleteInput {
                        id: MutationIdInput(3),
                    },
                },
                MassMutateEntry {
                    model_name: String::from("Oozer"),
                    variables: DeleteInput {
                        id: MutationIdInput(-1),
                    },
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
                    String::from(r#"{"input":{"ids":[1,2]}}"#),
                )),
                "Oozer" => expected_requests.push_back((
                    String::from("mutation { deleteManyOozer(input: $input) { id } }"),
                    String::from(r#"{"input":{"ids":[1]}}"#),
                )),
                _ => panic!("Unexpected request model"),
            }
        }

        let request_raw_mock = RequestRawMock::with_expect(expected_requests, responses);

        send_delete_many_mutations(delete_many_inputs, &request_raw_mock).unwrap();
    }
}
