use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::error::{P4McpError, Result};

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FileQueryAction {
    Content,
    History,
    Info,
    Metadata,
    Diff,
    Annotations,
    Search,
    Grep,
}

impl FileQueryAction {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Content => "content",
            Self::History => "history",
            Self::Info => "info",
            Self::Metadata => "metadata",
            Self::Diff => "diff",
            Self::Annotations => "annotations",
            Self::Search => "search",
            Self::Grep => "grep",
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
pub struct QueryFilesParams {
    pub action: FileQueryAction,
    pub file_path: String,
    #[serde(default)]
    pub file2: Option<String>,
    #[serde(default = "default_true")]
    pub diff2: bool,
    #[serde(default = "default_max_results")]
    pub max_results: u16,
    #[serde(default)]
    pub pattern: Option<String>,
    #[serde(default)]
    pub case_insensitive: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FileModifyAction {
    Add,
    Edit,
    Delete,
    Move,
    Revert,
    Reconcile,
    Resolve,
    Sync,
}

impl FileModifyAction {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Add => "add",
            Self::Edit => "edit",
            Self::Delete => "delete",
            Self::Move => "move",
            Self::Revert => "revert",
            Self::Reconcile => "reconcile",
            Self::Resolve => "resolve",
            Self::Sync => "sync",
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
pub struct ModifyFilesParams {
    pub action: FileModifyAction,
    #[serde(default)]
    pub file_paths: Option<Vec<String>>,
    #[serde(default = "default_changelist")]
    pub changelist: String,
    #[serde(default)]
    pub source_paths: Option<Vec<String>>,
    #[serde(default)]
    pub target_paths: Option<Vec<String>>,
    #[serde(default = "default_resolve_mode")]
    pub mode: String,
    #[serde(default)]
    pub force: bool,
    #[serde(default)]
    pub confirmation: Option<String>,
}

impl ModifyFilesParams {
    pub fn requires_confirmation(&self) -> bool {
        matches!(
            self.action,
            FileModifyAction::Delete | FileModifyAction::Revert
        )
    }

    pub fn confirmed(&self) -> Result<()> {
        if !self.requires_confirmation() || self.confirmation.as_deref() == Some("PROCEED") {
            return Ok(());
        }
        Err(P4McpError::ConfirmationRequired)
    }
}

fn default_true() -> bool {
    true
}

fn default_max_results() -> u16 {
    100
}

fn default_changelist() -> String {
    "default".to_string()
}

fn default_resolve_mode() -> String {
    "auto".to_string()
}
