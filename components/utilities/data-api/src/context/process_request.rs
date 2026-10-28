use crate::{
    context::{
        CaptureData, InternalId, MassMutateEntries, MassMutateEntry, ModelName, RequestRaw,
        data_structs::*,
    },
    exports::betty_blocks_types::data_api::data_api::JsonString,
};

enum Operation {
    Create,
    Update,
    Delete,
    Upsert,
}

struct Mutation<'a> {
    name: &'a str,
    operation: Operation,
    model_name: ModelName,
}

pub fn extract_mutation_data(
    request_sender: &impl RequestRaw,
    capture_data: &mut CaptureData,
    query: String,
    variables: JsonString,
) -> Result<JsonString, String> {
    let Some(Mutation {
        name,
        operation,
        model_name,
    }) = read_mutation(&query)?
    else {
        return request_sender.request_raw(query, variables);
    };

    let id = match operation {
        Operation::Create => capture_create(capture_data, model_name, &variables),
        Operation::Update => capture_update(capture_data, model_name, &variables),
        Operation::Delete => capture_delete(capture_data, model_name, &variables),
        Operation::Upsert => capture_upsert(capture_data, model_name, &variables),
    }?;

    Ok(generate_delayed_id_response(&id.to_string(), name))
}

/// Returns `None` for everything the capture does not hold back: queries, `...Many` mutations and
/// operations other than create, update, delete and upsert.
fn read_mutation(query: &str) -> Result<Option<Mutation<'_>>, String> {
    let query_remainder = match query.split_once("mutation") {
        Some((_, remainder)) => remainder,
        None => return Ok(None),
    };

    let query_remainder = query_remainder
        .split_once('{')
        .ok_or_else(|| String::from("query is improperly formatted"))?
        .1
        .trim();

    if query_remainder[6..10] == *"Many" {
        return Ok(None);
    }

    let (mutation_name_with_whitespace, _) = query_remainder
        .split_once('(')
        .ok_or_else(|| String::from("query is improperly formatted"))?;

    let name = mutation_name_with_whitespace.trim();

    let model_name = name[6..].to_string();

    let operation = match &name[..6] {
        "create" => Operation::Create,
        "update" => Operation::Update,
        "delete" => Operation::Delete,
        "upsert" => Operation::Upsert,
        _ => return Ok(None),
    };

    Ok(Some(Mutation {
        name,
        operation,
        model_name,
    }))
}

fn capture_create(
    capture_data: &mut CaptureData,
    model_name: ModelName,
    variables: &str,
) -> Result<InternalId, String> {
    let mut variables: MutationInput = serde_json::from_str(variables)
        .map_err(|error| format!("create mutation variables are improperly formatted: {error}"))?;

    let id = match *variables.input.id {
        0 => placeholder_of_new_record(capture_data, &model_name)?,
        id if id.is_positive() => id,
        _ => {
            return Err(String::from(
                "create mutations cannot specify a negative id",
            ));
        }
    };
    variables.input.id = MutationIdInput(id);

    let capture = current_capture(capture_data);
    let entry = capture.new_entry(model_name, variables);
    capture.create.push(entry);

    Ok(id)
}

fn capture_update(
    capture_data: &mut CaptureData,
    model_name: ModelName,
    variables: &str,
) -> Result<InternalId, String> {
    let variables = serde_json::from_str::<UpdateMutationInput>(variables)
        .map(MutationInput::from)
        .map_err(|error| format!("update mutation variables are improperly formatted: {error}"))?;

    if *variables.input.id == 0 {
        return Err(String::from("could not find id input for update query"));
    }

    capture_update_by_id(capture_data, model_name, variables)
}

/// Captures an update, or an upsert with `uniqueBy: ["id"]`. When the same capture upserts the
/// record by a unique value, the update goes into the row of that upsert. An update by a real id
/// can only be matched to an upsert at apply, so it keeps its capture order.
fn capture_update_by_id(
    capture_data: &mut CaptureData,
    model_name: ModelName,
    variables: MutationInput,
) -> Result<InternalId, String> {
    let MutationIdInput(id) = variables.input.id;
    let capture = current_capture(capture_data);
    let upsert_of_same_record = capture
        .upsert
        .iter_mut()
        .rev()
        .find(|entry| id.is_negative() && *entry.variables.mutation_input.input.id == id);

    match upsert_of_same_record {
        Some(upsert) => merge_update_into_upsert(upsert, variables)?,
        None => {
            let entry = capture.new_entry(model_name, variables);
            capture.update.push(entry);
        }
    }

    Ok(id)
}

fn capture_delete(
    capture_data: &mut CaptureData,
    model_name: ModelName,
    variables: &str,
) -> Result<InternalId, String> {
    let variables @ DeleteInput {
        id: MutationIdInput(id),
    } = serde_json::from_str(variables)
        .map_err(|error| format!("delete mutation variables are improperly formatted: {error}"))?;

    let capture = current_capture(capture_data);
    let entry = capture.new_entry(model_name, variables);
    capture.delete.push(entry);

    Ok(id)
}

fn capture_upsert(
    capture_data: &mut CaptureData,
    model_name: ModelName,
    variables: &str,
) -> Result<InternalId, String> {
    let variables: UpsertMutationInput = serde_json::from_str(variables)
        .map_err(|error| format!("upsert mutation variables are improperly formatted: {error}"))?;

    let [field] = variables.unique_by.as_slice() else {
        return Err(String::from(
            "upsert mutations need exactly one uniqueBy field",
        ));
    };

    if field == "id" {
        capture_upsert_by_id(capture_data, model_name, variables.mutation_input)
    } else {
        let field = field.clone();
        capture_upsert_by_unique_value(capture_data, model_name, field, variables)
    }
}

fn capture_upsert_by_id(
    capture_data: &mut CaptureData,
    model_name: ModelName,
    variables: MutationInput,
) -> Result<InternalId, String> {
    if *variables.input.id == 0 {
        return Err(String::from("could not find id input for upsert query"));
    }

    capture_update_by_id(capture_data, model_name, variables)
}

fn capture_upsert_by_unique_value(
    capture_data: &mut CaptureData,
    model_name: ModelName,
    field: String,
    mut variables: UpsertMutationInput,
) -> Result<InternalId, String> {
    if *variables.mutation_input.input.id != 0 {
        return Err(String::from(
            "an upsert with uniqueBy cannot also specify an id",
        ));
    }

    let value = variables
        .mutation_input
        .input
        .other_inputs
        .get(&field)
        .cloned()
        .unwrap_or_default();

    let internal_id = match UpsertKey::from_unique_value(&model_name, &field, &value) {
        Some(key) => placeholder_of_upserted_record(capture_data, &model_name, key, value)?,
        None => placeholder_of_new_record(capture_data, &model_name)?,
    };
    variables.mutation_input.input.id = MutationIdInput(internal_id);

    let capture = current_capture(capture_data);
    let entry = capture.new_entry(model_name, variables);
    capture.upsert.push(entry);

    Ok(internal_id)
}

fn placeholder_of_new_record(
    capture_data: &mut CaptureData,
    model_name: &str,
) -> Result<InternalId, String> {
    *capture_data
        .reserve_id_count_per_model
        .entry(model_name.to_string())
        .or_default() += 1;

    capture_data.new_internal_id(model_name)
}

fn placeholder_of_upserted_record(
    capture_data: &mut CaptureData,
    model_name: &str,
    key: UpsertKey,
    value: serde_json::Value,
) -> Result<InternalId, String> {
    if let Some(&internal_id) = capture_data.upsert_ids_by_unique_value.get(&key) {
        return Ok(internal_id);
    }

    let internal_id = capture_data.new_internal_id(model_name)?;
    capture_data
        .upsert_ids_by_unique_value
        .insert(key.clone(), internal_id);
    capture_data
        .unique_values_to_look_up
        .insert(internal_id, (key, value));

    Ok(internal_id)
}

pub fn merge_update_into_upsert(
    upsert: &mut MassMutateEntry<UpsertMutationInput>,
    MutationInput {
        input: update_input,
        validation_sets,
    }: MutationInput,
) -> Result<(), String> {
    let upsert_input = &mut upsert.variables.mutation_input;

    // An upsert without a unique value is written by its id, so only an upsert that is found by
    // its unique value must keep that value.
    for field in &upsert.variables.unique_by {
        let upserted_key = upsert_input
            .input
            .other_inputs
            .get(field)
            .and_then(|value| UpsertKey::from_unique_value(&upsert.model_name, field, value));

        if let Some(upserted_key) = upserted_key
            && let Some(new_value) = update_input.other_inputs.get(field)
            && UpsertKey::from_unique_value(&upsert.model_name, field, new_value)
                != Some(upserted_key)
        {
            return Err(format!(
                "an update cannot change {field} of a record upserted in the same mass mutate"
            ));
        }
    }

    upsert_input
        .input
        .other_inputs
        .extend(update_input.other_inputs);
    upsert_input.validation_sets.combine(validation_sets);

    Ok(())
}

fn current_capture(capture_data: &mut CaptureData) -> &mut MassMutateEntries {
    // request() only calls extract_mutation_data while a capture is open, so the stack is never
    // empty here.
    capture_data.capture_stack.last_mut().unwrap()
}

pub fn generate_delayed_id_response(id: &str, mutation_name: &str) -> JsonString {
    format!(r#"{{"data":{{"{mutation_name}":{{"id":"{id}"}}}}}}"#)
}

#[cfg(test)]
mod tests {
    use crate::context::tests::RequestRawMock;

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
                ..
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
                ..
            },
        ] = capture_data.capture_stack.try_into().unwrap();

        assert!(capture_data.reserved_ids.is_empty());
        assert!(capture_data.model_names_of_local_ids.is_empty());
        assert!(capture_data.reserve_id_count_per_model.is_empty());

        let [
            MassMutateEntry {
                model_name: create_model_name,
                variables: create_variables,
                ..
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
                ..
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
    fn validation_sets_list_test() {
        let mut capture_data = CaptureData::default();
        capture_data.capture_stack.push(Default::default());

        let request_raw_mock = RequestRawMock::with_expect(Default::default(), Default::default());

        let mutation = String::from(
            r#"mutation($input: UserInput, $validationSets: [String]) {
            createUser(input: $input, validationSets: $validationSets) {
                id
            }
        }"#,
        );

        for variables in [
            r#"{"input": {"name": "a"}, "validationSets": ["default"]}"#,
            r#"{"input": {"name": "b"}, "validationSets": ["empty"]}"#,
        ] {
            extract_mutation_data(
                &request_raw_mock,
                &mut capture_data,
                mutation.clone(),
                String::from(variables),
            )
            .unwrap();
        }

        let validation_sets: Vec<ValidationSets> = capture_data.capture_stack[0]
            .create
            .iter()
            .map(|entry| entry.variables.validation_sets)
            .collect();

        assert_eq!(
            validation_sets,
            [ValidationSets::Default, ValidationSets::Empty]
        );

        assert_eq!(
            extract_mutation_data(
                &request_raw_mock,
                &mut capture_data,
                mutation,
                String::from(r#"{"input": {}, "validationSets": ["custom"]}"#),
            )
            .unwrap_err()
            .as_str(),
            "create mutation variables are improperly formatted: \
             unsupported validation set: custom at line 1 column 43"
        );
    }

    #[test]
    fn update_with_top_level_id_test() {
        let mut capture_data = CaptureData::default();
        capture_data.capture_stack.push(Default::default());

        let request_raw_mock = RequestRawMock::with_expect(Default::default(), Default::default());

        let mutation = String::from(
            r#"mutation($id: Int!, $input: UserInput, $validationSets: [String]) {
            updateUser(id: $id, input: $input, validationSets: $validationSets) {
                id
            }
        }"#,
        );

        assert_eq!(
            extract_mutation_data(
                &request_raw_mock,
                &mut capture_data,
                mutation.clone(),
                String::from(
                    r#"{"id": "5", "input": {"name": "Joe"}, "validationSets": ["empty"]}"#
                ),
            )
            .unwrap()
            .as_str(),
            r#"{"data":{"updateUser":{"id":"5"}}}"#
        );

        assert_eq!(
            extract_mutation_data(
                &request_raw_mock,
                &mut capture_data,
                mutation,
                String::from(r#"{"input": {"name": "Joe"}}"#),
            )
            .unwrap_err()
            .as_str(),
            "could not find id input for update query"
        );

        let [
            MassMutateEntry {
                model_name,
                variables:
                    MutationInput {
                        input: MutationInputVariable { id, other_inputs },
                        validation_sets,
                    },
                ..
            },
        ] = capture_data
            .capture_stack
            .pop()
            .unwrap()
            .update
            .try_into()
            .unwrap();

        assert_eq!(model_name.as_str(), "User");
        assert_eq!(id, MutationIdInput(5));
        assert_eq!(other_inputs.get("name").unwrap(), "Joe");
        assert_eq!(validation_sets, ValidationSets::Empty);
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
                ..
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
                ..
            },
            MassMutateEntry {
                model_name: create_model_name2,
                variables: create_variables2,
                ..
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

    #[test]
    fn update_cannot_change_unique_value_of_upsert_test() {
        let mut capture_data = CaptureData::default();
        capture_data.capture_stack.push(Default::default());

        let request_raw_mock = RequestRawMock::with_expect(Default::default(), Default::default());

        extract_mutation_data(
            &request_raw_mock,
            &mut capture_data,
            String::from("mutation { upsertUser(input: $input, uniqueBy: $uniqueBy) { id } }"),
            String::from(r#"{"input": {"email": "ann@example.com"}, "uniqueBy": ["email"]}"#),
        )
        .unwrap();

        assert_eq!(
            extract_mutation_data(
                &request_raw_mock,
                &mut capture_data,
                String::from("mutation { updateUser(id: $id, input: $input) { id } }"),
                String::from(r#"{"id": -1, "input": {"email": "bob@example.com"}}"#),
            )
            .unwrap_err(),
            "an update cannot change email of a record upserted in the same mass mutate"
        );
    }

    #[test]
    fn upsert_by_unique_value_cannot_specify_an_id_test() {
        let mut capture_data = CaptureData::default();
        capture_data.capture_stack.push(Default::default());

        let request_raw_mock = RequestRawMock::with_expect(Default::default(), Default::default());

        assert_eq!(
            extract_mutation_data(
                &request_raw_mock,
                &mut capture_data,
                String::from("mutation { upsertUser(input: $input, uniqueBy: $uniqueBy) { id } }"),
                String::from(
                    r#"{"input": {"id": 12, "email": "ann@example.com"}, "uniqueBy": ["email"]}"#
                ),
            )
            .unwrap_err(),
            "an upsert with uniqueBy cannot also specify an id"
        );

        assert!(capture_data.capture_stack[0].upsert.is_empty());
        assert!(capture_data.model_names_of_local_ids.is_empty());
    }

    #[test]
    fn upsert_by_id_goes_into_upsert_of_same_record_test() {
        let mut capture_data = CaptureData::default();
        capture_data.capture_stack.push(Default::default());

        let request_raw_mock = RequestRawMock::with_expect(Default::default(), Default::default());
        let mutation =
            String::from("mutation { upsertUser(input: $input, uniqueBy: $uniqueBy) { id } }");

        for variables in [
            r#"{"input": {"email": "ann@example.com", "name": "Ann"}, "uniqueBy": ["email"]}"#,
            r#"{"input": {"id": -1, "name": "Annie"}, "uniqueBy": ["id"], "validationSets": ["empty"]}"#,
        ] {
            assert_eq!(
                extract_mutation_data(
                    &request_raw_mock,
                    &mut capture_data,
                    mutation.clone(),
                    String::from(variables),
                )
                .unwrap(),
                r#"{"data":{"upsertUser":{"id":"-1"}}}"#
            );
        }

        let MassMutateEntries { update, upsert, .. } = capture_data.capture_stack.pop().unwrap();
        assert!(update.is_empty());

        let [
            MassMutateEntry {
                variables:
                    UpsertMutationInput {
                        mutation_input:
                            MutationInput {
                                input,
                                validation_sets,
                            },
                        ..
                    },
                ..
            },
        ] = upsert.try_into().unwrap();
        assert_eq!(input.other_inputs.get("name").unwrap(), "Annie");
        assert_eq!(validation_sets, ValidationSets::Empty);
    }

    #[test]
    fn update_can_set_unique_value_of_upsert_without_one_test() {
        let mut capture_data = CaptureData::default();
        capture_data.capture_stack.push(Default::default());

        let request_raw_mock = RequestRawMock::with_expect(Default::default(), Default::default());

        extract_mutation_data(
            &request_raw_mock,
            &mut capture_data,
            String::from("mutation { upsertUser(input: $input, uniqueBy: $uniqueBy) { id } }"),
            String::from(r#"{"input": {"email": null, "name": "Ann"}, "uniqueBy": ["email"]}"#),
        )
        .unwrap();

        extract_mutation_data(
            &request_raw_mock,
            &mut capture_data,
            String::from("mutation { updateUser(id: $id, input: $input) { id } }"),
            String::from(r#"{"id": -1, "input": {"email": "ann@example.com"}}"#),
        )
        .unwrap();

        let upsert = &capture_data.capture_stack[0].upsert[0];
        assert_eq!(
            upsert
                .variables
                .mutation_input
                .input
                .other_inputs
                .get("email")
                .unwrap(),
            "ann@example.com"
        );
        assert!(capture_data.unique_values_to_look_up.is_empty());
    }
}
