use schemars::JsonSchema;
use serde::Serialize;
use serde_json::Value;

#[derive(Debug, Clone, Serialize, JsonSchema, PartialEq)]
#[serde(rename_all = "snake_case")]
pub struct ToolResponse {
    pub status: String,
    pub action: String,
    pub message: Value,
}

impl ToolResponse {
    pub fn success(action: impl Into<String>, message: Value) -> Self {
        Self {
            status: "success".to_string(),
            action: action.into(),
            message,
        }
    }

    pub fn dry_run(action: impl Into<String>, message: Value) -> Self {
        Self {
            status: "dry_run".to_string(),
            action: action.into(),
            message,
        }
    }
}
