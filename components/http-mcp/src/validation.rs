use jsonschema::validator_for;
use rust_mcp_schema::ToolInputSchema;
use serde_json::Value;

pub fn validate_arguments(
    arguments: Option<&serde_json::Map<String, Value>>,
    schema: &ToolInputSchema,
) -> Result<(), String> {
    let args_value = arguments
        .map(|m| Value::Object(m.clone()))
        .unwrap_or_else(|| Value::Object(serde_json::Map::new()));

    let schema_value =
        serde_json::to_value(schema).map_err(|e| format!("Failed to serialize schema: {e}"))?;

    let validator = validator_for(&schema_value).map_err(|e| format!("Invalid schema: {e}"))?;

    if validator.is_valid(&args_value) {
        Ok(())
    } else {
        let errors: Vec<String> = validator
            .iter_errors(&args_value)
            .map(|e| e.to_string())
            .collect();
        Err(errors.join("; "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn schema_from_json(value: Value) -> ToolInputSchema {
        serde_json::from_value(value).unwrap()
    }

    #[test]
    fn test_validate_arguments_required_fields() {
        let schema = schema_from_json(json!({
            "type": "object",
            "properties": {
                "location": { "type": "string" },
                "unit": { "type": "string" }
            },
            "required": ["location"]
        }));

        let valid = json!({ "location": "Amsterdam", "unit": "celsius" });
        assert!(validate_arguments(Some(valid.as_object().unwrap()), &schema).is_ok());

        let missing = json!({ "unit": "celsius" });
        assert!(validate_arguments(Some(missing.as_object().unwrap()), &schema).is_err());
    }

    #[test]
    fn test_validate_string_enum() {
        let schema = schema_from_json(json!({
            "type": "object",
            "properties": {
                "unit": {
                    "type": "string",
                    "enum": ["celsius", "fahrenheit"]
                }
            }
        }));

        let valid = json!({ "unit": "celsius" });
        assert!(validate_arguments(Some(valid.as_object().unwrap()), &schema).is_ok());

        let invalid = json!({ "unit": "kelvin" });
        assert!(validate_arguments(Some(invalid.as_object().unwrap()), &schema).is_err());
    }

    #[test]
    fn test_validate_string_length() {
        let schema = schema_from_json(json!({
            "type": "object",
            "properties": {
                "name": {
                    "type": "string",
                    "minLength": 3,
                    "maxLength": 10
                }
            }
        }));

        let valid = json!({ "name": "Alice" });
        assert!(validate_arguments(Some(valid.as_object().unwrap()), &schema).is_ok());

        let too_short = json!({ "name": "Al" });
        assert!(validate_arguments(Some(too_short.as_object().unwrap()), &schema).is_err());

        let too_long = json!({ "name": "Alexander the Great" });
        assert!(validate_arguments(Some(too_long.as_object().unwrap()), &schema).is_err());
    }

    #[test]
    fn test_validate_number_range() {
        let schema = schema_from_json(json!({
            "type": "object",
            "properties": {
                "age": {
                    "type": "number",
                    "minimum": 0,
                    "maximum": 120
                }
            }
        }));

        let valid = json!({ "age": 25 });
        assert!(validate_arguments(Some(valid.as_object().unwrap()), &schema).is_ok());

        let below = json!({ "age": -1 });
        assert!(validate_arguments(Some(below.as_object().unwrap()), &schema).is_err());

        let above = json!({ "age": 150 });
        assert!(validate_arguments(Some(above.as_object().unwrap()), &schema).is_err());
    }

    #[test]
    fn test_validate_integer() {
        let schema = schema_from_json(json!({
            "type": "object",
            "properties": {
                "count": { "type": "integer" }
            }
        }));

        let valid = json!({ "count": 42 });
        assert!(validate_arguments(Some(valid.as_object().unwrap()), &schema).is_ok());

        let float_val = json!({ "count": 42.5 });
        assert!(validate_arguments(Some(float_val.as_object().unwrap()), &schema).is_err());
    }

    #[test]
    fn test_validate_boolean_and_array() {
        let schema = schema_from_json(json!({
            "type": "object",
            "properties": {
                "active": { "type": "boolean" },
                "tags": {
                    "type": "array",
                    "items": { "type": "string" },
                    "minItems": 1,
                    "maxItems": 5
                }
            }
        }));

        let valid_bool = json!({ "active": true });
        assert!(validate_arguments(Some(valid_bool.as_object().unwrap()), &schema).is_ok());

        let invalid_bool = json!({ "active": "true" });
        assert!(validate_arguments(Some(invalid_bool.as_object().unwrap()), &schema).is_err());

        let valid_arr = json!({ "tags": ["rust", "wasm"] });
        assert!(validate_arguments(Some(valid_arr.as_object().unwrap()), &schema).is_ok());

        let empty = json!({ "tags": [] });
        assert!(validate_arguments(Some(empty.as_object().unwrap()), &schema).is_err());

        let too_many = json!({ "tags": ["a", "b", "c", "d", "e", "f"] });
        assert!(validate_arguments(Some(too_many.as_object().unwrap()), &schema).is_err());

        let wrong_type = json!({ "tags": ["valid", 123] });
        assert!(validate_arguments(Some(wrong_type.as_object().unwrap()), &schema).is_err());
    }

    #[test]
    fn test_validate_nested_object() {
        let schema = schema_from_json(json!({
            "type": "object",
            "properties": {
                "user": {
                    "type": "object",
                    "properties": {
                        "name": { "type": "string" },
                        "age": { "type": "integer" }
                    },
                    "required": ["name"]
                }
            }
        }));

        let valid = json!({
            "user": {
                "name": "Alice",
                "age": 30
            }
        });
        assert!(validate_arguments(Some(valid.as_object().unwrap()), &schema).is_ok());

        let missing = json!({
            "user": {
                "age": 30
            }
        });
        assert!(validate_arguments(Some(missing.as_object().unwrap()), &schema).is_err());
    }

    #[test]
    fn test_validate_no_arguments_optional_fields() {
        let schema = schema_from_json(json!({
            "type": "object",
            "properties": {
                "location": { "type": "string" }
            }
        }));
        assert!(validate_arguments(None, &schema).is_ok());
    }

    #[test]
    fn test_validate_no_arguments_required_fields() {
        let schema = schema_from_json(json!({
            "type": "object",
            "properties": {
                "location": { "type": "string" }
            },
            "required": ["location"]
        }));
        assert!(validate_arguments(None, &schema).is_err());
    }

    #[test]
    fn test_validate_no_arguments_empty_schema() {
        // Tool that expects no input at all — schema has no properties and no required fields
        let schema = schema_from_json(json!({
            "type": "object"
        }));
        assert!(
            validate_arguments(None, &schema).is_ok(),
            "None arguments should pass when schema expects no input"
        );

        let empty_obj = json!({});
        assert!(
            validate_arguments(Some(empty_obj.as_object().unwrap()), &schema).is_ok(),
            "empty object should also pass when schema expects no input"
        );
    }

    #[test]
    fn test_validate_type_mismatch() {
        let schema = schema_from_json(json!({
            "type": "object",
            "properties": {
                "name": { "type": "string" },
                "count": { "type": "number" }
            }
        }));

        let wrong = json!({ "name": 123, "count": 5 });
        assert!(validate_arguments(Some(wrong.as_object().unwrap()), &schema).is_err());
    }
}
