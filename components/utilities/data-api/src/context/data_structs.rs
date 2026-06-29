use std::collections::VecDeque;

use crate::context::{InternalId, RealId};

#[derive(serde::Serialize, serde::Deserialize)]
pub struct ReserveIdMutationResult {
    pub data: ReserveIdResult,
}

#[derive(serde::Serialize, serde::Deserialize)]
pub struct ReserveIdResult {
    #[serde(rename = "reserveRecords")]
    pub reserved_ids: ReservedIds,
}

#[derive(serde::Serialize, serde::Deserialize)]
pub struct ReservedIds {
    pub ids: VecDeque<RealId>,
}

#[derive(serde::Serialize, serde::Deserialize, Debug, Default, Clone, Copy)]
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

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct MutationInput {
    pub input: MutationInputVariable,
    #[serde(default)]
    pub validation_sets: ValidationSets,
}

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone)]
pub struct MutationInputVariable {
    #[serde(default)]
    pub id: MutationIdInput,
    #[serde(flatten)]
    pub other_inputs: serde_json::Map<String, serde_json::Value>,
}

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone)]
pub struct DeleteInput {
    pub id: MutationIdInput,
}

#[derive(Default, serde::Serialize, serde::Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub enum ValidationSets {
    Empty,
    #[default]
    Default,
}

impl ValidationSets {
    pub fn as_str(&self) -> &'static str {
        match self {
            ValidationSets::Default => "default",
            ValidationSets::Empty => "empty",
        }
    }
}

impl std::fmt::Display for ValidationSets {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}
