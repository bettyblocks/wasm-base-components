use crate::context::{RealId, data_structs::*};
use crate::exports::betty_blocks_types::data_api::data_api::JsonString;

/// What to do with a negative ID that does not have a reserved ID (yet).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnknownId {
    /// Applying a capture: every negative ID must have been reserved by now.
    Error,
    /// Passing a request through: the ID can belong to a capture that is not applied yet.
    Keep,
}

pub fn replace_negative_ids_in_mutation_input(
    reserved_ids: &[RealId],
    MutationInputVariable {
        id: MutationIdInput(id),
        other_inputs,
    }: &mut MutationInputVariable,
) -> Result<(), String> {
    replace_id(reserved_ids, id)?;

    replace_negative_ids_in_object(reserved_ids, other_inputs, UnknownId::Error)?;

    Ok(())
}

/// Replaces the negative IDs of earlier applied captures in the variables of a request that is
/// sent straight to the data API, so a record created in one mass mutate can be used after it.
pub fn replace_negative_ids_in_variables(
    reserved_ids: &[RealId],
    variables: JsonString,
) -> Result<JsonString, String> {
    if reserved_ids.is_empty() {
        return Ok(variables);
    }

    // Leave variables that are not a JSON object as they are, the data API reports the error.
    let Ok(serde_json::Value::Object(mut object)) = serde_json::from_str(&variables) else {
        return Ok(variables);
    };

    replace_negative_ids_in_object(reserved_ids, &mut object, UnknownId::Keep)?;

    serde_json::to_string(&object).map_err(|_| String::from("could not format variables"))
}

pub fn replace_id(reserved_ids: &[RealId], id: &mut isize) -> Result<(), String> {
    if id.is_negative() {
        let reserved_id = lookup_reserved_id(reserved_ids, *id as i64)
            .ok_or_else(|| format!("unknown temporary id {id}"))?;

        *id = reserved_id
            .try_into()
            .map_err(|error| format!("could not convert number to isize: {error}"))?;
    }

    Ok(())
}

/// Uses the negative ID to index for its reserved ID.
fn lookup_reserved_id(reserved_ids: &[RealId], negative_id: i64) -> Option<RealId> {
    // If `negative_id` is -1 which is the first possible negative ID, then `-1 - -1 = 0`.
    // Which is the first possible item in the reserved ID list.
    let index: usize = (-1 - negative_id).try_into().ok()?;

    reserved_ids.get(index).copied()
}

/// Loops through the entries of a serde_json::Map and replaces all the negative IDs with their
/// reserved counterparts.
fn replace_negative_ids_in_object(
    reserved_ids: &[RealId],
    variables: &mut serde_json::Map<String, serde_json::Value>,
    unknown_id: UnknownId,
) -> Result<(), String> {
    for (key, value) in variables.iter_mut() {
        if key == "id" {
            handle_id_value(reserved_ids, value, unknown_id)?;
        } else if let serde_json::Value::Object(object) = value {
            replace_negative_ids_in_object(reserved_ids, object, unknown_id)?;
        } else if let serde_json::Value::Array(array_of_values) = value {
            handle_array_of_values(reserved_ids, array_of_values, unknown_id)?;
        }
    }

    Ok(())
}

/// Replaces the negative ID or IDs with their reserved counterpart. An ID can be a number or a
/// string, a list of IDs (has many relations) or a filter on IDs (`{"eq": -1}`).
fn handle_id_value(
    reserved_ids: &[RealId],
    value: &mut serde_json::Value,
    unknown_id: UnknownId,
) -> Result<(), String> {
    match value {
        serde_json::Value::Number(number) => {
            if let Some(id) = number.as_i64()
                && let Some(reserved_id) = resolve_negative_id(reserved_ids, id, unknown_id)?
            {
                *value = serde_json::Value::from(reserved_id);
            }
        }
        serde_json::Value::String(string) => {
            if let Ok(id) = string.parse::<i64>()
                && let Some(reserved_id) = resolve_negative_id(reserved_ids, id, unknown_id)?
            {
                *value = serde_json::Value::String(reserved_id.to_string());
            }
        }
        serde_json::Value::Array(ids) => {
            for id in ids.iter_mut() {
                handle_id_value(reserved_ids, id, unknown_id)?;
            }
        }
        serde_json::Value::Object(filter) => {
            for id in filter.values_mut() {
                handle_id_value(reserved_ids, id, unknown_id)?;
            }
        }
        serde_json::Value::Null | serde_json::Value::Bool(_) => {}
    }

    Ok(())
}

/// Returns the reserved ID for a negative ID, or `None` when the ID is not negative or is unknown
/// and may be kept.
fn resolve_negative_id(
    reserved_ids: &[RealId],
    id: i64,
    unknown_id: UnknownId,
) -> Result<Option<RealId>, String> {
    if !id.is_negative() {
        return Ok(None);
    }

    match (lookup_reserved_id(reserved_ids, id), unknown_id) {
        (Some(reserved_id), _) => Ok(Some(reserved_id)),
        (None, UnknownId::Keep) => Ok(None),
        (None, UnknownId::Error) => Err(format!("unknown temporary id {id}")),
    }
}

/// Looks at the items in an array and if they're an object tries to replace negative IDs in that
/// object, and if they are an array it searches for objects or arrays within that object which
/// might have negative IDs to replace.
fn handle_array_of_values(
    reserved_ids: &[RealId],
    array_of_values: &mut [serde_json::Value],
    unknown_id: UnknownId,
) -> Result<(), String> {
    for item in array_of_values.iter_mut() {
        if let serde_json::Value::Object(object) = item {
            replace_negative_ids_in_object(reserved_ids, object, unknown_id)?;
        } else if let serde_json::Value::Array(array) = item {
            handle_array_of_values(reserved_ids, array, unknown_id)?;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn negative_id_replacement_test() {
        let mut mutation_input: MutationInput = serde_json::from_str(r#"{"input": {"id": -3, "key": "value", "relation": {"_replace": [{"id": -1}, {"id": 1}]}, "relation2": {"_add": [{"id": -2}]}}}"#).unwrap();

        replace_negative_ids_in_mutation_input(&[2, 3, 4], &mut mutation_input.input).unwrap();

        assert_eq!(*mutation_input.input.id, 4);
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
        let serde_json::Value::Array(replace) = relation.remove("_replace").unwrap() else {
            panic!("mutation input relation replace was not an array")
        };
        let [
            serde_json::Value::Object(mut id2),
            serde_json::Value::Object(mut id1),
        ] = TryInto::<[_; 2]>::try_into(replace).unwrap()
        else {
            panic!("mutation input relation replace was not an array of 2 id objects")
        };
        assert_eq!(id1.remove("id").unwrap(), 1);
        assert!(id1.is_empty());
        assert_eq!(id2.remove("id").unwrap(), 2);
        assert!(id2.is_empty());
        assert!(relation.is_empty());
        let serde_json::Value::Object(mut relation2) = mutation_input
            .input
            .other_inputs
            .remove("relation2")
            .unwrap()
        else {
            panic!("mutation input relation2 was not an object")
        };
        let serde_json::Value::Array(add) = relation2.remove("_add").unwrap() else {
            panic!("mutation input relation2 add was not an array")
        };
        let [serde_json::Value::Object(mut id3)] = TryInto::<[_; 1]>::try_into(add).unwrap() else {
            panic!("mutation input relation2 add pop was not an object")
        };
        assert_eq!(id3.remove("id").unwrap(), 3);
        assert!(id3.is_empty());
        assert!(relation2.is_empty());
    }

    #[test]
    fn string_and_list_id_replacement_test() {
        let mut mutation_input: MutationInput =
            serde_json::from_str(r#"{"input": {"id": "-1", "tags": {"id": ["-2", "7", -1]}}}"#)
                .unwrap();

        replace_negative_ids_in_mutation_input(&[5, 6], &mut mutation_input.input).unwrap();

        assert_eq!(*mutation_input.input.id, 5);
        assert_eq!(
            serde_json::Value::Object(mutation_input.input.other_inputs),
            serde_json::json!({"tags": {"id": ["6", "7", 5]}})
        );
    }

    #[test]
    fn unknown_negative_id_test() {
        let mut id = -3;
        assert_eq!(
            replace_id(&[5, 6], &mut id).unwrap_err(),
            "unknown temporary id -3"
        );

        let mut mutation_input: MutationInput =
            serde_json::from_str(r#"{"input": {"relation": {"_add": [{"id": -3}]}}}"#).unwrap();
        assert_eq!(
            replace_negative_ids_in_mutation_input(&[5, 6], &mut mutation_input.input).unwrap_err(),
            "unknown temporary id -3"
        );
    }

    #[test]
    fn variables_id_replacement_test() {
        assert_eq!(
            replace_negative_ids_in_variables(
                &[5],
                String::from(r#"{"id": "-1", "input": {"name": "Joe", "age": -1}}"#)
            )
            .unwrap(),
            r#"{"id":"5","input":{"age":-1,"name":"Joe"}}"#
        );

        assert_eq!(
            replace_negative_ids_in_variables(
                &[5],
                String::from(r#"{"where": {"id": {"eq": "-1", "in": [-1, -2]}}}"#)
            )
            .unwrap(),
            r#"{"where":{"id":{"eq":"5","in":[5,-2]}}}"#
        );

        let untouched = String::from(r#"{"id": "-1"}"#);
        assert_eq!(
            replace_negative_ids_in_variables(&[], untouched.clone()).unwrap(),
            untouched
        );
    }
}
