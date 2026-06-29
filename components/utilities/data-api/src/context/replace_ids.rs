use crate::context::{RealId, data_structs::*};

pub fn replace_negative_ids_in_mutation_input(
    reserved_ids: &[RealId],
    MutationInputVariable {
        id: MutationIdInput(id),
        other_inputs,
    }: &mut MutationInputVariable,
) -> Result<(), String> {
    replace_id(reserved_ids, id)?;

    replace_negative_ids_in_object(reserved_ids, other_inputs)?;

    Ok(())
}

pub fn replace_id(reserved_ids: &[RealId], id: &mut isize) -> Result<(), String> {
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
    use super::*;

    #[test]
    fn negative_id_replacement_test() {
        let mut mutation_input: MutationInput = serde_json::from_str(r#"{"input": {"id": -3, "key": "value", "relation": {"_replace": [{"id": -1}, {"id": 1}]}, "relation2": {"_add": [{"id": -2}]}}}"#).unwrap();

        replace_negative_ids_in_mutation_input(&[2, 3, 4], &mut mutation_input.input).unwrap();

        assert_eq!(mutation_input.input.id.0, 4);
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
}
