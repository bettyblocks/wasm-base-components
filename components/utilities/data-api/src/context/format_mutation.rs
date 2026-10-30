use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};

use crate::{
    context::{
        CaptureData, InternalId, MassMutateEntry, ModelName, RealId, RequestRaw,
        data_structs::*,
        process_request::merge_update_into_upsert,
        replace_ids::{replace_id, replace_negative_ids_in_mutation_input},
    },
    exports::betty_blocks_types::data_api::data_api::PendingMutation,
};

const CHUNK_SIZE: usize = 100_000;

const DATA_API_TAKE_LIMIT: usize = 200;

const RESERVE_ID_QUERY: &str = "mutation ($model: String!, $amount: Int!) { reserveRecords(model: $model, amount: $amount) { ids } }";

pub type FoundRecords = HashMap<InternalId, RealId>;

pub type NewRecordIds = HashMap<ModelName, VecDeque<RealId>>;

#[derive(Debug, Default)]
pub struct ResolvedIds {
    pub real_id_per_placeholder: Vec<RealId>,
    pub existing_record_ids: HashSet<InternalId>,
}

impl CaptureData {
    pub fn new_internal_id(&mut self, model_name: &str) -> Result<InternalId, String> {
        let internal_id = internal_id_of_reserved_index(
            self.reserved_ids.len() + self.model_names_of_local_ids.len(),
        )?;
        self.model_names_of_local_ids.push(model_name.to_string());
        Ok(internal_id)
    }

    pub fn look_up_existing_records(
        &self,
        request_sender: &impl RequestRaw,
    ) -> Result<FoundRecords, String> {
        let mut found_records = FoundRecords::new();

        for ((model_name, field), values) in self.unique_values_per_model_and_field() {
            for values in values.chunks(DATA_API_TAKE_LIMIT) {
                for (key, id) in find_records(request_sender, model_name, field, values)? {
                    if let Some(&internal_id) = self.upsert_ids_by_unique_value.get(&key) {
                        found_records.insert(internal_id, id);
                    }
                }
            }
        }

        Ok(found_records)
    }

    pub fn reserve_new_record_ids(
        &mut self,
        found_records: &FoundRecords,
        request_sender: &impl RequestRaw,
    ) -> Result<NewRecordIds, String> {
        for (internal_id, (key, _)) in &self.unique_values_to_look_up {
            if !found_records.contains_key(internal_id) {
                *self
                    .reserve_id_count_per_model
                    .entry(key.model.clone())
                    .or_default() += 1;
            }
        }

        self.reserve_id_count_per_model
            .drain()
            .map(|(model, amount)| {
                Self::request_reserved_ids(
                    request_sender,
                    ReserveIdMutationVariables { model, amount },
                )
            })
            .collect()
    }

    pub fn resolve_placeholders(
        &mut self,
        found_records: &FoundRecords,
        mut new_record_ids: NewRecordIds,
    ) -> Result<ResolvedIds, String> {
        for model_name in std::mem::take(&mut self.model_names_of_local_ids) {
            let internal_id = internal_id_of_reserved_index(self.reserved_ids.len())?;

            let real_id = match found_records.get(&internal_id) {
                Some(&existing_id) => {
                    self.existing_record_ids.insert(internal_id);
                    existing_id
                }
                None => take_new_record_id(&mut new_record_ids, &model_name)?,
            };

            self.unique_values_to_look_up.remove(&internal_id);
            self.reserved_ids.push(real_id);
        }

        let outermost_capture_is_applied = self.capture_stack.is_empty();

        if outermost_capture_is_applied {
            self.upsert_ids_by_unique_value.clear();
            Ok(ResolvedIds {
                real_id_per_placeholder: std::mem::take(&mut self.reserved_ids),
                existing_record_ids: std::mem::take(&mut self.existing_record_ids),
            })
        } else {
            Ok(ResolvedIds {
                real_id_per_placeholder: self.reserved_ids.clone(),
                existing_record_ids: self.existing_record_ids.clone(),
            })
        }
    }

    /// An outer upsert of the same value must then find the record instead of inserting it again.
    pub fn mark_upserts_as_existing_for_outer_capture(
        &mut self,
        written_ids: impl IntoIterator<Item = InternalId>,
    ) {
        if !self.capture_stack.is_empty() {
            self.existing_record_ids.extend(written_ids);
        }
    }

    pub fn forget_discarded_upserts(
        &mut self,
        discarded_upserts: Vec<MassMutateEntry<UpsertMutationInput>>,
    ) {
        let still_captured: HashSet<InternalId> = self
            .capture_stack
            .iter()
            .flat_map(|capture| &capture.upsert)
            .map(|entry| *entry.variables.mutation_input.input.id)
            .collect();

        for entry in discarded_upserts {
            let internal_id = *entry.variables.mutation_input.input.id;
            if still_captured.contains(&internal_id) {
                continue;
            }

            if let Some((key, _)) = self.unique_values_to_look_up.remove(&internal_id) {
                self.upsert_ids_by_unique_value.remove(&key);
                // The placeholder keeps its place in the list, so it still gets a reserved id
                // that is not used.
                *self
                    .reserve_id_count_per_model
                    .entry(key.model)
                    .or_default() += 1;
            }
        }

        if self.capture_stack.is_empty() {
            self.upsert_ids_by_unique_value.clear();
            self.existing_record_ids.clear();
        }
    }

    fn unique_values_per_model_and_field(&self) -> BTreeMap<(&str, &str), Vec<&serde_json::Value>> {
        let mut keys_and_values: Vec<_> = self.unique_values_to_look_up.values().collect();
        keys_and_values.sort_by_key(|(key, _)| key);

        let mut values_per_model_and_field: BTreeMap<_, Vec<_>> = BTreeMap::new();
        for (key, value) in keys_and_values {
            values_per_model_and_field
                .entry((key.model.as_str(), key.field.as_str()))
                .or_default()
                .push(value);
        }

        values_per_model_and_field
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

fn take_new_record_id(
    new_record_ids: &mut HashMap<ModelName, VecDeque<RealId>>,
    model_name: &str,
) -> Result<RealId, String> {
    new_record_ids
        .get_mut(model_name)
        .ok_or_else(|| format!("ids for model {model_name} were not properly reserved"))?
        .pop_front()
        .ok_or_else(|| format!("not enough ids were reserved for model {model_name}"))
}

fn find_records(
    request_sender: &impl RequestRaw,
    model_name: &str,
    field: &str,
    values: &[&serde_json::Value],
) -> Result<Vec<(UpsertKey, RealId)>, String> {
    let query = format!(
        "query($where: {model_name}FilterInput) {{ all{model_name}(where: $where, take: {DATA_API_TAKE_LIMIT}) {{ results {{ id {field} }} }} }}"
    );

    let mut filter = serde_json::Map::new();
    filter.insert(field.to_string(), serde_json::json!({ "in": values }));
    let variables = serde_json::to_string(&serde_json::json!({ "where": filter }))
        .map_err(|_| String::from("could not format lookup variables"))?;

    let response = request_sender.request_raw(query, variables)?;

    read_found_records(&response, model_name, field)
}

fn read_found_records(
    response: &str,
    model_name: &str,
    field: &str,
) -> Result<Vec<(UpsertKey, RealId)>, String> {
    let parse_error = || format!("could not parse the lookup of {model_name} records");

    let response: serde_json::Value = serde_json::from_str(response).map_err(|_| parse_error())?;
    if let Some(errors) = response["errors"].as_array()
        && !errors.is_empty()
    {
        return Err(format!(
            "lookup of {model_name} records failed: {}",
            response["errors"]
        ));
    }

    let records = response["data"][format!("all{model_name}")]["results"]
        .as_array()
        .ok_or_else(parse_error)?;

    let mut found_records = Vec::new();
    for record in records {
        let Some(key) = UpsertKey::from_unique_value(model_name, field, &record[field]) else {
            continue;
        };
        let id = record_id(record)
            .ok_or_else(|| format!("lookup of {model_name} returned an invalid id"))?;
        found_records.push((key, id));
    }

    Ok(found_records)
}

fn record_id(record: &serde_json::Value) -> Option<RealId> {
    record["id"]
        .as_i64()
        .or_else(|| record["id"].as_str().and_then(|id| id.parse().ok()))
        .and_then(|id| RealId::try_from(id).ok())
}

fn format_mutation(model_name: &str, operation: &str) -> String {
    let arguments = match operation {
        "delete" => "",
        "upsert" => ", uniqueBy: $uniqueBy, validationSets: $validationSets",
        _ => ", validationSets: $validationSets",
    };

    format!(
        "mutation {{ {}{model_name}(input: $input{arguments}) {{ id }} }}",
        operation
    )
}

fn internal_id_of_reserved_index(index: usize) -> Result<InternalId, String> {
    InternalId::try_from(index + 1)
        .map(|id| -id)
        .map_err(|_| String::from("ran out of internal ids"))
}

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
struct UpsertBatch {
    model_name: ModelName,
    unique_by: Option<String>,
    fields: Vec<String>,
}

struct UpsertRow {
    model_name: ModelName,
    unique_by: String,
    validation_sets: ValidationSets,
    input: MutationInputVariable,
}

type UpsertBatches = BTreeMap<UpsertBatch, (ValidationSets, Vec<serde_json::Value>)>;

type UpdatesAndUpserts = (
    Vec<MassMutateEntry<MutationInput>>,
    Vec<MassMutateEntry<UpsertMutationInput>>,
);

/// An update by real id goes into the row of an earlier upsert that found that record.
pub fn merge_updates_into_found_upserts(
    updates: Vec<MassMutateEntry<MutationInput>>,
    mut upserts: Vec<MassMutateEntry<UpsertMutationInput>>,
    resolved_ids: &ResolvedIds,
) -> Result<UpdatesAndUpserts, String> {
    let mut remaining_updates = Vec::new();

    for update in updates {
        match found_upsert_of_record(&mut upserts, &update, resolved_ids)? {
            Some(upsert) => merge_update_into_upsert(upsert, update.variables)?,
            None => remaining_updates.push(update),
        }
    }

    Ok((remaining_updates, upserts))
}

fn found_upsert_of_record<'a>(
    upserts: &'a mut [MassMutateEntry<UpsertMutationInput>],
    update: &MassMutateEntry<MutationInput>,
    resolved_ids: &ResolvedIds,
) -> Result<Option<&'a mut MassMutateEntry<UpsertMutationInput>>, String> {
    for upsert in upserts.iter_mut().rev() {
        let mut id = *upsert.variables.mutation_input.input.id;
        if upsert.capture_order > update.capture_order
            || upsert.model_name != update.model_name
            || !resolved_ids.existing_record_ids.contains(&id)
        {
            continue;
        }

        replace_id(&resolved_ids.real_id_per_placeholder, &mut id)?;
        if id == *update.variables.input.id {
            return Ok(Some(upsert));
        }
    }

    Ok(None)
}

pub fn send_creates_and_updates(
    create_entries: Vec<MassMutateEntry<MutationInput>>,
    update_entries: Vec<MassMutateEntry<MutationInput>>,
    resolved_ids: &ResolvedIds,
    request_sender: &impl RequestRaw,
) -> Result<(), String> {
    send_upsert_many_mutations(
        generate_upsert_many_inputs(
            create_entries,
            update_entries,
            &resolved_ids.real_id_per_placeholder,
        )?,
        request_sender,
    )
}

pub fn send_upserts(
    upsert_entries: Vec<MassMutateEntry<UpsertMutationInput>>,
    resolved_ids: &ResolvedIds,
    request_sender: &impl RequestRaw,
) -> Result<(), String> {
    let rows = merge_upserts_of_the_same_record(upsert_entries);
    let batches = group_into_batches(rows, resolved_ids)?;

    for (batch, (validation_sets, rows)) in batches {
        send_upsert_batch(request_sender, &batch, validation_sets, &rows)?;
    }

    Ok(())
}

pub fn send_deletes(
    delete_entries: Vec<MassMutateEntry<DeleteInput>>,
    resolved_ids: &ResolvedIds,
    request_sender: &impl RequestRaw,
) -> Result<(), String> {
    send_delete_many_mutations(
        generate_delete_many_inputs(delete_entries, &resolved_ids.real_id_per_placeholder)?,
        request_sender,
    )
}

fn merge_upserts_of_the_same_record(
    upsert_entries: Vec<MassMutateEntry<UpsertMutationInput>>,
) -> Vec<UpsertRow> {
    let mut rows: Vec<UpsertRow> = Vec::new();
    let mut row_of_record: HashMap<InternalId, usize> = HashMap::new();

    for MassMutateEntry {
        model_name,
        variables:
            UpsertMutationInput {
                mutation_input:
                    MutationInput {
                        input,
                        validation_sets,
                    },
                unique_by,
            },
        ..
    } in upsert_entries
    {
        match row_of_record.get(&*input.id) {
            Some(&index) => {
                let row = &mut rows[index];
                row.input.other_inputs.extend(input.other_inputs);
                row.validation_sets.combine(validation_sets);
            }
            None => {
                row_of_record.insert(*input.id, rows.len());
                rows.push(UpsertRow {
                    model_name,
                    unique_by: unique_by.into_iter().next().unwrap_or_default(),
                    validation_sets,
                    input,
                });
            }
        }
    }

    rows
}

fn group_into_batches(
    rows: Vec<UpsertRow>,
    resolved_ids: &ResolvedIds,
) -> Result<UpsertBatches, String> {
    let mut batches = UpsertBatches::new();

    for UpsertRow {
        model_name,
        unique_by,
        validation_sets,
        mut input,
    } in rows
    {
        let is_existing_record = resolved_ids.existing_record_ids.contains(&input.id);
        replace_negative_ids_in_mutation_input(&resolved_ids.real_id_per_placeholder, &mut input)?;

        let mut row = input.other_inputs;
        if !is_existing_record {
            row.insert(String::from("id"), serde_json::Value::from(*input.id));
        }

        let batch = UpsertBatch {
            model_name,
            unique_by: is_existing_record.then_some(unique_by),
            fields: row.keys().cloned().collect(),
        };
        let (batch_validation_sets, batch_rows) = batches
            .entry(batch)
            .or_insert((ValidationSets::Default, Vec::new()));
        batch_validation_sets.combine(validation_sets);
        batch_rows.push(serde_json::Value::Object(row));
    }

    Ok(batches)
}

fn send_upsert_batch(
    request_sender: &impl RequestRaw,
    UpsertBatch {
        model_name,
        unique_by,
        ..
    }: &UpsertBatch,
    validation_sets: ValidationSets,
    rows: &[serde_json::Value],
) -> Result<(), String> {
    let unique_by_argument = if unique_by.is_some() {
        ", uniqueBy: $uniqueBy"
    } else {
        ""
    };
    let query = format!(
        "mutation {{ upsertMany{model_name}(input: $input{unique_by_argument}, validationSets: $validationSets) {{ id }} }}"
    );

    for rows in rows.chunks(CHUNK_SIZE) {
        let mut variables = serde_json::json!({"input": rows, "validationSets": validation_sets});
        if let Some(field) = unique_by {
            variables["uniqueBy"] = serde_json::json!([field]);
        }
        let variables = serde_json::to_string(&variables)
            .map_err(|_| String::from("could not format input variables"))?;

        request_sender.request_raw(query.clone(), variables)?;
    }

    Ok(())
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
        ..
    } in [create_entries, update_entries].into_iter().flatten()
    {
        replace_negative_ids_in_mutation_input(reserved_ids, &mut variables.input)?;

        let (validation_sets, input_vec) = upsert_manys.entry(model_name).or_default();
        validation_sets.combine(variables.validation_sets);
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
        variables: DeleteInput {
            id: MutationIdInput(mut id),
        },
        ..
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
            let variables = serde_json::to_string(
                &serde_json::json!({"input": input_chunk, "validationSets": validation_sets}),
            )
            .map_err(|_| String::from("could not format input variables"))?;

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
    use crate::context::{
        MassMutateEntries, process_request::extract_mutation_data, tests::RequestRawMock,
    };

    use super::*;

    fn reserve_and_resolve(
        capture_data: &mut CaptureData,
        request_sender: &impl RequestRaw,
    ) -> Vec<RealId> {
        let new_record_ids = capture_data
            .reserve_new_record_ids(&FoundRecords::new(), request_sender)
            .unwrap();
        capture_data
            .resolve_placeholders(&FoundRecords::new(), new_record_ids)
            .unwrap()
            .real_id_per_placeholder
    }

    #[test]
    fn reserve_and_resolve_test() {
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
            ..Default::default()
        };

        let request_raw_mock = RequestRawMock::with_expect(expected_requests, responses);

        let real_ids = reserve_and_resolve(&mut capture_data, &request_raw_mock);

        assert_eq!(real_ids.as_slice(), [3, 1, 2, 4]);
        assert!(capture_data.reserve_id_count_per_model.is_empty());
        assert!(capture_data.reserved_ids.is_empty());
        assert!(capture_data.model_names_of_local_ids.is_empty());
    }

    #[test]
    fn reserve_and_resolve_saves_stacks_test() {
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
            ..Default::default()
        };

        let request_raw_mock = RequestRawMock::with_expect(expected_requests, responses);

        let real_ids = reserve_and_resolve(&mut capture_data, &request_raw_mock);

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

        let real_ids = reserve_and_resolve(&mut capture_data, &request_raw_mock);

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

        let real_ids = reserve_and_resolve(&mut capture_data, &request_raw_mock);

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
                    capture_order: 0,
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
                    capture_order: 0,
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
                    capture_order: 0,
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
                capture_order: 0,
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
                    expected_requests.push_back((String::from("mutation { upsertManyUser(input: $input, validationSets: $validationSets) { id } }"), String::from(r#"{"input":[{"id":1},{"id":2,"key":"value"}],"validationSets":["empty"]}"#))),
                "Oozer" => 
                    expected_requests.push_back((String::from("mutation { upsertManyOozer(input: $input, validationSets: $validationSets) { id } }"), String::from(r#"{"input":[{"id":1}],"validationSets":["default"]}"#))),
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
                    capture_order: 0,
                },
                MassMutateEntry {
                    model_name: String::from("User"),
                    variables: DeleteInput {
                        id: MutationIdInput(3),
                    },
                    capture_order: 0,
                },
                MassMutateEntry {
                    model_name: String::from("Oozer"),
                    variables: DeleteInput {
                        id: MutationIdInput(-1),
                    },
                    capture_order: 0,
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

    #[test]
    fn lookup_returns_graphql_errors_test() {
        let response = r#"{"data":null,"errors":[{"message":"unknown field emial"}]}"#;

        assert_eq!(
            read_found_records(response, "User", "emial").unwrap_err(),
            r#"lookup of User records failed: [{"message":"unknown field emial"}]"#
        );
    }

    /// Captures the mutations, then resolves placeholder -1 to the found record 7.
    fn capture_with_found_record(mutations: &[(&str, &str)]) -> (MassMutateEntries, ResolvedIds) {
        let mut capture_data = CaptureData::default();
        capture_data.capture_stack.push(Default::default());

        for (query, variables) in mutations {
            extract_mutation_data(
                &RequestRawMock::with_expect(Default::default(), Default::default()),
                &mut capture_data,
                String::from(*query),
                String::from(*variables),
            )
            .unwrap();
        }

        let resolved_ids = ResolvedIds {
            real_id_per_placeholder: vec![7],
            existing_record_ids: HashSet::from([-1]),
        };

        (capture_data.capture_stack.pop().unwrap(), resolved_ids)
    }

    #[test]
    fn upsert_by_real_id_after_upsert_goes_into_its_row_test() {
        let (MassMutateEntries { update, upsert, .. }, resolved_ids) = capture_with_found_record(
            &[
                (
                    "mutation { upsertUser(input: $input, uniqueBy: $uniqueBy) { id } }",
                    r#"{"input": {"email": "ann@example.com", "name": "Ann"}, "uniqueBy": ["email"]}"#,
                ),
                (
                    "mutation { upsertUser(input: $input, uniqueBy: $uniqueBy) { id } }",
                    r#"{"input": {"id": 7, "name": "Annie"}, "uniqueBy": ["id"]}"#,
                ),
                (
                    "mutation { updateOrder(id: $id, input: $input) { id } }",
                    r#"{"id": 7, "input": {"number": "A-1"}}"#,
                ),
            ],
        );

        let (remaining_updates, upsert) =
            merge_updates_into_found_upserts(update, upsert, &resolved_ids).unwrap();
        let [remaining_update] = remaining_updates.try_into().unwrap();
        assert_eq!(remaining_update.model_name, "Order");

        let [upsert] = upsert.try_into().unwrap();
        let input = upsert.variables.mutation_input.input;
        assert_eq!(*input.id, -1);
        assert_eq!(input.other_inputs.get("name").unwrap(), "Annie");
    }

    #[test]
    fn update_by_real_id_cannot_change_unique_value_of_upsert_test() {
        let (MassMutateEntries { update, upsert, .. }, resolved_ids) =
            capture_with_found_record(&[
                (
                    "mutation { upsertUser(input: $input, uniqueBy: $uniqueBy) { id } }",
                    r#"{"input": {"email": "ann@example.com"}, "uniqueBy": ["email"]}"#,
                ),
                (
                    "mutation { updateUser(id: $id, input: $input) { id } }",
                    r#"{"id": 7, "input": {"email": "bob@example.com"}}"#,
                ),
            ]);

        assert_eq!(
            merge_updates_into_found_upserts(update, upsert, &resolved_ids).unwrap_err(),
            "an update cannot change email of a record upserted in the same mass mutate"
        );
    }
}
