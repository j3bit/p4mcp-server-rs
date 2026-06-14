use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

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
    pub approval_token: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
pub struct CommonQueryParams {
    pub action: String,
    #[serde(default)]
    pub changelist_id: Option<String>,
    #[serde(default)]
    pub workspace_name: Option<String>,
    #[serde(default)]
    pub file_path: Option<String>,
    #[serde(default)]
    pub user: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub job_id: Option<String>,
    #[serde(default)]
    pub stream: Option<String>,
    #[serde(default)]
    pub owner: Option<String>,
    #[serde(default = "default_max_results")]
    pub max_results: u16,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
pub struct CommonModifyParams {
    pub action: String,
    #[serde(default)]
    pub changelist_id: Option<String>,
    #[serde(default)]
    pub workspace_name: Option<String>,
    #[serde(default)]
    pub stream: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub files: Vec<String>,
    #[serde(default)]
    pub form: Option<String>,
    #[serde(default)]
    pub approval_token: Option<String>,
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
