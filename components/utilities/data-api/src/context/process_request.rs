use crate::{
    context::{CaptureData, MassMutateEntry, RequestRaw, data_structs::*},
    exports::betty_blocks_types::data_api::data_api::JsonString,
};

pub fn generate_delayed_id_response(id: &str, mutation_name: &str) -> JsonString {
    format!(r#"{{"data":{{"{mutation_name}":{{"id":"{id}"}}}}}}"#)
}

pub fn extract_mutation_data(
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
    let id: isize =
        match &mutation_name[..6] {
            "create" => {
                match serde_json::from_str(&variables) {
                    // Create without a specified ID
                    Ok(
                        mut variables @ MutationInput {
                            input:
                                MutationInputVariable {
                                    id: MutationIdInput(0),
                                    ..
                                },
                            ..
                        },
                    ) => {
                        capture_data
                            .model_names_of_local_ids
                            .push(model_name.clone());
                        *capture_data
                            .reserve_id_count_per_model
                            .entry(model_name.clone())
                            .or_default() += 1;

                        let internal_id = -TryInto::<isize>::try_into(
                            capture_data.model_names_of_local_ids.len()
                                + capture_data.reserved_ids.len(),
                        )
                        .map_err(|_| String::from("ran out of internal ids"))?;

                        variables.input.id = MutationIdInput(internal_id);

                        capture_data.capture_stack.last_mut().unwrap().create.push(
                            MassMutateEntry {
                                model_name,
                                variables,
                            },
                        );

                        Ok(internal_id)
                    }
                    // Create with a specified positive ID
                    Ok(
                        variables @ MutationInput {
                            input:
                                MutationInputVariable {
                                    id: MutationIdInput(id),
                                    ..
                                },
                            ..
                        },
                    ) if id.is_positive() => {
                        capture_data.capture_stack.last_mut().unwrap().create.push(
                            MassMutateEntry {
                                model_name,
                                variables,
                            },
                        );

                        Ok(id)
                    }
                    // Create with a specified negative ID
                    Ok(_) => Err("create mutations cannot specify a negative id"),
                    Err(_) => Err("create mutation variables are improperly formatted"),
                }
            }
            "update" => {
                match serde_json::from_str(&variables) {
                    // Update without a specified ID
                    Ok(MutationInput {
                        input:
                            MutationInputVariable {
                                id: MutationIdInput(0),
                                ..
                            },
                        ..
                    }) => Err("could not find id input for update query"),
                    // Update with a specified ID
                    Ok(
                        variables @ MutationInput {
                            input:
                                MutationInputVariable {
                                    id: MutationIdInput(id),
                                    ..
                                },
                            ..
                        },
                    ) => {
                        capture_data.capture_stack.last_mut().unwrap().update.push(
                            MassMutateEntry {
                                model_name,
                                variables,
                            },
                        );

                        Ok(id)
                    }
                    Err(_) => Err("update mutation variables are improperly formatted"),
                }
            }
            "delete" => {
                let variables @ DeleteInput {
                    id: MutationIdInput(id),
                } = serde_json::from_str(&variables).map_err(|_| {
                    String::from("delete mutation variables are improperly formatted")
                })?;

                capture_data
                    .capture_stack
                    .last_mut()
                    .unwrap()
                    .delete
                    .push(MassMutateEntry {
                        model_name,
                        variables,
                    });

                Ok(id)
            }
            _ => return request_sender.request_raw(query, variables),
        }?;

    Ok(generate_delayed_id_response(&id.to_string(), mutation_name))
}

#[cfg(test)]
mod tests {
    use crate::context::{MassMutateEntries, tests::RequestRawMock};

    use super::*;

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

        let [
            MassMutateEntries {
                create,
                update,
                delete,
            },
        ] = capture_data.capture_stack.try_into().unwrap();

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
            r#"{"data":{"createUser":{"id":"2"}}}"#
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
            r#"{"data":{"deleteUser":{"id":"2"}}}"#
        );

        let [
            MassMutateEntries {
                create,
                update,
                delete,
            },
        ] = capture_data.capture_stack.try_into().unwrap();

        assert!(capture_data.reserved_ids.is_empty());
        assert!(capture_data.model_names_of_local_ids.is_empty());
        assert!(capture_data.reserve_id_count_per_model.is_empty());

        let [
            MassMutateEntry {
                model_name: create_model_name,
                variables: create_variables,
            },
        ] = create.try_into().unwrap();
        assert_eq!(create_model_name.as_str(), "User");
        assert!(
            matches!(create_variables, MutationInput { input: MutationInputVariable { id: MutationIdInput(2), other_inputs }, validation_sets: ValidationSets::Default } if other_inputs.is_empty())
        );
        let [
            MassMutateEntry {
                model_name: delete_model_name,
                variables: delete_variables,
            },
        ] = delete.try_into().unwrap();
        assert_eq!(delete_model_name.as_str(), "User");
        assert!(matches!(
            delete_variables,
            DeleteInput {
                id: MutationIdInput(2)
            }
        ));
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
            r#"{"data":{"createUser":{"id":"-1"}}}"#
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
            r#"{"data":{"createUser":{"id":"-2"}}}"#
        );

        let [
            MassMutateEntries {
                create,
                update,
                delete,
            },
        ] = capture_data.capture_stack.try_into().unwrap();

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

        let [
            MassMutateEntry {
                model_name: create_model_name1,
                variables: create_variables1,
            },
            MassMutateEntry {
                model_name: create_model_name2,
                variables: create_variables2,
            },
        ] = create.try_into().unwrap();
        assert_eq!(create_model_name1.as_str(), "User");
        assert!(
            matches!(create_variables1, MutationInput { input: MutationInputVariable { id: MutationIdInput(-1), other_inputs }, validation_sets: ValidationSets::Default } if other_inputs.is_empty())
        );
        assert_eq!(create_model_name2.as_str(), "User");
        assert!(
            matches!(create_variables2, MutationInput { input: MutationInputVariable { id: MutationIdInput(-2), other_inputs }, validation_sets: ValidationSets::Default } if other_inputs.is_empty())
        );
        assert!(delete.is_empty());
        assert!(update.is_empty());
    }
}
