use crate::context::{InternalId, ModelName, RealId};
use serde::{Deserialize, Serialize};
use std::{collections::VecDeque, ops::Deref};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ReserveIdMutationVariables {
    pub model: ModelName,
    pub amount: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ReserveIdMutationResult {
    pub data: ReserveIdResult,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ReserveIdResult {
    #[serde(rename = "reserveRecords")]
    pub reserved_ids: ReservedIds,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ReservedIds {
    pub ids: VecDeque<RealId>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "serde_json::Value", into = "InternalId")]
pub struct MutationIdInput(pub InternalId);

impl TryFrom<serde_json::Value> for MutationIdInput {
    type Error = &'static str;

    fn try_from(value: serde_json::Value) -> Result<Self, Self::Error> {
        Ok(Self(
            value
                .as_i64()
                .or(value.as_str().and_then(|s| s.parse().ok()))
                .ok_or("id input must be a string or integer")?
                .try_into()
                .map_err(|_| "id input is too big")?,
        ))
    }
}

impl From<MutationIdInput> for InternalId {
    fn from(value: MutationIdInput) -> Self {
        value.0
    }
}

impl Deref for MutationIdInput {
    type Target = InternalId;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MutationInput {
    pub input: MutationInputVariable,
    #[serde(default)]
    pub validation_sets: ValidationSets,
}

// The update mutation of the data API takes the id as a separate `$id` argument, while the
// captured update is sent as an upsert that needs the id inside the input.
#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
pub struct UpdateMutationInput {
    #[serde(default)]
    pub id: MutationIdInput,
    #[serde(flatten)]
    pub mutation_input: MutationInput,
}

impl From<UpdateMutationInput> for MutationInput {
    fn from(
        UpdateMutationInput {
            id,
            mut mutation_input,
        }: UpdateMutationInput,
    ) -> Self {
        if let MutationIdInput(0) = mutation_input.input.id {
            mutation_input.input.id = id;
        }

        mutation_input
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpsertMutationInput {
    #[serde(flatten)]
    pub mutation_input: MutationInput,
    pub unique_by: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct UpsertKey {
    pub model: ModelName,
    pub field: String,
    pub value: String,
}

impl UpsertKey {
    pub fn from_unique_value(model: &str, field: &str, value: &serde_json::Value) -> Option<Self> {
        let value = match value {
            serde_json::Value::String(value) => value.trim_end().to_lowercase(),
            serde_json::Value::Number(value) => value.to_string(),
            serde_json::Value::Bool(value) => value.to_string(),
            _ => return None,
        };

        Some(Self {
            model: model.to_string(),
            field: field.to_string(),
            value,
        })
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct MutationInputVariable {
    #[serde(default)]
    pub id: MutationIdInput,
    #[serde(flatten)]
    pub other_inputs: serde_json::Map<String, serde_json::Value>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct DeleteInput {
    pub id: MutationIdInput,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "Vec<String>", into = "Vec<String>")]
pub enum ValidationSets {
    Empty,
    #[default]
    Default,
}

// The GraphQL argument is `[String]`, so callers (e.g. the crud component) send a list,
// while the capture itself only distinguishes between "default" and "empty".
impl From<ValidationSets> for Vec<String> {
    fn from(value: ValidationSets) -> Self {
        vec![value.as_str().to_string()]
    }
}

impl TryFrom<Vec<String>> for ValidationSets {
    type Error = String;

    fn try_from(names: Vec<String>) -> Result<Self, Self::Error> {
        names
            .iter()
            .try_fold(ValidationSets::Empty, |acc, name| match name.as_str() {
                "default" => Ok(ValidationSets::Default),
                "empty" => Ok(acc),
                other => Err(format!("unsupported validation set: {other}")),
            })
    }
}

impl ValidationSets {
    pub fn as_str(&self) -> &'static str {
        match self {
            ValidationSets::Default => "default",
            ValidationSets::Empty => "empty",
        }
    }

    /// Mutations sent together get one set of validation sets, and the empty set wins.
    pub fn combine(&mut self, added: ValidationSets) {
        if let ValidationSets::Empty = added {
            *self = ValidationSets::Empty;
        }
    }
}

impl std::fmt::Display for ValidationSets {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(value: serde_json::Value) -> Option<String> {
        UpsertKey::from_unique_value("User", "number", &value).map(|key| key.value)
    }

    #[test]
    fn upsert_key_ignores_case_and_trailing_spaces_test() {
        assert_eq!(
            key(serde_json::json!("Ann@Example.com ")),
            key(serde_json::json!("ann@example.com"))
        );
        assert_eq!(key(serde_json::Value::Null), None);
    }
}
